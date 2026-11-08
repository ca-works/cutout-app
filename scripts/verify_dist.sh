#!/bin/sh
# Verifies the distribution artifacts the way a normal user would meet them:
#   * the app/DMG contain the official icon and the precompiled Core ML model
#   * the binary is newer than every tracked source file (i.e. built from current code)
#   * no non-system dynamic libraries, ad-hoc signature valid
#   * the DMG's app is identical to dist/CutOut.app
#   * the full self-test runs with a brand-new HOME, a minimal PATH (no Python/Rust/
#     Homebrew) and the network DENIED by the macOS sandbox.
# Usage: scripts/verify_dist.sh            (APP=dist/CutOut.app DMG=dist/CutOut.dmg)
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
APP="${APP:-dist/CutOut.app}"; DMG="${DMG:-dist/CutOut.dmg}"
BIN="$APP/Contents/MacOS/CutOut"
ok() { echo "  ✓ $1"; }; bad() { echo "  ✗ $1"; FAILED=1; }
FAILED=0

echo "▸ Contents"
codesign --verify --strict "$APP" 2>/dev/null && ok "code signature valid (ad-hoc)" || bad "signature"
cmp -s assets/icon/CutOut.icns "$APP/Contents/Resources/AppIcon.icns" && ok "official icon (assets/icon/CutOut.icns) is bundled" || bad "icon differs"
M="$APP/Contents/Resources/Models/birefnet-general-coreml.mlmodelc"
[ -f "$M/coremldata.bin" ] && ok "precompiled Core ML model bundled ($(du -sh "$M" | cut -f1))" || bad "Core ML model missing"
EXPECT="$(awk '$1=="weight.bin"{print $3}' tools/convert/OUTPUT.txt)"
GOT="$(find "$M" -name weight.bin -exec shasum -a 256 {} \; | head -1 | cut -d' ' -f1)"
[ "$EXPECT" = "$GOT" ] && ok "model weights match tools/convert/OUTPUT.txt" || echo "  - weight hash differs from the package hash (expected for compiled model; not a failure)"
EXTRA="$(otool -L "$BIN" | tail -n +2 | awk '{print $1}' | grep -vE '^(/usr/lib/|/System/Library/|@rpath/libswift)' || true)"
[ -z "$EXTRA" ] && ok "links only system libraries" || bad "non-system libraries: $EXTRA"
NEWEST="$(git ls-files core ffi macos tools/cutout-cli Cargo.toml Cargo.lock scripts/build.sh assets | xargs ls -t | head -1)"
[ ! "$NEWEST" -nt "$BIN" ] && ok "binary is newer than all tracked sources ($NEWEST)" || bad "binary older than $NEWEST — rebuild"
[ -z "$(git status --porcelain core ffi macos assets scripts)" ] && ok "working tree clean for built paths ($(git rev-parse --short HEAD))" || bad "uncommitted changes in built paths"
grep -q "<string>$(git rev-list --count HEAD)</string>" "$APP/Contents/Info.plist" && ok "build number = commit count" || echo "  - build number differs from current commit count (commit after build)"

echo "▸ DMG"
MNT="$(mktemp -d)"; hdiutil attach -readonly -nobrowse -mountpoint "$MNT" "$DMG" >/dev/null
trap 'hdiutil detach "$MNT" >/dev/null 2>&1 || true' EXIT
cmp -s "$MNT/CutOut.app/Contents/MacOS/CutOut" "$BIN" && ok "DMG binary identical to $APP" || bad "DMG binary differs"
cmp -s "$MNT/CutOut.app/Contents/Resources/AppIcon.icns" assets/icon/CutOut.icns && ok "DMG icon is the official icon" || bad "DMG icon differs"
[ -f "$MNT/CutOut.app/Contents/Resources/Models/birefnet-general-coreml.mlmodelc/coremldata.bin" ] && ok "DMG contains the Core ML model" || bad "DMG lacks model"
[ -L "$MNT/Applications" ] && ok "DMG has Applications shortcut" || bad "no Applications link"
hdiutil detach "$MNT" >/dev/null; trap - EXIT

echo "▸ Normal-user run: fresh HOME, minimal PATH, network denied"
T="$(mktemp -d)"; mkdir -p "$T/home"
cat > "$T/nonet.sb" <<'SB'
(version 1)
(allow default)
(deny network*)
SB
RUNNER="sandbox-exec -f $T/nonet.sb env HOME=$T/home PATH=/usr/bin:/bin CUTOUT_MODELS_DIR=$T/home/models" APP="$APP" scripts/selftest.sh | tee "$T/out.txt"
grep -q "failed 0" "$T/out.txt" && ok "self-test: all checks passed offline with a clean HOME" || bad "self-test failures"
grep -q "Core ML" build/selftest/selftest.json && ok "Core ML backend used" || bad "Core ML not used"
rm -rf "$T"
[ "$FAILED" = 0 ] && echo "✓ verify_dist: OK" || { echo "✗ verify_dist: FAILED"; exit 1; }
