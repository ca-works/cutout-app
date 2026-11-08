#!/bin/sh
# Full local recipe: pinned venv → pinned sources → Core ML conversion →
# verify the output weights → install for the app. Nothing is uploaded.
# Usage: tools/convert/install.sh            (≈5 min, ~9 GB RAM peak)
set -eu
cd "$(dirname "$0")/../.."
V=tools/convert/.venv
if [ ! -x "$V/bin/python" ]; then
  /usr/bin/python3 -m venv "$V"
  "$V/bin/pip" install -q --require-hashes --no-deps -r tools/convert/requirements.lock
fi
tools/convert/fetch.sh
OUT=build/convert/BiRefNet.mlpackage
rm -rf "$OUT"
"$V/bin/python" -I tools/convert/convert.py --src build/convert/src --out "$OUT" --skip-check 2>&1 | grep -E "verified|replaced|converted|weight_bin|rror"
W="$OUT/Data/com.apple.CoreML/weights/weight.bin"
EXPECT=$(grep '^weight.bin' tools/convert/OUTPUT.txt | awk '{print $3}')
GOT=$(shasum -a 256 "$W" | cut -d' ' -f1)
if [ "$GOT" != "$EXPECT" ]; then
  echo "✗ converted weights differ from the pinned output ($GOT != $EXPECT); not installing"
  exit 1
fi
DEST="$HOME/Library/Application Support/CutOut/Models/birefnet-general-coreml.mlpackage"
mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
cp -R "$OUT" "$DEST"
echo "✓ installed $DEST"
