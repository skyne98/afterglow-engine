# Hidden-window presentation check

## Result

Moving only the diagnostic window to an inactive workspace caused a long frame interruption.
The captured workload had a maximum rAF interval of 600.439 ms.
The longest host presentation interval was 597.410 ms.
Surface presentation occupied 596.066 ms of that interval.
HUD composition occupied 0.049 ms, and runtime processing occupied 0.899 ms.
The focus state changed from focused to unfocused during the interval.
Occlusion stayed unknown, and suspension stayed off.

The preceding host interval was 541.932 ms, with 540.867 ms in surface presentation.
RPC spans overlap these intervals, but the presentation spans account for almost all their duration.
Thus long RPC durations alone do not identify worker computation as the cause.

| Workload | Mean rAF interval | p99 | Maximum |
|---|---:|---:|---:|
| Before capture | 6.927 ms | 9.975 ms | 10.682 ms |
| During capture and workspace change | 12.446 ms | 63.783 ms | 600.439 ms |
| After capture | 7.554 ms | 28.584 ms | 50.564 ms |

Paint checks found 954 colored pixels in each workload.
The capture had no failures or lost records.
The analysis contains 29,193 host records, no missing span starts, and two open span ends.

## Method

```sh
nix-shell shell.nix --run \
  "bun docs/benchmarks/native-paint-shell/pause-visibility.ts $directory"
```

The script identifies the child window by its process ID.
It checks that the original workspace is active and that the destination workspace is empty and inactive.
At 2.5 seconds after the readiness message, it moves only that window without changing the active workspace.
After 4.5 seconds, it restores the original workspace location.
The script does not move or close user windows.
The captured `visibility.json` has no error and records both actions.

The existing paint driver supplies the workload, bounded capture, pixel checks, and process cleanup.
The shell uses the existing release binary without a presentation policy change.

## Limits

This run has an active collector.
The [subsequent control check](../profiling-pause-visibility-controls-1788731282/README.md) reproduced a 997.555 ms interruption without a collector.
The observer action timestamps and DGTL timestamps have no explicit clock mapping.
This run reproduces a half-second presentation interruption, not the exact earlier one-second interruption.
It does not identify the internal driver or compositor operation responsible for the wait.
These measurements are not overhead or sustained performance acceptance evidence.

`result.json`, `host.log`, `visibility.json`, `analysis.json`, and the DGTL file retain the measurements.
