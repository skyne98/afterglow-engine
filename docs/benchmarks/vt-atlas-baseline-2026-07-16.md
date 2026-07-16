# VT atlas-state baseline — 2026-07-16

Hardware and display are the `fox-laptop` configuration recorded in
`AGENTS.md`: Ryzen 7 6800U, Radeon 680M/RADV, 1440×900 logical CEF window at
144 Hz. The desktop session was unlocked. The dungeon used `.big` v5, nine
physical VT channels, a 3,600-slot 8160² atlas, BC7 transcode, an eight-page
admission budget, four uploads per poll, a 0.25 ms scheduler budget, and a
0.35 ms upload budget.

## Method

`baseline-vt-atlas.sh` invokes deterministic diagnostic feedback through
`window.__afterglowDungeon.runAtlasScenario()`:

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

## Follow-up: two-page commit pacing (2026-07-16)

The original four-page / 0.35 ms commit quantum was tightened to **two pages /
0.20 ms** after an unlocked 1440×900 CEF repeat exposed 33–67 ms presentation
intervals during full-atlas admission. The repeat used the same Radeon 680M,
BC7, v5 nine-channel dungeon, eight-page admission budget, and 3,600-slot
atlas. This desktop session presented at a 60 Hz rAF cadence, so these are not
directly comparable to the 144 Hz baseline above; they are a targeted
full-cache pacing regression.

| State | Duration | Mean frame | Max frame | >17 ms | Resident | Evictions | Failed / overflow / GPU errors |
|---|---:|---:|---:|---:|---:|---:|---:|
| Half | 14.56 s | 16.694 ms | 33.350 ms | 1 | 1,896 / 3,600 | 0 | 0 / 0 / 0 |
| Full | 16.00 s | 16.692 ms | 33.350 ms | 1 | 3,598 / 3,600 | 80 | 0 / 0 / 0 |
| Churn | 8.12 s | 16.744 ms | 50.025 ms | 1 | 3,598 / 3,600 | 1,019 | 0 / 0 / 0 |

Relative to the earlier four-page run, full admission improved from 23 frames
above 17 ms with a 66.695 ms maximum to one such frame with a 33.350 ms maximum.
Churn retained one isolated 50.025 ms interval, so this is a substantial
reduction—not a strict zero-drop certification. GPU timings remained bounded:
main 1.00–1.05 ms, feedback 0.013–0.015 ms, aggregate render 4.01–5.90 ms.

## Follow-up: range/transcode latency and progressive priority

Interactive close-wall testing exposed a software pipeline bottleneck rather
than an SSD or GPU limit. The old route admitted 64 small ranges through the
page-side AssetLoader wasm executor and then serialized Basis conversion through
one texture worker. At the initial dungeon view it averaged **445.81 ms/page**:
**440.96 ms** was attributed to the AssetLoader range stage, while actual BC7
transcoding averaged only **1.51 ms**. Direct serving-layer control reads against
the same 727 MiB container took 0.81–1.64 ms for serial and four-way parallel
20,001-byte ranges.

The corrected path uses exact `fetch + Range`, four independent one-in-flight
texture workers on fox-laptop, and submits the complete missing mip chain into
exact coarse-to-fine priority lanes in one feedback update. After warm-up and a
close-wall transition, 48 new physical PBR pages completely settled in
**283.29 ms** with the tuner at four pages / 0.30 ms. Cumulative stage means were
**26.25 ms/page** admission-to-ready, **10.39 ms** range read, **13.39 ms**
transcode queue, **2.42 ms** transcode, and roughly 0.1 ms/upload. A larger new
view showed its first page at **79.32 ms**, first 12 coarse-priority pages at
**132.75 ms**, and all 306 physical pages at 1.60 s; the latter is intentionally
bounded by presentation-safe atlas commits. No page failed, no scheduler
overflowed, and no WebGPU error occurred.

A subsequent deterministic full-atlas run with repeated diagnostic feedback
filled all 3,600 slots in **38.99 s**. It averaged 16.711 ms/frame, had four
intervals above 17 ms and a 50.025 ms maximum, and finished with zero pending,
failed, or overflowed work. Final stage means were 8.48 ms admission-to-ready,
6.08 ms range read, 0.66 ms transcode queue, and 1.70 ms transcode across four
workers. The upload tuner reached four pages / 0.35 ms; one rejected probe
rolled back and later recovered under its hysteresis policy.

## Follow-up: persistent GPU-block cache

The generic `PersistentBlobCache` was composed into VT with a namespace covering
source ETag/size, selected format, transcoder/layout versions, and adapter
identity. CEF 149 rejects OPFS for the secure custom `afterglow://` origin, so
its automatic persistent IndexedDB backend was exercised.

The cold launch wrote 297 BC7 pages (5,493,312 bytes) with zero cache errors or
rejections. The following process launch restored the same view with **zero
`.big` page reads and zero Basis transcodes**. It recorded 365 cache hits, zero
misses/writes/errors, 8.21 ms average cache read, and 8.22 ms average
admission-to-ready latency. The cache therefore removes repeat-launch transcode
CPU work while retaining comparable page latency under IndexedDB. The exact
chunk fast path reduced the initial generic cursor implementation from 11.42 ms
to 8.21 ms/page. Three additional independent CEF launches × three viewpoints
passed GPU validation with the persistent cache enabled.
