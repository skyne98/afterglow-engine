#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
: "${DISPLAY:=:0}"
export DISPLAY
REPETITIONS="${VT_GPU_REPETITIONS:-3}"

nix-shell shell.nix --run \
  'cargo build -p afterglow-cef --example vt-demo && cargo build -p latency-tool --release'

cleanup() { pkill -f 'target/debug/examples/vt-demo' 2>/dev/null || true; }
trap cleanup EXIT
cleanup

for repetition in $(seq 1 "$REPETITIONS"); do
  LOG="${TMPDIR:-/tmp}/afterglow-vt-gpu-test-${repetition}.log"
  rm -f "$LOG"
  nix-shell shell.nix --run \
    'DISPLAY='"$DISPLAY"' ./target/debug/examples/vt-demo --ozone-platform=x11' >"$LOG" 2>&1 &
  PID=$!

  ready=false
  for _ in $(seq 1 60); do
    if grep -q 'DevTools listening' "$LOG" && \
       ./target/release/latency-tool eval 'typeof window.__afterglowVtGpuTest' 127.0.0.1:9222 2>/dev/null | grep -q 'object'; then
      ready=true; break
    fi
    if ! kill -0 "$PID" 2>/dev/null; then
      echo "VT demo exited before repetition $repetition initialized" >&2
      tail -100 "$LOG" >&2; exit 1
    fi
    sleep 1
  done
  if [[ "$ready" != true ]]; then
    echo "VT GPU repetition $repetition timed out" >&2; tail -100 "$LOG" >&2; exit 1
  fi

  RESULT="$(./target/release/latency-tool eval \
    'window.__afterglowVtGpuTest.run().then(JSON.stringify)' 127.0.0.1:9222)"
  echo "repetition $repetition/$REPETITIONS: $RESULT"
  if ! grep -q '\\"ok\\":true' <<<"$RESULT"; then
    echo "VT GPU repetition $repetition did not report success" >&2
    tail -100 "$LOG" >&2; exit 1
  fi
  # Every internal path must report three successful executions.
  for direction in east west rotated; do
    grep -Fq "\\\"direction\\\":\\\"$direction\\\"" <<<"$RESULT" || { echo "missing feedback direction $direction" >&2; exit 1; }
  done
  [[ "$(grep -Fo '\"valid\":1024' <<<"$RESULT" | wc -l)" -eq 3 ]] || { echo 'feedback path count mismatch' >&2; exit 1; }
  for scenario in eastbound westbound diagonal-lod; do
    grep -Fq "\\\"name\\\":\\\"$scenario\\\"" <<<"$RESULT" || { echo "missing residency scenario $scenario" >&2; exit 1; }
  done
  grep -Fq '\"rgba\":3' <<<"$RESULT" || { echo 'RGBA path count mismatch' >&2; exit 1; }
  if grep -Eq 'Uncaught|GPUValidationError|GPUDevice lost' "$LOG"; then
    echo "VT GPU validation failure in repetition $repetition" >&2
    tail -100 "$LOG" >&2; exit 1
  fi
  cleanup
  wait "$PID" 2>/dev/null || true
done

echo "VT real-GPU regression: PASS ($REPETITIONS independent runs; each internal path exercised 3×)"
