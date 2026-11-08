# Third-party components

CutOut's own source is licensed under the MIT License (see `LICENSE`). This file lists third-party code
and model weights and their terms. Model weights are licensed separately from
the application source.

## AI model weights

The standard build contains no weights. The full build (`BUNDLE_COREML=1`)
contains BiRefNet converted to Core ML; the ONNX models are optional user
downloads.

| Model | Publisher | Licence | Notes |
|---|---|---|---|
| BiRefNet (general), ONNX export | Peng Zheng et al. ([ZhengPeng7/BiRefNet](https://github.com/ZhengPeng7/BiRefNet)); ONNX conversion by [onnx-community](https://huggingface.co/onnx-community/BiRefNet-ONNX) | MIT | Pinned revision `534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67`, SHA-256 `58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c`. MIT permits redistribution with the copyright notice. |
| BiRefNet (general), Core ML conversion | Converted by CutOut's `tools/convert` from `ZhengPeng7/BiRefNet@66f7f80c` (`model.safetensors` SHA-256 `77277264…4519`) | MIT | fp16 ML Program; deformable convolutions decomposed into equivalent ops. Output `weight.bin` SHA-256 `66044c149f7a152d9b1c522a681a09ebb9d73db5fd27fefaf05c79ec9034287f`. |
| BiRefNet Lite, ONNX export | same | MIT | Revision `de15b22ba131738a16dff04aab8bdf8dc32e3ac1`, SHA-256 `5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333`. |

Models evaluated but **not** used for licensing reasons: BRIA RMBG-1.4/2.0
(BiRefNet-based, but CC BY-NC 4.0 — non-commercial only, would restrict
CutOut's future open-source distribution).

## Apple system frameworks

Vision (Fast mode), Core ML, ImageIO, CoreText, AppKit, SwiftUI — part of
macOS, not redistributed.

## Windows edition

The Windows app (`app/`) links the following in addition to the engine's
dependencies. Everything is permissive except where noted. Licence texts of the
crates are in their source packages (`cargo vendor`), and the ones that require
an attached notice are installed under `Licenses\` next to `CutOut.exe`.

| Component | Licence | Notes |
|---|---|---|
| egui / eframe / egui-wgpu / egui-winit, winit, wgpu, naga | MIT OR Apache-2.0 | user interface and GPU rendering |
| Fonts embedded by egui's `default_fonts` feature (Ubuntu-Light, Hack, NotoEmoji, emoji-icon-font) | Ubuntu Font Licence 1.0, MIT, SIL OFL 1.1 | used as UI fall-backs |
| **Lucide icons** (`app/assets/lucide.ttf`, lucide-static 1.69.0) | ISC | copy in `LICENSES/Lucide-ISC.txt` |
| Segoe UI and other Windows system fonts | Microsoft | loaded from `C:\Windows\Fonts` at run time; **not redistributed** |
| cosmic-text, fontdb, swash, ttf-parser, rustybuzz | MIT OR Apache-2.0 | text layers |
| rfd | MIT | native file dialogs (`IFileDialog` on Windows) |
| `windows` / `windows-sys` (Microsoft) | MIT OR Apache-2.0 | clipboard, DLL search hardening |
| moxcms | BSD-3-Clause OR Apache-2.0 | ICC colour management |
| ureq, native-tls | MIT OR Apache-2.0 | model download only (SChannel on Windows) |
| ONNX Runtime (static) | MIT | Microsoft; linked through the `ort` crate |
| option-ext (via `dirs`) | MPL-2.0 | file-level copyleft, used unmodified |
| webpki-root-certs | CDLA-Permissive-2.0 | certificate data |
| Mona Sans wordmark (rendered to `app/assets/wordmark.png`) | SIL OFL 1.1 | the font file is not distributed with the Windows app; rendering text with an OFL font is unrestricted |
| Inno Setup (build tool) | Inno Setup licence | not redistributed; the installer it produces carries no licence obligation |

Summary of the 235 third-party crates in the Windows build by licence: 137 MIT OR
Apache-2.0, 25 Apache-2.0 OR MIT, 22 MIT, 11 Apache-2.0, 8 MIT/Apache-2.0,
2 ISC, 3 Zlib, plus a handful of Zlib/BSD/Unlicense/0BSD dual licences, one
MPL-2.0 (above) and one CDLA-Permissive-2.0 (above). No GPL/LGPL crate is used
under a copyleft option.

## Rust crates linked into the app

93 crates, all permissive. Summary by licence expression:

| Licence | Crates |
|---|---|
| MIT OR Apache-2.0 (and equivalent orderings) | 66 |
| MIT | 10 |
| Unlicense OR MIT | 2 |
| MIT OR Apache-2.0 OR Zlib / Zlib variants | 6 |
| BSD-2-Clause / BSD-3-Clause OR Apache-2.0 (OR MIT) | 4 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| Apache-2.0 | 1 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 |
| Apache-2.0 WITH LLVM-exception OR … | 1 |

Notable: `ort` / `ort-sys` (ONNX Runtime bindings, MIT OR Apache-2.0),
**ONNX Runtime** itself (MIT, © Microsoft, statically linked), `image`,
`png`, `zip` (MIT), `rayon`, `serde`, `half`.

Build-only (not linked into the app): `ureq`, `native-tls`,
`webpki-root-certs` (CDLA-Permissive-2.0) — used by `ort-sys`'s build script
to download the ONNX Runtime library.

Regenerate the full list with:

```bash
cargo tree -p cutout-ffi -e normal --prefix none --format "{p} {l}" | sort -u
```

## Benchmark images (downloaded by `bench/fetch_images.sh`, not committed)

Wikimedia Commons files; licences per image are in `bench/images.tsv`
(CC BY-SA 4.0, CC BY 2.0, CC0, public domain). Attribution is required for the
CC BY / BY-SA images if they are ever redistributed.

## BiRefNet licence (applies to all BiRefNet weights above)

```
MIT License

Copyright (c) 2024 ZhengPeng

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Mona Sans (the CutOut wordmark)

`assets/fonts/MonaSans-Latin.woff2` is the Latin subset of Mona Sans (variable: weight and width),
as packaged by Fontsource (`@fontsource-variable/mona-sans` 5.3.0), used unmodified.
Copyright 2022 The Mona Sans Project Authors (https://github.com/github/mona-sans), with Reserved
Font Name "Mona". Licensed under the SIL Open Font License 1.1; the full licence is in
`LICENSES/MonaSans-OFL.txt` and is shipped inside the app bundle next to the font.
