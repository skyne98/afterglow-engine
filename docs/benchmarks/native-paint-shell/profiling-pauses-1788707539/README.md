# Host pause measurements

Six fresh native processes used the same paint workload and phase delays.
The order was control, capture, capture, control, control, capture.
The release build and unit tests completed before these runs.
All runs passed pixel and capture-state checks.
Each capture retained 140 RPC pairs and 44 paint records without loss.

## Middle workload

Times are rAF intervals in milliseconds, not physical display timing.

| Run | Mean | p99 | Maximum | Maximum after disconnect/control delay |
|---|---:|---:|---:|---:|
| Control 1 | 7.061 | 18.530 | 19.474 | 24.998 |
| Capture 1 | 7.148 | 19.238 | 21.934 | 40.516 |
| Capture 2 | 7.090 | 18.653 | 20.169 | 1120.622 |
| Control 2 | 7.207 | 20.226 | 20.771 | 1020.296 |
| Control 3 | 7.207 | 18.236 | 23.897 | 1025.179 |
| Capture 3 | 7.226 | 21.170 | 27.889 | 26.168 |

The median mean is 7.148 ms with capture and 7.207 ms without capture.
This difference does not establish negative overhead.
The small sample and large later interruptions prevent an overhead acceptance claim.
One-second interruptions occurred in two processes without a collector.
Thus, capture is not necessary for this symptom.
The cause of those interruptions remains unknown.

## Location of measured pauses

The host now records runtime turns, frame work, presentation work, redraw requests,
and focus/occlusion/suspension state.
`scripts/analyze-native-paint-capture.ts` reads the captures through the public CLI.
Each `analysis.json` contains the eight longest presentation intervals and the total interval count.
It also reports incomplete span boundaries.

- Capture 1: a 30.964 ms interval contains 30.822 ms of presentation work and no application RPC.
- Capture 2: its longest 21.559 ms interval contains 17.646 ms of presentation work and 3.829 ms of runtime work.
- Capture 3: its longest 31.797 ms interval contains 27.451 ms of presentation work.

These are measured wall-time intervals, not CPU samples or GPU timestamps.
Nested work can overlap, so the columns must not be added.
RPC overlap does not identify the worker as the cause.
The presentation path includes HUD scene preparation, GPU submission, and surface presentation.
Separate measurements of these stages are the next check.

Each capture ends with two open host spans and no missing span beginnings.
The finite capture boundary cuts those spans. The recorder reports zero lost records.
Focus is known only where an event supplied it. Occlusion is unknown in these runs.
Suspension is false, with no change inside the listed intervals.
No inference about compositor activity follows from an unknown window state.

## Reproduction

Run each process inside `shell.nix`, after the release and Vite builds:

```sh
bun scripts/test-native-paint-capture.ts OUTPUT/control-1 --control
bun scripts/test-native-paint-capture.ts OUTPUT/capture-1
bun scripts/analyze-native-paint-capture.ts OUTPUT/capture-1/CAPTURE.dgtl
```

The capture files retain the measurements available at this run.
Later versions of the driver can demand additional descriptors and cannot validate these old files unchanged.
