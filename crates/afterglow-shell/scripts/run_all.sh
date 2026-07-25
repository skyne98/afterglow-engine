#!/usr/bin/env bash
# Run every webgpu_* example that has an upstream screenshot.
#
# Results distinguish:
#   PASS  rendered and perceptual diff is below DIFF_THRESHOLD
#   FAIL  rendered, but exceeded the pixel threshold
#   ERR   runtime/load/evaluation/timeout/diff failure
#
# Every run starts from a deleted output PNG, so a stale image can never turn a
# failed process into a render success. Full JS stacks remain in /tmp/runs/*.log.
set -uo pipefail
CRATE_ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORKSPACE_ROOT=$(cd "$CRATE_ROOT/../.." && pwd)
cd "$CRATE_ROOT"

XLIBS=(libxcb.so.1 libX11.so.6 libX11-xcb.so.1 libXcursor.so.1 libXrandr.so.1 libXi.so.6 libXrender.so.1 libXext.so.6 libXfixes.so.3 libXau.so.6 libXdmcp.so.6 libXdamage.so.1 libxkbcommon.so libxcb-render.so.0)
LP=""
for lib in "${XLIBS[@]}"; do
  d=$(find /nix/store -maxdepth 3 -name "$lib" 2>/dev/null | head -1)
  [ -n "$d" ] && d=$(dirname "$d")
  [ -n "$d" ] && LP="$LP:$d"
done
export LD_LIBRARY_PATH="${LP#:}:/run/opengl-driver/lib"
if [ "${PIXEL_GPU:-lavapipe}" = "lavapipe" ]; then
  LVP_ICD=$(find /nix/store -path '*/share/vulkan/icd.d/lvp_icd.x86_64.json' 2>/dev/null | head -1)
  export VK_ICD_FILENAMES="$LVP_ICD"
else
  export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json}
fi
export VK_DRIVER_FILES="$VK_ICD_FILENAMES"
export NO_ENABLE_TIMELINE_SEMAPHORE=1
export RUSTY_V8_ARCHIVE=${RUSTY_V8_ARCHIVE:-/tmp/v149.a}
unset WAYLAND_DISPLAY DISPLAY

export ROOT=$CRATE_ROOT
export BROWSER_TEST=$WORKSPACE_ROOT/target/debug/examples/browser_test
export VENDOR=${1:-/tmp/threejs}
export SUITE=${SUITE:-game}
export EXCLUSIONS="$ROOT/e2e/game-engine-exclusions.txt"
export DIFF_THRESHOLD=${DIFF_THRESHOLD:-0.1}
export TEST_TIMEOUT=${TEST_TIMEOUT:-40}
export RESULT_DIR=/tmp/afterglow-shell-results
mkdir -p /tmp/refs /tmp/runs "$RESULT_DIR"
rm -f "$RESULT_DIR"/*.tsv /tmp/batch.txt

summarize_error() {
  local log=$1 summary
  summary=$(grep -A6 -m1 '^\[host\]' "$log" 2>/dev/null | tr '\n\t' '  ' | tr -s ' ' | head -c 500)
  if [ -z "$summary" ]; then
    summary=$(tail -n 6 "$log" 2>/dev/null | tr '\n\t' '  ' | tr -s ' ' | head -c 500)
  fi
  printf '%s' "$summary"
}

run_one() {
  local html=$1 ex ref out log result status pct reason
  ex=$(basename "$html" .html)
  if [ "$SUITE" != "all" ] && grep -qx "$ex" "$EXCLUSIONS"; then return 0; fi
  ref="$VENDOR/examples/screenshots/$ex.jpg"
  [ -f "$ref" ] || return 0

  out="/tmp/runs/$ex.png"
  log="/tmp/runs/$ex.log"
  result="$RESULT_DIR/$ex.tsv"
  rm -f "$out" "$log" "$result"

  timeout -s KILL "$TEST_TIMEOUT" \
    "$BROWSER_TEST" "$VENDOR" "$ex" "$out" \
    >"$log" 2>&1
  status=$?

  if [ "$status" -ne 0 ] || [ ! -s "$out" ]; then
    reason=$(summarize_error "$log")
    printf 'ERR\t%s\texit=%s %s\t%s\n' "$ex" "$status" "$reason" "$log" >"$result"
    return
  fi

  pct=$(cd "$ROOT/cdp_client" && bun "$ROOT/e2e/diff_pct.ts" "$out" "$ref" 2>>"$log")
  status=$?
  if [ "$status" -ne 0 ] || ! [[ "$pct" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    reason=$(summarize_error "$log")
    printf 'ERR\t%s\tdiff exit=%s %s\t%s\n' "$ex" "$status" "$reason" "$log" >"$result"
  elif awk "BEGIN { exit !($pct < $DIFF_THRESHOLD) }"; then
    printf 'PASS\t%s\t%s%%\t%s\n' "$ex" "$pct" "$log" >"$result"
  else
    printf 'FAIL\t%s\t%s%%\t%s\n' "$ex" "$pct" "$log" >"$result"
  fi
}
export -f run_one summarize_error
export BROWSER_TEST

# Parallelism is opt-in because concurrent GPU processes can alter timing and
# memory pressure. Use JOBS=1 for the canonical result.
JOBS=${JOBS:-1}
find "$VENDOR/examples" -maxdepth 1 -type f -name 'webgpu_*.html' -print0 \
  | sort -z \
  | xargs -0 -r -n1 -P "$JOBS" bash -c 'run_one "$1"' _

find "$RESULT_DIR" -maxdepth 1 -type f -name '*.tsv' -print0 \
  | sort -z \
  | xargs -0 -r cat > /tmp/batch.txt

P=$(awk -F '\t' '$1 == "PASS" { n++ } END { print n+0 }' /tmp/batch.txt)
F=$(awk -F '\t' '$1 == "FAIL" { n++ } END { print n+0 }' /tmp/batch.txt)
E=$(awk -F '\t' '$1 == "ERR"  { n++ } END { print n+0 }' /tmp/batch.txt)
T=$((P + F + E))
R=$((P + F))
awk -v p="$P" -v f="$F" -v e="$E" -v t="$T" -v r="$R" -v threshold="$DIFF_THRESHOLD" 'BEGIN {
  render_rate = t ? 100*r/t : 0;
  suite_pass_rate = t ? 100*p/t : 0;
  rendered_pass_rate = r ? 100*p/r : 0;
  printf "=== SUMMARY total=%d pass=%d fail=%d err=%d threshold=%s%% ===\n", t, p, f, e, threshold;
  printf "=== RATES render_success=%.2f%% suite_pixel_pass=%.2f%% rendered_pixel_pass=%.2f%% ===\n", render_rate, suite_pass_rate, rendered_pass_rate;
}' >> /tmp/batch.txt

cat /tmp/batch.txt

if [ "$F" -ne 0 ] || [ "$E" -ne 0 ]; then
  exit 1
fi
