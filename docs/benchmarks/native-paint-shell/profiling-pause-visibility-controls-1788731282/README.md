# One-second visibility interruptions

## Result

The inactive-workspace check reproduces a one-second interruption with no collector connected.
The same check with capture and syscall tracing also reproduces the interruption.
Capture is not necessary for this symptom.

| Run | Middle workload mean | p99 | Maximum |
|---|---:|---:|---:|
| No collector | 26.075 ms | 995.906 ms | 997.555 ms |
| Capture with syscall tracing | 26.021 ms | 998.548 ms | 1000.058 ms |

The no-collector run has `connected: false`, `recording: false`, zero batches, and zero bytes.
Both runs passed paint checks with 954 colored pixels in each workload.
Neither run reported a capture failure.
The traced capture has no lost records.
After the window returned to its original workspace, the later workload maxima were 49.130 and 25.564 ms.

## Location of the interruption

The traced capture contains these longest host intervals:

| Host interval | Surface presentation | HUD composition | Runtime processing |
|---:|---:|---:|---:|
| 1000.178 ms | 999.239 ms | 0.056 ms | 0.298 ms |
| 1000.010 ms | 999.398 ms | 0.050 ms | 0.183 ms |
| 998.353 ms | 997.662 ms | 0.063 ms | 0.224 ms |
| 994.469 ms | 993.773 ms | 0.053 ms | 0.271 ms |

These four intervals have no overlapping application RPC span.
The recorded state is unfocused, with unknown occlusion and no suspension.
The analysis contains 22,313 host records, no missing span starts, and two open span ends.
Nested intervals can overlap. Do not add their durations.

The longest syscall is different from the earlier short GPU fence waits.
It is a `ppoll` on a Unix stream socket, not `anon_inode:sync_file`.
The longest four socket waits are 998.113, 996.758, 994.803, and 992.165 ms.
The longest `poll` is only 18.580 ms.

The first long socket wait is at `trace/syscalls.log:9350`:

```text
1788731318.730566 ppoll([{fd=3<UNIX-STREAM:[123963898->123956110]>, events=POLLIN}], 1, NULL, NULL, 8) = 1 ([{fd=3, revents=POLLIN}]) <0.998113>
```

The [subsequent stack check](../profiling-pause-visibility-stacks-1788731483/README.md)
identifies `wl_display_dispatch_queue` inside NVIDIA presentation as the caller.
This identifies a Wayland event wait on the main thread, not a diagnostics socket wait.

## Method

The existing release binaries and generated paint page are unchanged.
Run the checks in sequence inside `shell.nix`:

```sh
bun docs/benchmarks/native-paint-shell/pause-visibility.ts "$root/control" --control
bun docs/benchmarks/native-paint-shell/pause-visibility.ts "$root/trace" --strace
```

The driver moves only its verified child window to an empty inactive workspace.
It does not change the active workspace or move user windows.
It restores the diagnostic window after 4.5 seconds.
Each `visibility.json` records the actions and any error.
The original paint driver supplies bounded execution, capture validation, and process cleanup.

Each directory retains `result.json`, `host.log`, and `visibility.json`.
The traced directory also retains the DGTL file, `analysis.json`, `syscalls.log`, and `syscall-summary.json`.

## Limits

The earlier one-second interruptions have no retained visibility history sufficient to prove the same cause for every occurrence.
This experiment identifies a reproducible trigger, not every possible interruption.
The syscall clock, observer clock, and DGTL clock have no explicit common mapping.
The experiment does not establish capture overhead, foreground presentation stability, or physical input latency.
No runtime presentation policy changed.
