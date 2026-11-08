# CutOut architecture

CutOut is a native macOS app with a **portable Rust engine** and a **thin
native front end**. Everything that defines the image — pixels, masks,
layers, selections, compositing, tools, history, the project format and AI
inference — lives in Rust. The macOS front end owns only what is genuinely
platform-specific: windows, menus, input events, GPU presentation, the
pasteboard, file dialogs, image decoding/colour management, text
rasterisation and Apple Vision.

```
┌──────────────────────────── macOS front end (Swift) ─────────────────────────────┐
│ SwiftUI: window, toolbar, tool sidebar, layers inspector, sheets, settings       │
│ AppKit:  CanvasNSView — events, zoom/pan, overlays, 512 px CALayer tile grid     │
│ Platform: NSPasteboard · ImageIO (decode/encode, EXIF, P3→sRGB) · CoreText ·     │
│           Vision (Fast mode) · URLSession (opt-in model download, SHA-256)       │
└───────────────┬──────────────────────────────────────────────────────────────────┘
                │ C ABI (ffi/include/cutout.h): JSON command protocol + bulk pixel calls
┌───────────────▼──────────────── cutout-core (Rust, no platform deps) ────────────┐
│ engine     command dispatcher, display buffer + dirty-rect tracking              │
│ document   layers (pixels + non-destructive mask + original), transforms, text  │
│ composite  premultiplied source-over, integer fast path, bilinear for transforms │
│ selection  rect/ellipse/polygon, ΔE2000 magic wand, quick selection, outlines    │
│ brush      stroke-buffer brushes (erase / restore / paint), shapes               │
│ filters    box/gaussian, morphology, colour guided filter, decontamination       │
│ adjust     exposure/contrast/HSL/temperature/…, auto levels, unsharp             │
│ history    copy-on-write snapshots, bounded by count and unique bytes           │
│ project    .cutout ZIP container, defensive loader                               │
│ ai         ONNX Runtime session (BiRefNet), preprocessing, matte refinement      │
└──────────────────────────────────────────────────────────────────────────────────┘
```

## Why SwiftUI + AppKit + Rust (and not an all-Rust UI)

| Criterion | SwiftUI/AppKit + Rust core | Slint (Rust) | egui + wgpu (Rust) |
|---|---|---|---|
| Native macOS look (toolbar, sidebar, inspector, menus, materials, sheets) | ✅ real system controls | ⚠️ themed imitation | ❌ immediate-mode look |
| Accessibility (VoiceOver, Reduce Motion, keyboard nav) | ✅ system-provided | ⚠️ partial | ❌ limited |
| Startup time | ✅ ≈0.7 s warm (measured) | ✅ | ✅ |
| Canvas performance | ✅ GPU via Core Animation tiles | ✅ | ✅ |
| Pasteboard / file dialogs / Vision / CoreText | ✅ direct | ⚠️ via crates/FFI | ⚠️ via crates/FFI |
| Windows reuse | ✅ engine reused; new native shell | ✅ same UI | ✅ same UI |

The decisive factors were the "looks and feels like an Apple app" requirement
and first-class platform services (pasteboard formats, ImageIO colour
management, Vision). Slint and egui would share more UI code with Windows but
cannot match native macOS controls or accessibility. Keeping *all* image
semantics in Rust means the Windows port only re-implements the shell (see
[WINDOWS.md](WINDOWS.md)).

Electron, web views, bundled Python and React/Tailwind were deliberately
avoided. The web references in the handbook (Motion, GSAP, React Bits …) were
used as *inspiration* only; motion is implemented with SwiftUI/Core Animation.

## Engine API

* `cutout_command(engine, json)` — ~70 commands (`magic_wand`, `brush_begin`,
  `merge_down`, `refine_preview`, `save_project`, …). Small, versionable,
  identical on every platform, easy to log and test.
* Binary calls for bulk data: `cutout_import_rgba`, `cutout_render` (returns
  the premultiplied display buffer **and the dirty rectangle**),
  `cutout_export_rgba`, `cutout_layer_pixels`, `cutout_set_layer_mask`,
  `cutout_layer_thumbnail`, `cutout_selection_outline`, `cutout_text_layer`.
* AI: `cutout_model_load/predict/cancel/free` are thread-safe and run on a
  background queue; the engine itself is only touched from the main thread.
* Every entry point is wrapped in `catch_unwind`; a Rust panic becomes an error
  value, never an unwinding crash into Swift.

## Data model

* Pixels are 8-bit sRGB RGBA with **straight alpha**; buffers sit behind
  `Arc` so history snapshots are O(1) and copy-on-write.
* **Non-destructive masks**: Eraser, Magic Eraser, colour deletion,
  "Delete Selected Pixels" and background removal only write the layer mask.
  The Restore brush and "Restore All" bring pixels back at any time.
  Decontamination keeps the pre-edit pixels in `original`, so restoring also
  restores true colours.
* Layer transform = centre position, scale X/Y, rotation. Content outside the
  canvas is kept (crop is non-destructive for layers).
* Text layers store their properties (text, font, size, colour, style) and a
  raster cache produced by the platform text engine; they remain editable.

## Rendering

The engine composites only the dirty rectangle into a canvas-sized
premultiplied buffer (rayon-parallel per row). The canvas view copies only the
512×512 tiles that intersect the dirty rect into `CALayer` contents; zoom and
pan are Core Animation transforms, so they never recomposite. Nearest-neighbour
magnification above 200 % shows true pixels. Marching ants are a pixel-edge
outline (merged runs) rendered by `CAShapeLayer`.

Measured on an M1 with a 12 MP image: full composite 8 ms, brush event incl.
dirty composite ≈0.1 ms.

## Colour-based tools

Tolerance is a **CIEDE2000 ΔE** in CIELAB, combined with alpha difference.
That is what keeps a white machined part when deleting a beige backdrop (ΔE ≈
12 between them, while beige in shadow is ΔE ≈ 2). A per-seed distance map is
cached, so dragging the tolerance slider only re-thresholds. Pixels within
`tolerance` are fully selected; pixels within an additional soft band get
partial coverage, which is how anti-aliased edge pixels are handled. Contiguous
mode floods through fully selected pixels only; global mode thresholds the
whole layer.

## Background removal pipeline

1. Layer source pixels (never the masked result) → flatten existing alpha on
   grey → resize to 1024² → ImageNet normalisation.
2. BiRefNet (ONNX Runtime, all CPU cores) → sigmoid matte.
3. Bicubic upscale to the source resolution (no downscaling of the result).
4. Colour guided-filter refinement in the uncertain edge band only (solid
   interior/background stay exactly 255/0).
5. Matte becomes the layer **mask** (undoable, restorable).
6. Optional Refine Edges: guided radius, smooth, feather, contrast, shift edge,
   colour decontamination (Forte & Pitié blur-fusion foreground estimation).

Fast mode uses Apple Vision's `VNGenerateForegroundInstanceMaskRequest`
followed by the same full-resolution edge refinement.

## Security model

* No network access at runtime except the model download the user starts.
  The Rust library links no HTTP/TLS code (ureq is used only by the
  ONNX Runtime *build script* to fetch the runtime library at build time).
* Model download: pinned Hugging Face revision, exact byte size and SHA-256
  checked before the file is moved into place. ONNX graphs are data — no
  pickle or custom code execution.
* Image decoding through ImageIO with a 1 GB file cap; huge images are
  downsampled during decode (≤ 16 384 px per side, ≤ 120 MP) instead of
  allocating unbounded buffers.
* `.cutout` loader: bounded entry count, per-entry and total decompressed size,
  dimension checks, duplicate-ID rejection, transform validation, entry names
  never used as file paths; atomic saves (temp file + rename).
* The pasteboard is only read on an explicit paste/drop; never monitored.
* Hardened runtime, no entitlements requested.
