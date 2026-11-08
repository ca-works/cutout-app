#!/bin/sh
# Build CutOut.app (and optionally CutOut.dmg) locally. No network access is
# needed after the first Rust dependency fetch; nothing is uploaded.
#
#   scripts/build.sh            # release build → dist/CutOut.app
#   scripts/build.sh --dmg      # also create dist/CutOut.dmg
#   BUNDLE_COREML=1 scripts/build.sh --dmg   # "full" build: embed the precompiled Core ML
#                                            # BiRefNet (≈ 445 MB) — no setup, fastest, offline
#   BUNDLE_MODEL=1 scripts/build.sh   # also embed the ONNX CPU fallback model (≈ 973 MB)
#   MODEL_ARCHIVE=1 scripts/build.sh  # also write dist/birefnet-general-coreml.zip for "Install from File…"
# Bundled/archived models are never uploaded by this script.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET=14.0
VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
BUILD_NUM="$(git rev-list --count HEAD 2>/dev/null || echo 1)"
APP="${APP_OUT:-dist/CutOut.app}"

echo "▸ Rust engine (release)"
cargo build --release -p cutout-ffi

echo "▸ Swift app (release)"
(cd macos && swift build -c release --arch arm64)
BIN="$(cd macos && swift build -c release --arch arm64 --show-bin-path)/CutOut"

echo "▸ Bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/CutOut"
cp assets/icon/CutOut.icns "$APP/Contents/Resources/AppIcon.icns"
cp LICENSES/THIRD_PARTY_NOTICES.md "$APP/Contents/Resources/" 2>/dev/null || true
mkdir -p "$APP/Contents/Resources/Fonts"
cp assets/fonts/MonaSans-Latin.woff2 LICENSES/MonaSans-OFL.txt "$APP/Contents/Resources/Fonts/"
if [ "${BUNDLE_MODEL:-0}" = "1" ]; then
  M="$HOME/Library/Application Support/CutOut/Models/birefnet-general-fp32.onnx"
  [ -f "$M" ] || { echo "model not installed at $M"; exit 1; }
  mkdir -p "$APP/Contents/Resources/Models"
  cp "$M" "$APP/Contents/Resources/Models/"
fi
if [ "${BUNDLE_COREML:-0}" = "1" ] || [ "${MODEL_ARCHIVE:-0}" = "1" ]; then
  PKG="${COREML_PACKAGE:-$HOME/Library/Application Support/CutOut/Models/birefnet-general-coreml.mlpackage}"
  [ -d "$PKG" ] || { echo "Core ML package not found at $PKG (run tools/convert/install.sh)"; exit 1; }
  EXPECT="$(awk '$1=="weight.bin"{print $3}' tools/convert/OUTPUT.txt)"
  GOT="$(shasum -a 256 "$PKG/Data/com.apple.CoreML/weights/weight.bin" | cut -d' ' -f1)"
  [ "$EXPECT" = "$GOT" ] || { echo "Core ML weights checksum mismatch ($GOT)"; exit 1; }
  echo "  Core ML package verified ($GOT)"
fi
if [ "${BUNDLE_COREML:-0}" = "1" ]; then
  echo "▸ Precompile Core ML model (deployment target macOS 14)"
  rm -rf build/coreml && mkdir -p build/coreml "$APP/Contents/Resources/Models"
  xcrun coremlcompiler compile "$PKG" build/coreml --deployment-target 14.0 >/dev/null
  cp -R build/coreml/*.mlmodelc "$APP/Contents/Resources/Models/birefnet-general-coreml.mlmodelc"
fi
if [ "${MODEL_ARCHIVE:-0}" = "1" ]; then
  echo "▸ Model archive for Install from File…"
  rm -f dist/birefnet-general-coreml.zip
  ditto -c -k --keepParent "$PKG" dist/birefnet-general-coreml.zip
  shasum -a 256 dist/birefnet-general-coreml.zip | awk '{print "  zip sha256: " $1}'
fi
sed -e "s/__VERSION__/$VERSION/g" -e "s/__BUILD__/$BUILD_NUM/g" macos/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

echo "▸ Ad-hoc code signature (not notarized)"
codesign --force --deep --sign - --options runtime --entitlements macos/CutOut.entitlements "$APP"
codesign --verify --strict "$APP" && echo "  signature OK (ad-hoc)"

du -sh "$APP" | awk '{print "  size: " $1}'

if [ "${1:-}" = "--dmg" ]; then
  echo "▸ DMG"
  STAGE="build/dmg"
  rm -rf "$STAGE" dist/CutOut.dmg
  mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "CutOut" -srcfolder "$STAGE" -ov -format UDZO -fs HFS+ dist/CutOut.dmg >/dev/null
  du -sh dist/CutOut.dmg | awk '{print "  dmg: " $1}'
fi
echo "✓ $APP"
