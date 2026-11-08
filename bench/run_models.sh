#!/bin/sh
# Benchmarks installed model variants on bench/all (see docs/MODELS.md).
# Usage: bench/run_models.sh [accel]   (accel: cpu (default) | coreml)
cd "$(dirname "$0")/.."
ACC="${1:-cpu}"
M="$HOME/Library/Application Support/CutOut/Models"
for name in birefnet-general-fp32 birefnet-lite-fp32 birefnet-general-fp16; do
  [ -f "$M/$name.onnx" ] || continue
  /usr/bin/time -l ./target/release/cutout-cli bench "$M/$name.onnx" "$ACC" bench/all bench/out 2>&1 | grep -E "model|warm|image|maximum resident|error"
done
