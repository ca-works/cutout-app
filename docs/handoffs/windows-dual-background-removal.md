# Windows dual background removal — Phase 1 handoff

## Scope and source

Repository: `ca-works/cutout-app`. Base: `main` at
`51f2364ac44203ac7394e906d62e72cd00772947`. Branch:
`feature/windows-dual-background-removal`. No AGENTS.md was present.

All changes are in the Windows Rust shell (`app/`), tests, and this document.
The Swift macOS application, shared `core/` and `ffi/`, models, manifests,
dependencies/lockfile, installer, release infrastructure and website are unchanged.
The Rust shell can also run on Linux/macOS for development; it is not the shipping
macOS application.

## Completed implementation

* `app/src/background.rs`: independent CPU-only conventional mask algorithm.
  Estimates a single dominant opaque border colour with a 4096-bin RGB histogram.
  Transparent border pixels never contribute arbitrary hidden RGB. Four-connected
  flood fill from matching perimeter pixels removes only exterior-connected
  background. Maximum-channel RGB distance <=6 is removed; distances 7–23 receive
  proportional visibility; >=24 remains foreground. No resizing, blurring, small
  component deletion, RGB changes, semantic detection, network or model calls.
  With no opaque border samples it conservatively preserves the existing cutout.
  Dimensions and source alpha are preserved: the result is a visibility mask,
  multiplied by source alpha once by the existing compositor.
* ENG Drawing Viewer was inspected **read-only** at
  `ca-works/eng-drawing-viewer`, commit
  `2fd012084bf08b09990719895d35ba3681d48ab6`,
  `src/eng_viewer/core/image_tools.py:644` (`remove_background`) and
  `docs/IMAGE_EDITOR.md`. Its implementation uses Lab border k-means, edge walls,
  connected components, GrabCut and mask refinement via NumPy/OpenCV/Pillow.
  This is an independent Rust adaptation of its dominant-border/exterior-connected
  approach, **not a port of GrabCut**. No Python/OpenCV dependencies were added;
  costly segmentation and speck deletion were omitted for latency and small text
  preservation. ENG Drawing Viewer was not modified.
* `app/src/app.rs`: conventional work executes on a background thread using a
  separate result channel, not the AI job type. Only the active layer's mask is
  replaced using the existing `set_layer_mask` API. Unrelated layers, transforms,
  source pixels and text remain intact. Undo/redo is the existing single mask
  history operation; undo allows trying AI on the original. Cancellation/clearing
  drops the result receiver, so a late worker cannot resurrect a cleared workspace.
  The CPU worker finishes its current calculation after cancellation.
* `app/src/ui/mod.rs`: adjacent matching-height Remove Background and AI Remove
  Background buttons in a dedicated compact toolbar row; enough space at the
  existing 700-point minimum window width. Ordinary button has **no tooltip**.
  AI action routes to the existing `RemovalMode::Best`; the existing Lite AI
  option remains available in the menu. Ctrl+B retains its existing AI mapping.
* `app/src/ui/widgets.rs`: AI-only rounded tooltip, exact text
  `Slower, but better for complex images`; foreground area avoids panel clipping,
  horizontal clamping, below-button fallback at the top of the viewport,
  150 ms opacity animation and 3-point upward movement. Uses current theme;
  respects reduced motion. Native visual verification remains required.
* Clear Workspace already existed under File. `app/src/ui/sheets.rs` adds a
  cancelable confirmation using the existing modal toolkit. **Clear is not
  undoable**: the existing shared engine clears all history. Confirmation is used
  rather than introducing a second history system or changing macOS semantics.
  Empty workspaces disable Clear without prompting. No new keyboard shortcut.
  Explicit Clear writes no recovery file and never touches clipboard or settings.
  Copy & Clear keeps its existing recovery-file behavior.
* Clearing resets the engine, source layers/masks/selection/history, canvas tiles,
  drag/polygon/crop/compare/reveal state, selection outline, thumbnails, selected
  layers, layer rename/drag/opacity state, refinement/adjustment previews, sheet,
  processing result receiver and project path. Preferences/export settings/AI
  settings and already-requested file exports are retained. Image commands disable.
* `app/src/main.rs` registers the algorithm module; `app/src/engine.rs` adds a
  mask-compositing integration test. App construction accepts a context internally
  so editor tests can run without an OS window or loading user preferences.
* `app/src/selftest.rs` exposes conventional and confirmed-clear script actions.
  Existing smoke/screens scripts explicitly accept the new confirmation;
  `app/tests/scripts/dual-background.json` exercises removal, undo, redo,
  clear cancel/confirm. Existing AI script actions retain their original mappings.
* `app/benches/conventional.rs` is a standalone production-algorithm benchmark.

## Algorithm limitations to assess

This is for plain backgrounds, not semantic subject extraction. Near-identical
foreground connected directly to the border may be removed. Enclosed regions of
background colour are deliberately retained to protect white logos/details; use
Magic Eraser for holes if desired. Multiple unrelated background colours, gradients,
shadows and photographic textures may remain. The narrow mask-only transition does
not unmix antialiased RGB or decontaminate strongly tinted edges. Use existing
Refine Edges / AI for complex boundaries. No general photo-quality claim is made.

## Cloud verification

Linux x86_64 managed VM, 3 vCPUs, AMD EPYC 9V74, ~9.7 GiB RAM; Rust 1.99.1.
Rust was installed under `/workspace/.cargo` and `/workspace/.rustup` because the
image initially lacked Rust. No dependency manifests were changed.

Standalone algorithm tests: 7 passed (white/black/coloured backgrounds,
small text/detail and enclosed colour preservation, similar foreground,
partial/existing transparency, dimensions, malformed/empty/overflow input,
large image, soft transition).

Cloud optimized synthetic benchmark, 11 runs each, concurrent compilation:

| Size | Median ms | Min ms | Max ms |
|---|---:|---:|---:|
| 512 × 512 | 2.68 | 2.43 | 32.80 |
| 1024 × 1024 | 11.11 | 6.87 | 41.74 |
| 1920 × 1080 | 20.85 | 17.65 | 56.04 |
| 3840 × 2160 | 112.68 | 98.46 | 159.17 |

Input: opaque white border with central coloured rectangle; timings cover mask
computation/allocation only, not source copying, UI, compositing or inference.
These are cloud measurements, not Windows results or promises.

Reproduce from repository root:

```sh
rustc --edition=2021 --test app/src/background.rs -o /tmp/background-tests
/tmp/background-tests
rustc -O --edition=2021 app/benches/conventional.rs -o /tmp/conventional-bench
/tmp/conventional-bench
cargo test --locked -p cutout-core -p cutout-desktop
```

Additional results and final commit identifiers are recorded below after validation.

## Remaining physical Windows checklist

1. Check out the branch and inspect `git log -3 --oneline` and this handoff.
   Preserve local/unrelated changes. Do not merge, release or update the website.
2. Run `cargo test --locked -p cutout-core -p cutout-desktop`.
3. In an interactive desktop session, run the clipboard tests serially:
   `cargo test -p cutout-desktop -- --ignored --test-threads=1`.
   These tests overwrite clipboard contents with test images.
4. Build/run with existing MSVC toolchain:
   `cargo build --release --locked -p cutout-desktop` and
   `cargo run --release -p cutout-desktop`.
5. Test regular removal without a model installed, offline, and with the existing
   prewarm left enabled. Observe that this action itself starts no AI session.
   Existing automatic preload on first import is intentionally unchanged.
6. Exercise imports via Open, paste, Explorer files and drag/drop; image/text/shape
   layers, transforms, layer order/visibility/opacity, selections, crop, resize,
   refine, undo/redo, Copy, Copy & Clear, Copy & Close, PNG/JPEG export and default
   export settings. Ensure source files remain byte-identical.
7. Check both themes, 100/150/200% DPI, window resizing to 700 × 460, toolbar
   alignment, tooltip above/below positioning, fade-in/out, reduced motion,
   and lack of any tooltip on the regular action. Inspect accessibility labels.
8. Test Clear cancel, Escape/outside dismissal and confirm; include text editing,
   active crop/lasso/transform, selection, preview panels, unsaved project,
   clipboard sentinel and background operation. Verify empty canvas, no stale
   overlays, unchanged settings/files/clipboard, disabled image commands. Clear
   intentionally cannot be undone and warns before confirmation.
9. Test AI using the same original inputs/models/settings as base commit
   `51f2364ac44203ac7394e906d62e72cd00772947`; compare exported alpha exactly for
   identical runtime/provider conditions. Run both DirectML GPU and CPU fallback,
   cancellation and low-memory warning. AI inference/pre/postprocessing and model
   code are unchanged; no real inference was requested for Phase 1.
10. Compile the standalone benchmark with `rustc -O --edition=2021
    app/benches/conventional.rs -o conventional-bench.exe`; run it and separately
    measure end-to-end UI time on representative logos/screenshots/drawings/photos.
    Document CPU/RAM/GPU, resolution, cold/warm medians and peak memory.
11. For scripted GUI tests, inspect `app/src/selftest.rs` for `CUTOUT_SCRIPT`,
    `${OUT}` and fixture requirements. `dual-background.json` uses the checked-in
    `app/tests/data/test-subject.png`. Existing `smoke.json` also needs its external
    Unicode-path fixture. Existing AI removal scripts need the installed model. Do not claim scripts ran merely because unit tests passed.

## Installer preparation (Windows only)

Follow actual `installer/build-installer.ps1`, `installer/CutOut.iss` and
`docs/WINDOWS.md`. Requirements: Rust MSVC, Visual Studio C++ Build Tools,
Inno Setup 6 / ISCC. For the existing compact v2 offline package, use the already
verified model directory (do not recreate/convert/replace the model for this task):

```powershell
.\installer\build-installer.ps1 -Arch x64 -Model v2 -ModelDir <existing-model-directory>
Get-FileHash .\dist\CutOut-Windows-x64-Setup.exe -Algorithm SHA256
```

The script builds the target, stages under `dist\win-x64`, validates pinned hashes,
creates `dist\CutOut-Windows-x64-Setup.exe` and writes `dist\SHA256SUMS.txt`.
For a packaging-only smoke test, `-Model none` exists, but is not an offline AI
production installer. `-Model fp32`/`fp16` can download large models if missing;
avoid unnecessary transfers. Use `-Arch arm64` only where needed; do not claim
ARM64 hardware verification from x64 testing.

Test standard-user install, upgrade preserving settings, offline launch/removal,
clipboard after exit, uninstall/reinstall and packaged smoke tests. Observe
Defender/SmartScreen without disabling protections. Do not publish an installer.
Some existing docs refer to `.github/workflows/windows.yml` and PARITY/RELEASING
files absent in this checkout; no workflows were added or dispatched.

Example interactive Windows script run (after building):

```powershell
$env:CUTOUT_TEST_DATA = (Resolve-Path .\app\tests\data).Path
$env:CUTOUT_TEST_OUT = Join-Path $env:TEMP "cutout-dual-background"
New-Item -ItemType Directory -Force $env:CUTOUT_TEST_OUT | Out-Null
$env:CUTOUT_SCRIPT = (Resolve-Path .\app\tests\scripts\dual-background.json).Path
$env:CUTOUT_SCRIPT_RESULT = Join-Path $env:CUTOUT_TEST_OUT "result.json"
Start-Process -Wait -FilePath .\target\release\CutOut.exe
Get-Content $env:CUTOUT_SCRIPT_RESULT
```

Use a disposable test profile via the existing `CUTOUT_DATA_DIR` override to keep
normal settings/session data separate. Inspect the JSON report's `ok` field.

### Cloud runtime download restriction

The default `cargo test -p cutout-desktop --no-run --locked` could not finish:
`ort-sys 2.0.0-rc.15` received HTTP 403 downloading its pinned ONNX Runtime 1.31
archive from `cdn.pyke.io`. This is an environment/dependency-host restriction,
not an AI implementation failure. The VM has an older 1.21 shared library; it
was deliberately not substituted for the pinned runtime.

Non-inference tests are instead built with the dependency's supported
`load-dynamic` feature, supplied **only on the command line**:

```sh
cargo test --locked -p cutout-core -p cutout-desktop --features ort/load-dynamic
```

This avoids link-time binary download; no inference session is initialized by
these tests, and it does not verify model/runtime execution. Cargo.toml,
Cargo.lock and the shipping feature defaults remain unchanged. On Windows,
run the normal command without this workaround using the required runtime.

### Automated coverage and verification results

The dynamic-loading test configuration passed **34 core tests and 21 desktop
tests** (55 total). The existing `codec::perf::codec_timings` benchmark is ignored
by default. Windows clipboard tests are compiled only on Windows and were not
executed on Linux. There was an initial headless-harness failure because egui
texture updates were dropped without being consumed; the harness now explicitly
clears renderer deltas and the test passes.

Coverage added: seven standalone algorithm tests; source-alpha/compositor
integration and exact source restoration through undo/redo; actual conventional
worker/action dispatch without a model; preservation of unrelated layers;
clear confirmation/cancel/empty behavior; state and preference preservation;
discarding stale worker results after clear or editing; actual pointer clicks
on both buttons, disabled buttons and AI-only tooltip show/hide in light/dark
at normal and top-of-viewport placements. A Windows-only ignored test writes
an image sentinel to the native clipboard, clears the workspace, then verifies
that the clipboard is unchanged.

Existing AI reference tests `preprocess_matches_the_image_crate`,
`upscale_matches_the_image_crate`, and `solid_regions_stay_solid` passed.
`app/src/models.rs`, all of `core/`, model/checksum files and the prewarm function
are unchanged. A source comparison confirmed the AI removal body is identical;
only its shared busy guard now also rejects an overlapping conventional job.
These checks do not establish physical GPU/CPU runtime equivalence.

`git diff --check`, standalone new-module Rust formatting checks and parsing all
11 GUI script JSON files passed. No Python files were changed (Python compilation
is not applicable). Installer commands were checked against existing source;
installer execution, native GUI scripts, native clipboard and packaged tests
were not run on Linux. Headless egui tests did run without an OS display.

Clippy completed successfully with the same command-line runtime workaround:

```sh
cargo clippy --locked -p cutout-core -p cutout-desktop --features ort/load-dynamic --tests --no-deps
```

It reported existing style, dead-code and egui deprecation warnings (25 core-test
warnings including duplicates, 35 desktop-test warnings). No warning points to the
new conventional algorithm. Unrelated code was not changed to silence warnings.
The final full test rerun, including both tooltip placements and fade-out checks,
passed again: 34 core + 21 desktop, one existing codec timing test ignored.

## Git state and continuation

* Branch: `feature/windows-dual-background-removal` (pushed to `origin`).
* Implementation/tests commit: `0b080e675f5c761cd72cb24aee3c8d42185f3578`.
* Documentation commit: the following commit on the same branch; use
  `git log -2 --oneline` to resolve its final hash (a commit cannot contain its own hash).
* Draft PR: https://github.com/ca-works/cutout-app/pull/1 — leave unmerged.
* GitHub initially rejected the unpublished implementation commit because the
  local Git identity used a protected private email. That new commit alone was
  recreated with the account's GitHub noreply address, then pushed successfully.
  No unrelated history was rewritten and no force-push was used.
* No release, installer publication, website update or CI dispatch was performed.

Ready-to-copy continuation prompt:

> Continue Phase 2 of CutOut's Windows-only update on branch
> `feature/windows-dual-background-removal` in `ca-works/cutout-app` (draft PR #1).
> Read `docs/handoffs/windows-dual-background-removal.md` first. Implementation
> and Linux non-inference tests are complete. Run the native Windows checklist,
> verify tooltip visuals, clipboard preservation, GPU/CPU AI equivalence and
> performance, then build/test the existing installer without publishing it.
> Preserve AI inference/model/prewarm code and macOS behavior. Do not merge,
> publish a release or update the website.
