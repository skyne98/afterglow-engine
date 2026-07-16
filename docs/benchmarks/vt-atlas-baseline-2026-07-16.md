# VT atlas-state baseline — 2026-07-16

Hardware and display are the `fox-laptop` configuration recorded in
`AGENTS.md`: Ryzen 7 6800U, Radeon 680M/RADV, 1440×900 logical CEF window at
144 Hz. The desktop session was unlocked. The dungeon used `.big` v5, nine
physical VT channels, a 3,600-slot 8160² atlas, BC7 transcode, an eight-page
admission budget, four uploads per poll, a 0.25 ms scheduler budget, and a
0.35 ms upload budget.

## Method

`baseline-vt-atlas.sh` invokes deterministic diagnostic feedback through
`window.__afterglowVtDungeon.runAtlasScenario()`:

- **cold:** wait for pinned mip tails and startup feedback to settle;
- **half:** request unique mip-0 material groups until at least 50% occupancy;
- **full:** request a disjoint range until at least 99.5% occupancy, then drain;
- **churn:** at full occupancy request another disjoint range for 17 feedback
  epochs, making previous scheduler generations stale and forcing eviction.

The four scenarios ran sequentially in one CEF process, so cache and cumulative
counters intentionally carry forward. Frame intervals are rAF timestamps.
Heap values are Chromium's non-standard `performance.memory.usedJSHeapSize` and
are noisy across GC; a delta is not an allocation count. `PerformanceObserver`
recorded long tasks. Three.js WebGPU timestamp queries were enabled. The raw records include the
latest main-context, feedback-context, and aggregate render GPU durations;
query resolution occurs once per second outside the frame hot path.

## Results

| State | Duration | Resident | Evictions (cumulative) | Mean frame | Max frame | >17 ms | Heap delta | Long tasks | Failed / overflow / GPU errors |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Cold | 0.05 s | 108 / 3600 | 0 | 6.949 ms | 6.955 ms | 0 | +2.3 MiB | 0 | 0 / 0 / 0 |
| Half | 9.36 s | 1896 / 3600 | 0 | 6.950 ms | 6.955 ms | 0 | −2.1 MiB | 0 | 0 / 0 / 0 |
| Full | 9.33 s | 3600 / 3600 | 78 | 6.950 ms | 6.955 ms | 0 | +5.8 MiB | 0 | 0 / 0 / 0 |
| Churn | 4.92 s | 3600 / 3600 | 1014 | 6.970 ms | 20.850 ms | 1 | −3.9 MiB | 0 | 0 / 0 / 0 |

| State | Main GPU | Feedback GPU | Aggregate render GPU |
|---|---:|---:|---:|
| Cold | 0.018 ms | 0.007 ms | 0.128 ms |
| Half | 0.169 ms | 0.007 ms | 0.481 ms |
| Full | 0.149 ms | 0.018 ms | 0.465 ms |
| Churn | 0.142 ms | 0.018 ms | 0.458 ms |

All scenarios drained to zero pending, scheduled, and ready-upload work. No page
load failed and no scheduler queue overflowed. Full occupancy caused one upload
budget exhaustion. Churn missed one 144 Hz interval but retained a 20.85 ms
maximum under deliberate full-cache replacement; this exceeds the 16.67 ms
60 Hz release target and therefore remains an optimization/retest gate.

The high rejection counters are expected admission backpressure, not lost
requests: persistent scheduler generations retain visible work and retry under
fixed in-flight limits. Raw per-second samples and final counters are adjacent:

- `vt-atlas-cold-2026-07-16.log`
- `vt-atlas-half-2026-07-16.log`
- `vt-atlas-full-2026-07-16.log`
- `vt-atlas-churn-2026-07-16.log`
