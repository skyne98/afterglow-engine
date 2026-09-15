# Native WebSocket capture comparison

## Method

Six fresh native processes used the same paint workload and history depth.
The order was control, capture, capture, control, control, capture.
Each process used an NVIDIA RTX 3090 and a selected cadence of 143.975 Hz.
Each phase measured 240 requestAnimationFrame intervals with four paint strokes.
The middle phase started approximately two seconds after readiness.
The last phase started at least twenty seconds after readiness and after capture disconnection.
Actual phase timestamps are in `summary.json` and each `result.json`.

Control processes had no collector connection.
Capture processes connected to the CLI WebSocket server.
The Vite build and TypeScript check completed before the six processes started.
The driver closed only its own processes.

## Middle-phase results

All values are milliseconds.

| Run | Mean | p99 | Maximum |
| --- | ---: | ---: | ---: |
| control-1 | 7.118454 | 18.614 | 19.865 |
| capture-1 | 7.322921 | 20.735 | 37.497 |
| capture-2 | 7.206412 | 18.803 | 36.171 |
| control-2 | 7.148492 | 19.127 | 21.993 |
| control-3 | 7.089979 | 18.258 | 19.620 |
| capture-3 | 7.205950 | 19.208 | 33.123 |

The median mean was 7.118454 ms for control and 7.206412 ms for capture.
The difference was 0.087958 ms (1.24%).
The median p99 was 18.614 ms for control and 19.208 ms for capture.
The difference was 0.594 ms (3.19%).
All capture maxima exceeded all control maxima.
The median maximum was 19.865 ms for control and 36.171 ms for capture.
These differences are observations from three processes per mode, not a statistical bound on capture overhead.

## Functional results and limits

All six pixel checks returned 954 colored pixels and zero capture failures.
All three capture files passed the driver checks for source identity, sequence, losses, RPC pairs, samples, and raster publication.
The control processes produced no capture files or batches.
The driver PASS result applies to these functional checks, not to a frame-time acceptance limit.

The last phase of capture-2 had a 49.905396 ms mean, 1010.877 ms p99, and 1011.840 ms maximum.
This occurred after disconnection.
Its cause is unknown, and the result remains in the data.
There is no automatic exclusion of this sample.

The middle phase was slower than the initial phase in both modes.
Thus the earlier comparison against only the initial phase included workload-age effects.
These short samples do not prove physical presentation timing, input latency, stable memory, or sustained performance.
The overhead check remains open because capture runs had larger frame-time peaks.

## CLI record analysis

Run `bun docs/benchmarks/native-paint-shell/profiling-comparison-1788655121419/analyze.ts` from the repository root.
The script uses the public `records` command and rejects incomplete query results or RPC pairs.
It writes `host-gap-analysis.json` with the eight largest host presentation intervals per capture and their overlapping RPC intervals.
This is an explicit eight-interval selection, not a report of every interval.
All 140 RPC pairs per capture enter the analysis.

The largest paint-workload intervals were approximately 40.741, 39.890, and 37.393 ms in the three captures.
Each interval overlapped worker 19, method 0, and eight method 1 replies.
This overlap does not identify the cause or distinguish worker computation from response scheduling.
RPC intervals measure the complete round trip, not worker CPU time.
Host presentation records measure host calls, not physical display timing.

Capture-2 also contains a 1059.472 ms host interval with no overlapping application RPC.
Thus its cadence interruption started before capture ended, not only in the last phase.
The cause remains unknown. The current records do not identify window visibility, display suspension, or garbage collection.
