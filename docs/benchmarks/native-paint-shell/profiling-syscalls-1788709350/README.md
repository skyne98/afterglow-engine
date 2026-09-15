# Synchronization fence waits

The native paint capture and tracer cleanup passed.
The capture retained 32,081 host records, 140 complete RPC pairs,
40 paint samples, and four paint publications without loss.
Two host spans are open at the finite capture boundary. No span beginnings are missing.

## Descriptor evidence

The trace uses `--decode-fds=path,socket`.
Its longest polls identify `anon_inode:sync_file`, a Linux synchronization fence:

| Trace line | Descriptor | Poll wall time |
|---|---|---:|
| 9905 | `66<anon_inode:sync_file>` | 27.729 ms |
| 9980 | `66<anon_inode:sync_file>` | 25.251 ms |
| 25930 | `63<anon_inode:sync_file>` | 13.119 ms |

These polls do not wait on an RPC socket.
Their return value indicates readable fence descriptors.
The trace does not identify which GPU submission or driver operation controls each fence.
Tracing and scheduler preemption can also increase syscall wall time.

The longest captured presentation interval was 36.376 ms.
It contained 25.581 ms in the surface presentation call and 6.421 ms in HUD composition.
The syscall trace and DGTL file have no explicit clock mapping.
Do not assume that the longest syscall corresponds to the longest captured interval.

## Status

The wait resource is now identified. The underlying GPU or driver cause remains unknown.
No one-second interruption occurred in this run.
Earlier interruptions occurred in control processes without a collector.
This work does not establish sustained profiling overhead or physical presentation latency.
No presentation mode, worker scheduling, or permissions changed.

`analysis.json` uses the public collector CLI.
To reproduce `syscall-summary.json` from the repository root:

```sh
bun docs/benchmarks/native-paint-shell/profiling-syscalls-1788709111/analyze.ts \
  docs/benchmarks/native-paint-shell/profiling-syscalls-1788709350/syscalls.log
```

The summary lists the longest 20 completed calls and reports omitted lines.
The raw trace has a 250,000-syscall limit and a 128 MiB shell file limit.
