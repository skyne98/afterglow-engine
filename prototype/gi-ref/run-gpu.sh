#!/usr/bin/env bash
# Run the GPU stages in afterglow-shell and stop as soon as they are dumped.
#
#   prototype/gi-ref/run-gpu.sh [logname]
#
# Writes prototype/gi-ref/<logname>.log, then diffs against the CPU reference.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
LOG="prototype/gi-ref/${1:-gpu}.log"
: > "$LOG"
setsid nix-shell shell.nix --run \
  "AFTERGLOW_WINDOW_WIDTH=320 AFTERGLOW_WINDOW_HEIGHT=200 AFTERGLOW_STARTUP_TIMEOUT_MS=240000 \
   RUST_LOG=warn stdbuf -oL -eL ./target/release/afterglow-shell prototype/gi-ref/gpu.js" \
  > "$LOG" 2>&1 &
PID=$!
for _ in $(seq 1 200); do
  grep -q 'GIREF done' "$LOG" 2>/dev/null && break
  grep -q 'panicked' "$LOG" 2>/dev/null && break
  sleep 0.2
done
kill -TERM -- -$PID 2>/dev/null
sleep 1
kill -KILL -- -$PID 2>/dev/null
wait $PID 2>/dev/null
node prototype/gi-ref/compare.mjs "$LOG"
