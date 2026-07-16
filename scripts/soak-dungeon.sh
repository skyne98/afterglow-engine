#!/usr/bin/env bash
set -euo pipefail
DURATION="${1:-600}"
SCENARIO="${2:-traverse}"
OUTPUT="${3:-vt-soak-${SCENARIO}-${DURATION}s-$(date +%Y%m%d-%H%M%S).log}"
PORT="${AFTERGLOW_DEVTOOLS_PORT:-9222}"
[[ "$DURATION" =~ ^[1-9][0-9]*$ ]] || { echo "duration must be positive seconds" >&2; exit 2; }
case "$SCENARIO" in stable|traverse|thrash) ;; *) echo "scenario must be stable, traverse, or thrash" >&2; exit 2;; esac
[[ -x target/release/latency-tool ]] || cargo build --release -p latency-tool
curl -sf "http://127.0.0.1:${PORT}/json/version" >/dev/null || {
  echo "no VT dungeon DevTools endpoint on 127.0.0.1:${PORT}; run DISPLAY=:0 ./scripts/run-dungeon.sh first" >&2
  exit 1
}
cat >&2 <<EOF
Running ${SCENARIO} VT soak for ${DURATION}s. The desktop session must remain unlocked.
Raw CDP output: ${OUTPUT}
EOF
EXPR="(async()=>{const duration=${DURATION}*1000,scenario='${SCENARIO}',a=window.__afterglowDungeon;if(!a)throw new Error('VT dungeon API unavailable');a.setProgrammatic(true);a.setHudVisible(false);await a.resolveGpuTimings();a.setGpuTimingEnabled(false);const started=performance.now(),samples=[],heapStart=performance.memory?.usedJSHeapSize??null;let second=0,count=0,sum=0,max=0,over17=0,previous=-1,frame=0,longTaskCount=0,longTaskMs=0,longTaskMaxMs=0;const observer=typeof PerformanceObserver==='function'&&PerformanceObserver.supportedEntryTypes?.includes('longtask')?new PerformanceObserver(list=>{for(const entry of list.getEntries()){longTaskCount++;longTaskMs+=entry.duration;longTaskMaxMs=Math.max(longTaskMaxMs,entry.duration)}}):null;observer?.observe({entryTypes:['longtask']});await new Promise(resolve=>{function tick(now){if(previous>=0){const dt=now-previous;count++;sum+=dt;if(dt>max)max=dt;if(dt>17)over17++}previous=now;const elapsed=now-started;if(scenario==='traverse'){const u=(elapsed%8000)/8000;a.setPose(-7.45+14.9*(u<.5?u*2:(1-u)*2),-7.65,0,0)}else if(scenario==='thrash'){const poses=[[-7.4,-7.4,0],[7.4,-7.4,3.14159],[7.4,7.4,3.14159],[-7.4,7.4,0],[-2.6,-4,-1.57],[2.6,-4,1.57],[-1.6,3,-1.57],[3.6,6,1.57]];const p=poses[frame%poses.length];a.setPose(p[0],p[1],p[2],0)}else a.setPose(-5.5+Math.sin(elapsed*.001)*.05,-5.5,0,0);frame++;const nextSecond=Math.floor(elapsed/1000);if(nextSecond>second){const s=a.telemetry(),t=a.timing(),m=performance.memory;samples.push({second,frames:count,meanMs:count?sum/count:0,maxMs:max,over17,resident:s.atlasSlotsUsed,pending:s.pendingPages,pendingBytes:s.pendingBytes,scheduled:s.scheduledRequests,readyUploads:s.readyUploads,failed:s.failedLoads,overflows:s.schedulerOverflows,evictions:s.cacheEvictions,vtCpuUs:t.vtCpuUs,renderSubmitUs:t.renderSubmitUs,feedbackSubmitUs:t.feedbackSubmitUs,frameCpuUs:t.frameCpuUs,gpuMainMs:t.gpuMainMs,gpuFeedbackMs:t.gpuFeedbackMs,gpuTotalMs:t.gpuTotalMs,usedJsHeap:m?.usedJSHeapSize??null});second=nextSecond;count=0;sum=0;max=0;over17=0}if(elapsed<duration)requestAnimationFrame(tick);else resolve()}requestAnimationFrame(tick)});await a.waitForIdle(5000);observer?.disconnect();const heapEnd=performance.memory?.usedJSHeapSize??null,final={...a.telemetry(),errors:a.errorCount()};a.setPose(-5.5,-5.5,0,0);a.setProgrammatic(false);a.setHudVisible(true);a.setGpuTimingEnabled(true);return JSON.stringify({scenario,durationSeconds:${DURATION},heapStart,heapEnd,heapDelta:heapStart===null||heapEnd===null?null:heapEnd-heapStart,longTaskCount,longTaskMs,longTaskMaxMs,gpuTimestampSupported:a.timing().gpuTimestampSupported,gpuMainMs:a.timing().gpuMainMs,gpuFeedbackMs:a.timing().gpuFeedbackMs,gpuTotalMs:a.timing().gpuTotalMs,pipelines:a.pipelineTelemetry(),samples,final})})()"
./target/release/latency-tool eval "$EXPR" "127.0.0.1:${PORT}" | tee "$OUTPUT"
