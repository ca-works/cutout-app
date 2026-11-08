#!/bin/sh
# Runs CutOut's end-to-end self test against dist/CutOut.app and verifies that
# Copy & Close leaves a transparent PNG on the clipboard after the app exits.
# RUNNER="<prefix command>" runs the app through a wrapper (e.g. scripts/verify_dist.sh).
# Results: build/selftest/selftest.json and window snapshots.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="$ROOT/build/selftest"
rm -rf "$OUT"; mkdir -p "$OUT"
BIN="${APP:-dist/CutOut.app}/Contents/MacOS/CutOut"
[ -x "$BIN" ] || { echo "build first: scripts/build.sh"; exit 1; }
[ -d bench/all ] || { echo "prepare bench/all (bench/fetch_images.sh + cutout-cli synth)"; exit 1; }

echo "▸ startup (5 cold-ish launches)"
for i in 1 2 3 4 5; do
  CUTOUT_STARTUP_PROBE="$OUT/startup_$i.json" ${RUNNER:-} "$BIN" >/dev/null 2>&1 || true
  cat "$OUT/startup_$i.json"
done

echo "▸ self test"
CUTOUT_SELFTEST="$OUT" CUTOUT_SELFTEST_BENCH="$ROOT/bench/all" CUTOUT_SELFTEST_COPYCLOSE=1 ${RUNNER:-} "$BIN" >"$OUT/app.log" 2>&1 || true
python3 - "$OUT/selftest.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]))
for c in r["checks"]:
    print(("  ✓ " if c["ok"] else "  ✗ ")+c["name"]+f"  [{c['ms']} ms]  "+c["detail"])
print(f"  passed {r['passed']}, failed {r['failed']}, resident at end {r['resident_mb_end']} MB")
PY

echo "▸ clipboard after Copy & Close (app has exited)"
if pgrep -x CutOut >/dev/null; then echo "  ✗ app still running"; else echo "  ✓ app exited"; fi
osascript -e 'clipboard info' | tr ',' '\n' | grep -E "PNGf|TIFF" | head -3 | sed 's/^/  /'
osascript -e 'try' -e 'set d to the clipboard as «class PNGf»' -e 'return "  ✓ PNG readable after exit"' -e 'on error' -e 'return "  ✗ no PNG"' -e 'end try'
