#!/bin/sh
# Downloads the pinned BiRefNet sources listed in SOURCES.txt into
# build/convert/src and verifies size + SHA-256 of each file.
set -eu
cd "$(dirname "$0")/../.."
REV=66f7f80c1a64b3c4df327a07eeca4e87130b787b
DST=build/convert/src
mkdir -p "$DST"
grep -v '^#' tools/convert/SOURCES.txt | while read -r f bytes sha; do
  [ -n "$f" ] || continue
  if [ ! -f "$DST/$f" ] || [ "$(shasum -a 256 "$DST/$f" | cut -d' ' -f1)" != "$sha" ]; then
    echo "▸ $f"
    curl -fL --retry 3 -o "$DST/$f.part" "https://huggingface.co/ZhengPeng7/BiRefNet/resolve/$REV/$f"
    mv "$DST/$f.part" "$DST/$f"
  fi
  got=$(shasum -a 256 "$DST/$f" | cut -d' ' -f1)
  size=$(stat -f %z "$DST/$f")
  [ "$got" = "$sha" ] && [ "$size" = "$bytes" ] || { echo "✗ $f: sha256 $got size $size (expected $sha $bytes)"; exit 1; }
  echo "✓ $f"
done
