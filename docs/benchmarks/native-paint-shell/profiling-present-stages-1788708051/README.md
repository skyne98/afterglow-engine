# Presentation stage measurements

Three fresh native paint processes used the RTX 3090 and the 143.975 Hz host cadence.
The release build and unit tests completed before capture.
All three runs passed pixel, source, sequence, and loss checks.
Each capture retained 140 complete RPC pairs and 44 paint records.

## Results

| Capture | Host records | Longest presentation interval | Surface present call within that interval | HUD composition within that interval |
|---|---:|---:|---:|---:|
| 1 | 31,948 | 15.070 ms | 14.896 ms | 0.035 ms |
| 2 | 31,929 | 16.984 ms | 16.865 ms | 0.023 ms |
| 3 | 32,025 | 18.358 ms | 18.236 ms | 0.022 ms |

The longest intervals in these runs occurred almost entirely inside `GPUCanvasContext::present()`.
No application RPC overlapped these three intervals.
That method calls `wgpu_core::Global::surface_present()` before it releases the current texture handle.
The measurement includes both operations. It does not separate GPU work, driver waits, compositor waits, or host preemption.
The next lower-level check needs driver or scheduling evidence, not another inference from RPC overlap.

The middle paint-workload rAF means were 6.975, 7.003, and 6.952 ms.
Their p99 values were 10.816, 10.413, and 11.299 ms.
These captures do not establish that more instrumentation improved performance.
They differ from the previous runs in instrumentation and external scheduling conditions.
No presentation mode, paint scheduling, or connection behavior changed.

All captures have zero lost records, no missing span beginnings, and two open spans at the finite capture boundary.
Focus is false in the largest interval of capture 1 and unknown in captures 2 and 3.
Occlusion is unknown. Suspension is false. There are no window-state changes inside these intervals.
Unknown state does not establish that a window is visible or hidden.

## Scope and remaining checks

These measurements locate a host wall-time cost. They are not CPU profiles or GPU timestamps.
They do not identify the cause of the earlier one-second interruptions.
Those interruptions also occurred in control processes without a collector.
Driver-level pause causes, sustained overhead, memory behavior, and physical latency remain open.

The generic CLI supplied every analyzed record. `analysis.json` files contain the
longest eight intervals, total interval count, overlapping spans, and incomplete boundaries.
To reproduce the analysis from the repository root:

```sh
bun scripts/analyze-native-paint-capture.ts PATH/TO/CAPTURE.dgtl
```
