#!/bin/sh
# Dev-only: build the Swift app against an existing Rust lib and bundle it at dist/CutOut.app.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT/macos"
export CUTOUT_RUST_LIB_DIR="${CUTOUT_RUST_LIB_DIR:-$ROOT/../cutout/target/release}"
swift build -c release --arch arm64
A="$ROOT/dist/CutOut.app"
cp "$(swift build -c release --arch arm64 --show-bin-path)/CutOut" "$A/Contents/MacOS/CutOut"
codesign -s - --force "$A"
