# Golden mattes (quality parity reference)

`<scene>.png` are the alpha mattes produced on the **macOS development host**
(Apple M1) by the shared engine's ONNX CPU path — BiRefNet general fp32
(`birefnet-general-fp32.onnx`, SHA-256 `58f621f0…ebd3c`), refinement on — for the
four deterministic synthetic scenes of `cutout-cli synth`. The scenes have exact
ground-truth alpha; the scores are in `docs/MODELS.md`
(gear IoU 0.9994, strands 0.9920, bracket 0.8344, translucent disc 0.1047).

A Windows (or any other) build is **at parity** when
`cutout-cli parity bench/synth <preds> birefnet-general-fp32_cpu bench/golden`
passes: mean absolute difference ≤ 0.004 and IoU of the binarised mattes
≥ 0.998 for every scene. ONNX Runtime kernels differ per CPU architecture, so
bit-identical output is not expected; these tolerances are far tighter than the
differences between models (see `docs/MODELS.md`).

Regenerate (only when the model or pipeline intentionally changes):

```sh
cutout-cli synth bench/synth
cutout-cli bench <birefnet-general-fp32.onnx> cpu bench/synth bench/synth_out
for n in chrome_bracket fine_strands translucent_disc white_gear_on_beige; do
  cutout-cli matte bench/synth_out/${n}__birefnet-general-fp32_cpu.png bench/golden/$n.png
done
```
