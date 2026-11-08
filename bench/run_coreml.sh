#!/bin/sh
# Builds bench/coreml_bench.swift against the Rust FFI and benchmarks the
# Core ML BiRefNet on bench/all, then scores it against ground truth and
# against the ONNX CPU outputs (parity). See docs/MODELS.md.
# Usage: bench/run_coreml.sh <BiRefNet.mlpackage> [all|ane|gpu|cpu]
set -eu
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
PKG="$1"; UNITS="${2:-all}"
cargo build --release -q -p cutout-ffi -p cutout-cli
mkdir -p build/bench
cp bench/coreml_bench.swift build/bench/main.swift   # top-level code must live in main.swift
swiftc -O -import-objc-header ffi/include/cutout.h -Ltarget/release -lcutout_ffi -lc++ -liconv \
  -framework CoreML -framework Accelerate \
  macos/Sources/CutOut/Model/CoreMLSegmenter.swift build/bench/main.swift -o build/bench/coreml_bench
/usr/bin/time -l build/bench/coreml_bench "$PKG" bench/all bench/out "$UNITS" 2>&1 | grep -E "model|warm|image|peak|maximum resident|rror"
./target/release/cutout-cli score bench/all bench/out "birefnet-coreml_$UNITS" "birefnet-general-fp32_cpu"
