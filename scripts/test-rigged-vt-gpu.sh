#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
: "${DISPLAY:=:0}"; export DISPLAY
nix-shell shell.nix --run 'cargo build -p afterglow-cef --example rigged-vt-demo && cargo build -p latency-tool --release'
cleanup(){ pkill -f '^./target/debug/examples/rigged-vt-demo' 2>/dev/null || true; }
trap cleanup EXIT; cleanup
LOG="${TMPDIR:-/tmp}/afterglow-rigged-vt.log"; rm -f "$LOG"
nix-shell shell.nix --run 'DISPLAY='"$DISPLAY"' ./target/debug/examples/rigged-vt-demo --ozone-platform=x11' >"$LOG" 2>&1 & PID=$!
ready=false
for _ in $(seq 1 60); do
  if ./target/release/latency-tool eval 'typeof window.__afterglowRiggedVT' 127.0.0.1:9222 2>/dev/null | grep -q object; then ready=true; break; fi
  sleep 1
done
[[ "$ready" == true ]] || { tail -100 "$LOG"; exit 1; }
RESULT="$(./target/release/latency-tool eval '(async()=>{const a=window.__afterglowRiggedVT;a.setProgrammatic(true);a.setActiveModel(1);a.setAnimationEnabled(false);const ground=[];for(let t=0;t<5.85;t+=.5){a.setAnimationTime(t);await a.step(1);ground.push(a.measureBounds().minY)}a.setAnimationTime(0);a.setSkeletonVisible(true);await a.step(2);a.setSkeletonVisible(false);a.setProgrammatic(false);dispatchEvent(new KeyboardEvent("keydown",{key:"r"}));dispatchEvent(new KeyboardEvent("keyup",{key:"r"}));dispatchEvent(new KeyboardEvent("keydown",{key:"d"}));dispatchEvent(new KeyboardEvent("keydown",{key:"w"}));await a.step(20);dispatchEvent(new KeyboardEvent("keyup",{key:"d"}));dispatchEvent(new KeyboardEvent("keyup",{key:"w"}));const released=a.status();await a.step(12);const coasted=a.status();a.setProgrammatic(true);a.setActiveModel(2);a.setAnimationEnabled(true);await a.step(600);a.setAnimationEnabled(false);for(let i=0;i<20&&a.telemetry().pendingPages!==0;i++)await a.step(30);const dragon=a.status(),dragonGround=a.measureBounds().minY,stats=a.telemetry(),textures=a.debugSnapshot().textures,required=[21,22,23,30,31,32,42,43,44].map(i=>textures.find(x=>x.path.endsWith("#image-"+i)));return JSON.stringify({ok:a.errorCount()===0&&coasted.skinnedMeshes===1&&coasted.bones===26&&coasted.shadows&&coasted.shadowMapSize===2048&&coasted.meshOptimized&&ground.every(y=>Math.abs(y)<.01)&&released.orbitAngle!==0&&released.cameraDistance<4.1&&Math.abs(coasted.orbitAngle)>Math.abs(released.orbitAngle)&&coasted.cameraDistance<released.cameraDistance&&dragon.activeModel===2&&dragon.meshes===18&&dragon.skinnedMeshes===18&&dragon.clip==="Idle"&&dragon.feedbackChannels===1&&dragon.shadows&&dragon.shadowMapSize===2048&&required.every(x=>x&&x.residentPages>0)&&dragon.meshOptimized&&dragon.sameMeshFeedback&&dragon.rendererSealed&&dragon.pipelineViolations===0&&Math.abs(dragonGround)<.01&&stats.textureCount===48&&stats.pendingPages===0,first:coasted,dragon,ground,dragonGround,controls:{released:[released.orbitAngle,released.cameraDistance],coasted:[coasted.orbitAngle,coasted.cameraDistance]},resident:stats.atlasSlotsUsed,pending:stats.pendingPages,required:required.map(x=>[x.path,x.residentPages]),errors:a.errors()})})()' 127.0.0.1:9222)"
echo "$RESULT"
grep -Fq '\"ok\":true' <<<"$RESULT" || { tail -100 "$LOG"; exit 1; }
if grep -Eq 'Uncaught|GPUValidationError|GPUDevice lost|Multiple instances of Three' "$LOG"; then tail -100 "$LOG"; exit 1; fi
cleanup; wait "$PID" 2>/dev/null || true
echo 'Rigged VT GPU regression: PASS'
