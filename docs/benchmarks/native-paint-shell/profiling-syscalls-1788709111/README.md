# Completed main-thread syscall check

The corrected tracer completed the native paint capture and cleanup.
All capture checks passed: 31,881 host records, 140 complete RPC pairs,
40 paint samples, four paint publications, and no record loss.
The finite capture boundary contains two open host spans and no missing span beginnings.

## Findings

The longest captured presentation interval was 20.771 ms.
It contained 13.498 ms inside the surface presentation call.
The complete main-thread syscall trace contains a 13.230 ms `poll()` on file descriptor 66.
Other long polls use descriptors 63 and 66.
The longest `futex()` was 1.448 ms. The longest `ioctl()` was 13.645 ms during startup.

Descriptor numbers do not identify the resource or the owner.
The trace uses wall-clock timestamps, while the capture uses a monotonic clock.
There is no explicit clock mapping between these files, so the longest syscall
is not proven to be inside the longest captured presentation interval.
A further trace will include descriptor paths to identify the resource.

## Limits

Tracing changes timing. These values are not normal performance measurements.
No one-second interruption occurred in this run.
The cause of the earlier interruptions and sustained profiling overhead remain open.
The capture does not establish CPU time, GPU execution time, or physical presentation latency.

`analysis.json` comes from the public collector CLI through
`scripts/analyze-native-paint-capture.ts`.
`syscall-summary.json` lists counts and the longest 20 completed syscall lines.
It reports omitted lines explicitly. Reproduce that file with:

```sh
bun docs/benchmarks/native-paint-shell/profiling-syscalls-1788709111/analyze.ts
```

The driver uses `--interruptible=1`, `--kill-on-exit`, a 250,000-syscall limit,
and a 128 MiB shell file limit for this run.
Two fixture tests passed for tracer cleanup after success and failure paths.
No application presentation or permission policy changed.
