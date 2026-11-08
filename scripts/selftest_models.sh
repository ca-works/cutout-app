#!/bin/sh
# Model-management scenarios for the end-to-end self-test.
#   FULL_APP   app built with BUNDLE_COREML=1 (default build/CutOut-full.app)
#   STD_APP    standard app (default dist/CutOut.app)
#   ARCHIVE    model archive from MODEL_ARCHIVE=1 (default dist/birefnet-general-coreml.zip)
# Uses empty temporary model folders (CUTOUT_MODELS_DIR) so the user's
# installed models are never modified.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
FULL_APP="${FULL_APP:-build/CutOut-full.app}"; STD_APP="${STD_APP:-dist/CutOut.app}"
ARCHIVE="${ARCHIVE:-dist/birefnet-general-coreml.zip}"
OUT="$ROOT/build/selftest-models"; rm -rf "$OUT"; mkdir -p "$OUT"
summary() {
  python3 - "$1/selftest.json" "$2" <<'PY'
import json,sys
r=json.load(open(sys.argv[1])); print(f"■ {sys.argv[2]}: passed {r['passed']}, failed {r['failed']}")
for c in r["checks"]:
    if (not c["ok"]) or any(k in c["name"] for k in ("Best","model","install")):
        print(("  ✓ " if c["ok"] else "  ✗ ")+c["name"]+"  — "+c["detail"])
PY
}
run() { # name app expect extra-env...
  name="$1"; app="$2"; expect="$3"; shift 3
  d="$OUT/$name"; mkdir -p "$d/models"
  env CUTOUT_SELFTEST="$d" CUTOUT_SELFTEST_BENCH="$ROOT/bench/all" CUTOUT_MODELS_DIR="$d/models" \
      CUTOUT_SELFTEST_EXPECT_BACKEND="$expect" "$@" "$app/Contents/MacOS/CutOut" >"$d/app.log" 2>&1
  summary "$d" "$name"
}
run fresh-user-full-app "$FULL_APP" "Core ML"
run install-from-file "$STD_APP" "Core ML" CUTOUT_SELFTEST_INSTALL_ARCHIVE="$ROOT/$ARCHIVE"
# Core ML fails at inference → automatic ONNX fallback (ONNX model linked into the temp folder)
mkdir -p "$OUT/coreml-fails-onnx-fallback/models"
ln -sf "$HOME/Library/Application Support/CutOut/Models/birefnet-general-fp32.onnx" "$OUT/coreml-fails-onnx-fallback/models/"
run coreml-fails-onnx-fallback "$FULL_APP" "ONNX" CUTOUT_COREML_FORCE_FAIL=1
run no-models "$STD_APP" "-"
