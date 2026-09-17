#!/usr/bin/env bash
# Fast demo runner. Starts the native shell, waits only for the capture marker in
# the log, then kills it. This is the whole point: a run costs ~4 s instead of
# waiting out the shell's startup timeout (~70 s), and the soak mode replaces the
# marker with a wall-clock budget.
#
#   prototype/probe-gi-demo/run.sh [logname] [page] [seconds]
#
#   logname   output log + frame-<n>.png prefix      (default last-run)
#   page      shell path, relative to the repo root  (default index.html)
#             e.g. prototype/probe-gi-demo/configs/complex-orbit.js
#   seconds   soak: run this long instead of stopping at the capture marker
#
# Writes <logname>.log and decodes every captured view to frame-<n>.png
set -uo pipefail
cd /home/fox/dev/afterglow-engine || exit 1
# Preflight: a syntax error in any demo module (a stray backtick inside a WGSL template
# literal is the usual one) costs a shell launch and a startup timeout to discover.
for module in demo.js bvh.js gi.wgsl.js render.wgsl.js; do
  node --check "prototype/probe-gi-demo/$module" || { echo "preflight failed: $module"; exit 1; }
done
LOG="prototype/probe-gi-demo/${1:-last-run}.log"
PAGE=${2:-prototype/probe-gi-demo/index.html}
SECS=${3:-0}
: > "$LOG"
W=${W:-960}; H=${H:-540}
# Own process group: the cleanup kill must not reach the caller's shell.
setsid nix-shell shell.nix --run \
  "AFTERGLOW_WINDOW_WIDTH=$W AFTERGLOW_WINDOW_HEIGHT=$H AFTERGLOW_STARTUP_TIMEOUT_MS=${SECS}000 \
   RUST_LOG=warn stdbuf -oL -eL ./target/release/afterglow-shell $PAGE" \
  > "$LOG" 2>&1 &
PID=$!
if [ "$SECS" -gt 0 ]; then
  sleep "$SECS"
else
  for _ in $(seq 1 150); do
    if grep -q 'capture-all-done' "$LOG" 2>/dev/null; then break; fi
    if grep -q 'panicked' "$LOG" 2>/dev/null; then break; fi
    sleep 0.2
  done
fi
kill -TERM -- -$PID 2>/dev/null
sleep 1
kill -KILL -- -$PID 2>/dev/null
wait $PID 2>/dev/null
# A failed shader compile would otherwise leave stale frame-*.png in place and look like
# a successful run with identical images.
if grep -q 'WGSL ERROR\|panicked' "$LOG"; then
  echo "run failed (shader compile or panic):"
  grep -m3 'WGSL ERROR\|panicked' "$LOG"
  exit 1
fi
python3 prototype/probe-gi-demo/decode.py "$LOG"
