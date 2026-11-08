# Building, running and testing CutOut

## Requirements

* macOS 14 Sonoma or later (developed on macOS 27, Apple Silicon M1).
* Xcode 16+ command-line tools (Swift 5.10+). Developed with Xcode 27.1 / Swift 6.4.
* Rust stable (developed with 1.99.1): `brew install rustup && rustup default stable`.
* ~2 GB free disk for build artefacts. First build downloads Rust crates and
  the ONNX Runtime static library (≈85 MB, pinned by the `ort` crate, cached in
  `~/Library/Caches/ort.pyke.io`). After that, builds work offline.

## Build the app

```bash
scripts/build.sh
```

Produces `dist/CutOut.app` (Apple Silicon, ad-hoc signed, hardened runtime).

```bash
scripts/build.sh --dmg
```

Also produces `dist/CutOut.dmg` (drag-to-Applications layout).

```bash
BUNDLE_COREML=1 scripts/build.sh --dmg
```

**Full build (recommended for users):** precompiles the verified Core ML
BiRefNet into the app, so Best Quality works offline from the first launch
with no setup (app 477 MB, DMG 403 MB). Needs the converted package once on
the build machine (`tools/convert/install.sh`, developers only); its weight
hash is checked against `tools/convert/OUTPUT.txt`.

```bash
MODEL_ARCHIVE=1 scripts/build.sh
```

Writes `dist/birefnet-general-coreml.zip` (401 MB) that users of the
standard 12 MB build can install with **Model setup ▸ Install from File…**.

```bash
BUNDLE_MODEL=1 scripts/build.sh --dmg
```

Embeds the BiRefNet model (≈973 MB) in the app so that best-quality removal
works offline from the first launch, with no download prompt. Requires the
model to be installed locally first (see below).

## Install and launch (unsigned development build)

The build is **ad-hoc signed, not notarized** (no Apple Developer certificate
was used). Gatekeeper will block the first launch of a copy that was
downloaded:

1. Copy `CutOut.app` to `/Applications` (or open the DMG and drag it).
2. Right-click CutOut.app → **Open** → **Open**. Or, in System Settings →
   Privacy & Security, click **Open Anyway**.
3. Alternatively: `xattr -dr com.apple.quarantine /Applications/CutOut.app`.

Builds made on the same Mac are not quarantined and open normally.

## The best-quality model

**Full build:** nothing to do — the Core ML model is inside the app.

**Standard build:** on first use of **Remove Background**, the model setup
sheet shows which backend is active and offers **Install from File…** for the
Core ML archive (no Python, verified before install). An in-app download of
the Core ML archive is implemented but disabled until a hosting location is
approved. CutOut never downloads anything unless you ask. It also offers the
ONNX CPU models (slower fallback):

* **BiRefNet (Best Quality)** — 973 MB, MIT, SHA-256
  `58f621f0…ebd3c`, from `huggingface.co/onnx-community/BiRefNet-ONNX` at
  revision `534d3c82`.
* **BiRefNet Lite** — 224 MB, MIT.
* or **Use Fast Mode Now** (Apple Vision, built in, no download).

The file is verified (size + SHA-256) and stored in
`~/Library/Application Support/CutOut/Models/`. To install manually:

```bash
mkdir -p ~/Library/Application\ Support/CutOut/Models
curl -L -o ~/Library/Application\ Support/CutOut/Models/birefnet-general-fp32.onnx https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model.onnx
```

```bash
shasum -a 256 ~/Library/Application\ Support/CutOut/Models/birefnet-general-fp32.onnx
```

The hash must be `58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c`.

## Tests

Rust unit tests (geometry, CIEDE2000 reference pairs, magic-wand colour
separation, contiguous/global modes, anti-aliased edges, quick selection,
brushes, compositing, decontamination, project round-trip, adjustments):

```bash
cargo test -p cutout-core
```

End-to-end app self-test (drives the real UI model, engine, AI, pasteboard;
also measures startup and verifies Copy & Close clipboard persistence after
the app exits):

```bash
bench/fetch_images.sh && ./target/release/cutout-cli synth bench/synth
```

```bash
mkdir -p bench/all && cp -R bench/synth/* bench/images/* bench/all/
```

```bash
scripts/build.sh && scripts/selftest.sh
```

Results land in `build/selftest/` (`selftest.json` + window snapshots).
Model-management scenarios (fresh user with the full app, install from file,
forced Core ML failure → ONNX fallback, no model installed), using temporary
model folders so installed models are never touched:

```bash
BUNDLE_COREML=1 MODEL_ARCHIVE=1 scripts/build.sh && cp -R dist/CutOut.app build/CutOut-full.app && scripts/build.sh && scripts/selftest_models.sh
```

Model benchmark (quality metrics on synthetic ground truth + timings):

```bash
bench/run_models.sh cpu
```

Optional, Apple Silicon: build the Core ML version of BiRefNet locally
(≈5 min, ≈9 GB RAM; pinned + hash-verified, nothing is uploaded) — Best
Quality then runs in ≈2 s instead of ≈14 s. The ONNX model stays installed as
fallback. See `tools/convert/README.md`.

```bash
tools/convert/install.sh
```

Core ML speed and parity against the ONNX outputs (run `bench/run_models.sh cpu` first):

```bash
bench/run_coreml.sh build/convert/BiRefNet.mlpackage gpu
```

Engine performance on a 12 MP image:

```bash
./target/release/cutout-cli perf
```

## Project layout

```
core/            cutout-core — portable Rust engine (+ unit tests)
ffi/             cutout-ffi — C ABI (include/cutout.h)
macos/           SwiftPM package for the app (Sources/CutOut, Info.plist)
tools/cutout-cli benchmark / batch CLI (bench, score, synth, perf)
tools/convert    pinned BiRefNet → Core ML conversion recipe
bench/           benchmark image manifest and scripts (images are downloaded, not committed)
scripts/         build, icon, self-test, preview helpers
docs/            architecture, models, performance, testing, Windows plan, licensing
```
