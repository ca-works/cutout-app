#!/bin/sh
# Downloads the real-photo benchmark set (not committed; see images.tsv for licences).
set -eu
cd "$(dirname "$0")"
mkdir -p images
grep -v '^#' images.tsv | while IFS="$(printf '\t')" read -r name cat lic url; do
  [ -f "images/$name" ] || curl -sfL -A "CutOutBench/0.1" -o "images/$name" "$url"
  echo "$name  $(shasum -a 256 "images/$name" | cut -c1-16)  $lic"
done
