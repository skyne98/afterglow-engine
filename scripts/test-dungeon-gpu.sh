#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)";cd "$ROOT";: "${DISPLAY:=:0}";export DISPLAY
RUNS="${DUNGEON_REPETITIONS:-3}";nix-shell shell.nix --run 'cargo build -p afterglow-cef --example dungeon && cargo build -p latency-tool --release'
cleanup(){ pkill -f '^./target/debug/examples/dungeon' 2>/dev/null||true; };trap cleanup EXIT;cleanup
for run in $(seq 1 "$RUNS");do
 LOG="${TMPDIR:-/tmp}/afterglow-dungeon-$run.log";rm -f "$LOG";nix-shell shell.nix --run 'DISPLAY='"$DISPLAY"' ./target/debug/examples/dungeon --ozone-platform=x11' >"$LOG" 2>&1 & PID=$!
 ready=false;for _ in $(seq 1 60);do if ./target/release/latency-tool eval 'typeof window.__afterglowDungeon' 127.0.0.1:9222 2>/dev/null|grep -q object;then ready=true;break;fi;sleep 1;done
 [[ "$ready" == true ]]||{ tail -100 "$LOG";exit 1; }
 RESULT="$(./target/release/latency-tool eval '(async()=>{const a=window.__afterglowDungeon;a.setProgrammatic(true);const scenarios=[];for(const name of ["forward","reverse","corner"]){const s=await a.runScenario(name);scenarios.push({name,pose:s.pose,resident:s.atlasSlotsUsed,pending:s.pendingPages,errors:s.errors})}for(let i=0;i<600&&a.telemetry().cacheQueuedWrites;i++)await a.step(1);const final=a.snapshot(),cache={enabled:final.cacheEnabled,backend:final.cacheBackend,entries:final.cacheEntries,hits:final.cacheHits,misses:final.cacheMisses,writes:final.cacheWrites,errors:final.cacheErrors,queued:final.cacheQueuedWrites},pom=a.pomStatus(),pipelines=a.pipelineTelemetry();return JSON.stringify({ok:final.textures.length===9&&final.errors.length===0&&final.failedLoads===0&&cache.enabled&&cache.errors===0&&cache.queued===0&&pom.enabled&&pom.minLayers===8&&pom.maxLayers===32&&pom.maxOffsetRatio===2&&pom.heightSource==="resident ambientCG AO"&&pipelines.renderPipelineViolations===0&&pipelines.computePipelineViolations===0&&scenarios.every(s=>s.pending===0&&s.errors.length===0),textures:final.textures.map(t=>({id:t.textureId,path:t.path,size:t.virtualSize})),cache,pom,pipelines,scenarios})})()' 127.0.0.1:9222)"
 echo "run $run/$RUNS: $RESULT";grep -Fq '\"ok\":true'<<<"$RESULT"||{ tail -100 "$LOG";exit 1; };for s in forward reverse corner;do grep -Fq "\\\"name\\\":\\\"$s\\\""<<<"$RESULT"||exit 1;done
 grep -Eq 'Uncaught|GPUValidationError|GPUDevice lost' "$LOG"&&{ tail -100 "$LOG";exit 1; };cleanup;wait "$PID" 2>/dev/null||true
done
echo "Dungeon GPU regression: PASS ($RUNS independent launches × 3 viewpoints)"
