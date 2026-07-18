#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
: "${DISPLAY:=:0}"; export DISPLAY
nix-shell shell.nix --run 'cargo build -p afterglow-cef --example lod-demo && cargo build -p latency-tool --release'
cleanup(){ pkill -f '^./target/debug/examples/lod-demo' 2>/dev/null || true; }
trap cleanup EXIT; cleanup
LOG="${TMPDIR:-/tmp}/afterglow-lod.log"; rm -f "$LOG"
nix-shell shell.nix --run 'DISPLAY='"$DISPLAY"' ./target/debug/examples/lod-demo --ozone-platform=x11' >"$LOG" 2>&1 & PID=$!
ready=false
for _ in $(seq 1 60); do
  if ./target/release/latency-tool eval 'typeof window.__afterglowLod' 127.0.0.1:9222 2>/dev/null | grep -q object; then ready=true; break; fi
  sleep 1
done
[[ "$ready" == true ]] || { tail -100 "$LOG"; exit 1; }
RESULT="$(./target/release/latency-tool eval '(async()=>JSON.stringify(await window.__afterglowLod.run()))()' 127.0.0.1:9222)"
echo "$RESULT"
grep -Fq '\"ok\":true' <<<"$RESULT" || { tail -100 "$LOG"; exit 1; }
grep -Fq '\"levels\":[0,1,2,3,2,1,0]' <<<"$RESULT" || { tail -100 "$LOG"; exit 1; }
if grep -Eq 'Uncaught|GPUValidationError|GPUDevice lost|Multiple instances of Three' "$LOG"; then tail -100 "$LOG"; exit 1; fi
cleanup; wait "$PID" 2>/dev/null || true
echo 'Static LOD GPU regression: PASS'
