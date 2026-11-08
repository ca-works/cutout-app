# BiRefNet → Core ML conversion

**Developers only.** Builds the Core ML model that CutOut's **Best Quality**
mode uses on Apple Silicon (CPU+GPU) from the original PyTorch release of
BiRefNet (ZhengPeng7, **MIT**). End users never run this: they get the model
precompiled inside the full app (`BUNDLE_COREML=1 scripts/build.sh`) or as a
verified archive for **Install from File…** (`MODEL_ARCHIVE=1`). Everything
runs locally; the converted model has not been uploaded or published, and the
repository does not contain it.

```sh
tools/convert/install.sh
```

does the whole recipe (≈5 minutes, ≈9 GB peak memory, ~3 GB disk):

1. creates `tools/convert/.venv` with the system Python 3.9 and installs the
   exact wheels in `requirements.lock` (`pip --require-hashes`; torch 2.7.0,
   coremltools 9.0, …);
2. `fetch.sh` downloads `birefnet.py`, `BiRefNet_config.py` and
   `model.safetensors` from `huggingface.co/ZhengPeng7/BiRefNet` at the pinned
   revision `66f7f80c…` and checks size + SHA-256 against `SOURCES.txt`
   (same 2024-07-28 weights as the onnx-community ONNX export the CPU path
   uses);
3. `convert.py` converts to an fp16 ML Program with a static 1×3×1024×1024
   input (see below) — `python -I`, safetensors only, no pickle;
4. checks the SHA-256 of the produced `weight.bin` against `OUTPUT.txt` and
   copies the package to
   `~/Library/Application Support/CutOut/Models/birefnet-general-coreml.mlpackage`.

The app verifies the same weight hash before it uses an installed package,
compiles it once into `~/Library/Caches/CutOut/coreml/compiled`, and falls
back to the ONNX CPU model if Core ML fails and an ONNX model is installed
(the ONNX model is optional).

## What the converter changes

| Problem | Fix |
|---|---|
| Core ML has no deformable convolution (ORT's CoreML EP failed on it) | Each of the 20 `DeformableConv2d` (modulated, torchvision `deform_conv2d`) is rewritten exactly as Σₖ W₍ₖ₎·(maskₖ · bilinear-sample(x, p + pₖ + Δpₖ)) with `grid_sample(align_corners=True, zeros)` — same out-of-range behaviour as torchvision. PyTorch check: max \|Δp\| 8·10⁻⁵ vs the original. |
| Core ML tensors are rank ≤ 5; Swin `window_partition`/`window_reverse` and BiRefNet `image2patches` use rank-6 reshapes | Equivalent rank-5 versions (batch fixed to 1). |
| `transformers` / `kornia` imports | Stubs: only used for the Hugging Face wrapper class and a training-only branch. |
| Output | Only the final map, passed through sigmoid (probabilities). |

`convert.py` without `--skip-check` also runs the original and the
decomposed model in PyTorch and refuses to convert if they differ.

## Checking parity and speed

```sh
bench/run_coreml.sh build/convert/BiRefNet.mlpackage all   # or ane | gpu | cpu
```

runs the package through the app's own `CoreMLSegmenter` and the shared Rust
pre/post-processing on `bench/all`, then `cutout-cli score` compares it with
ground truth and with the ONNX CPU outputs from `bench/run_models.sh cpu`.
Results are in `docs/MODELS.md` and `docs/PERFORMANCE.md`.
