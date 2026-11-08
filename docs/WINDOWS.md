# CutOut for Windows

CutOut for Windows is the same application as the macOS edition, built on the
same engine. This page covers the architecture, how to build and install it,
troubleshooting and the known limitations.

> **Verification levels used in these documents**
>
> * **Implemented** – the code exists and compiles for the target.
> * **Automated** – a scripted test passed (on the macOS development host
>   and/or on a GitHub Windows runner; each claim names where).
> * **Independently verified on Windows** – a person (or a clean Windows VM)
>   exercised the installed app. **Not yet done** for any item in this release
>   candidate; see "What still needs a human on real Windows" below.

## Architecture: one engine, two native shells

```
 macOS app (Swift, SwiftUI/AppKit)            Windows app (Rust, egui + wgpu)
 macos/                                        app/
        \                                         /
         \__ cutout-ffi (C ABI, JSON commands)   /  direct Rust calls
                         \                      /
                          cutout-core (Rust): layers, masks, selections, tools,
                          compositing, undo/redo, project format, AI pipeline
```

* **Shared, unchanged:** everything that defines an image — `core/` (document
  and layer model, selection algorithms, brushes, ΔE2000 colour tools, guided
  filter refinement, history, `.cutout` format, export compositing, the BiRefNet
  pre/post-processing and ONNX Runtime session). The Windows app drives it with
  the *same JSON command protocol* the macOS app uses (`Doc::run`), so a tool
  behaves identically on both systems.
* **Windows shell (`app/`):** window, menus, toolbar, tool sidebar, layers
  panel, sheets, canvas interaction, clipboard, file dialogs, image decoding,
  text rasterisation, model management. Written in Rust with
  [egui](https://github.com/emilk/egui) on wgpu (Direct3D 12 / Vulkan), so one
  language and one binary, no .NET or Visual C++ runtime installer to ship.
* **Why not a second native UI toolkit (WinUI 3)?** The shell had to be built
  and type-checked without a Windows machine. A Rust shell runs on the macOS
  development host (same code, scripted end-to-end tests, screenshots) and
  cross-checks for `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`;
  a C#/WinUI shell could not be compiled or run off Windows. The macOS app is
  untouched; it keeps its native SwiftUI interface.
* **Platform layer** (`app/src/platform/`): the only operating-system specific
  code. Everything else is portable.

### What differs on Windows (by design)

| Area | macOS | Windows |
|---|---|---|
| Fast background removal | Apple Vision (built in) | BiRefNet **Lite** when installed (no OS equivalent; not faster than Best Quality on CPU) |
| Best-quality acceleration | Core ML on GPU, ≈2.5 s | ONNX Runtime on all CPU cores, ≈15–25 s, ≈10 GB peak RAM; optional **DirectML** GPU provider (experimental, off by default) |
| Clipboard | PNG + TIFF with alpha | registered **PNG** + **CF_DIBV5** (alpha) + **CF_DIB** (flattened on white) |
| Icons | SF Symbols | [Lucide](https://lucide.dev) (ISC), same metaphors |
| Text rendering | CoreText | cosmic-text (shaping/layout identical on every platform) |
| Menus | global menu bar | in-window menu bar (Windows convention) |
| Shortcuts | ⌘ / ⌥ / ⇧ | Ctrl / Alt / Shift (same letters) |
| Window | unified toolbar | standard Windows title bar and controls |
| Colour management | ColorSync | embedded ICC profiles are converted to sRGB (moxcms) |

## Installing

Download `CutOut-Windows-x64-Setup.exe` from the release page (ARM64 PCs can use
the x64 installer through Windows' built-in emulation, or
`CutOut-Windows-ARM64-Setup.exe` for a native build). Double-click it and follow
the wizard. Per-user installation needs no administrator rights; choose
"Install for all users" if you want a machine-wide install. The installer
includes the AI model, so **Remove Background works offline immediately**.

Requirements: Windows 10 (1809) or Windows 11, 64-bit; a DirectX 12 or Vulkan
capable graphics driver (software rendering through Windows' built-in WARP
adapter works too); **16 GB RAM recommended** for Best Quality removal (see
limitations).

**Windows SmartScreen.** Release 0.1 is **not Authenticode-signed**
(see Installing), so
SmartScreen may show "Windows protected your PC". Verify the download first
(below), then choose *More info → Run anyway*. Never disable Windows Defender
or SmartScreen to run CutOut.

**Verify the download** (PowerShell):

```powershell
Get-FileHash .\CutOut-Windows-x64-Setup.exe -Algorithm SHA256
```

Compare the value with `SHA256SUMS.txt` on the release page.

**Uninstall** from *Settings → Apps → Installed apps → CutOut*. You are asked
whether to also delete settings, the saved last session and downloaded models;
the default is to keep them. Silent uninstall: `unins000.exe /VERYSILENT`
(`/PURGEUSERDATA` also removes user data).

Where things live: program `%LOCALAPPDATA%\Programs\CutOut`, settings
`%APPDATA%\CutOut\settings.json`, models and last session
`%LOCALAPPDATA%\CutOut`. `CUTOUT_DATA_DIR` redirects all of it (portable use,
tests).

## Building from source

Requirements: Rust (MSVC toolchain, `rustup`), Visual Studio Build Tools with
the C++ workload, Inno Setup 6 (for the installer).

```powershell
cargo build --release -p cutout-desktop          # target\release\CutOut.exe
cargo test  --release -p cutout-core -p cutout-desktop
.\installer\build-installer.ps1 -Arch x64 -Model fp32   # dist\CutOut-Windows-x64-Setup.exe
```

`-Model fp16` bundles the same weights at half precision (490 MB instead of
973 MB; identical mattes in our tests, ≈15 % slower); `-Model none` builds a
small installer without a model (the user then installs one from *Manage AI
Models*). The model is verified against its pinned SHA-256 before it is packaged
and again before first use.

On macOS you can run the same shell for development: `cargo run -p cutout-desktop`.

## Automated tests

* `cargo test -p cutout-core -p cutout-desktop` – engine tests, codec round
  trips, Windows clipboard DIB encode/decode.
* `cargo test -p cutout-desktop -- --ignored` – the real Windows clipboard
  round trip (needs a desktop session).
* **Scripted GUI tests** (`app/tests/scripts/*.json`): the app runs a script
  against its real window — clicks, drags, shortcuts, sheets, screenshots,
  assertions — via `CUTOUT_SCRIPT`. `smoke.json` covers open (Unicode path with
  spaces), selection, magic eraser, brush/eraser, crop, resize, text with
  non-Latin scripts, layers, project save/load, compare, copy and the clipboard
  round trip. `removal.json` runs the real model through the installed app.
* GitHub Actions (`.github/workflows/windows.yml`) runs all of the above on a
  clean Windows runner, plus an installer test (silent install → run →
  upgrade → uninstall → reinstall → purge).

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| "CutOut could not start — the graphics system could not be initialised" | No Direct3D 12 / Vulkan adapter. Update the graphics driver; in a VM enable 3D acceleration. |
| Remove Background is very slow or the PC becomes unresponsive | The best-quality model needs ≈10 GB of free memory. Close other programs, or install *BiRefNet Lite* in *Manage AI Models*. |
| "No model installed" | Used the no-model installer or deleted the model: *Image → Manage AI Models…* → Download or *Install from File…*. |
| Pasting a copied file does nothing | Only image files are accepted; use *Edit → Paste* or drop the file on the window. |
| Transparent image pastes with a white/black box in an old program | That program ignores alpha. Use *Copy as PNG* into a program that supports PNG, or place the image on a coloured layer first. |
| Blurry text/UI on a mixed-DPI setup | CutOut is Per-Monitor-V2 DPI aware; if a window was moved between monitors with very different scaling, resize it once to refresh. |
| SmartScreen warning | The build is unsigned; see above. |

## Known limitations

The important ones: best-quality removal is CPU-only by default (slow, memory-hungry
compared with Core ML); Fast mode on Windows is BiRefNet Lite, not an instant
mode; HEIC/HEIF files are not opened; images dragged from a web browser (as
opposed to files) are not accepted; the build is not code-signed; nothing in
this release has yet been verified by a person on real Windows hardware.
