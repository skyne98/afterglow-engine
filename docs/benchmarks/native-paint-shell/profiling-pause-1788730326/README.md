# Capture pause repetition

TODO #1 resumed with the existing native paint workload and syscall tracer.
The run used the NVIDIA RTX 3090, driver 595.99.02, and the niri Wayland session.
`AFTERGLOW_HOST_TRACE=1` added host state output.
The driver closed only its own processes.

## Results

The capture retained 31,961 host records with no missing span beginnings.
Two spans extend beyond the capture boundary.
All requested host measurements were present.
Pixel, source, sequence, and loss checks passed.

| Phase | Mean rAF interval | p99 | Maximum |
| --- | ---: | ---: | ---: |
| Before capture | 6.925 ms | 10.171 ms | 10.482 ms |
| During capture | 7.207 ms | 19.320 ms | 34.473 ms |
| After capture | 7.119 ms | 19.259 ms | 23.190 ms |

The largest observed host presentation interval was 37.766 ms.
Its surface presentation call occupied 25.792 ms, and HUD composition occupied 6.812 ms.
The window had focus, no recorded suspension, and unknown occlusion state.
No window-state change occurred within that interval.

The longest syscall was a 27.679 ms poll on `anon_inode:sync_file`.
A second fence poll took 25.259 ms.
The syscall and capture clocks still have no explicit mapping.
These calls cannot be paired with specific captured intervals from their durations alone.

## Limits

No one-second pause occurred in this run.
The repeated fence waits do not identify the GPU workload that controls them.
Tracing changes timing, so this run is not an overhead acceptance check.
The earlier one-second pauses remain unexplained.

## Reproduction

From the repository root, use an unused output directory:

```sh
nix-shell shell.nix --run 'AFTERGLOW_HOST_TRACE=1 bun scripts/test-native-paint-capture.ts <directory> --strace'
bun scripts/analyze-native-paint-capture.ts <capture.dgtl> > <directory>/analysis.json
bun docs/benchmarks/native-paint-shell/profiling-syscalls-1788709111/analyze.ts <directory>/syscalls.log > <directory>/syscall-summary.json
```
