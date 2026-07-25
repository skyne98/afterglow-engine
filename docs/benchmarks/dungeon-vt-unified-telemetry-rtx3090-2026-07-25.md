# Dungeon VT unified-telemetry profile — RTX 3090 — 2026-07-25

## Method

- Public-web Dungeon under Chromium 150 WebGPU/Vulkan on an NVIDIA RTX 3090
  (`nvidia` / `ampere` adapter), 144 Hz compositor.
- Fresh Chromium profile and empty OPFS cache.
- 1440×900 window; nine deterministic camera poses over 4.05 seconds.
- Capture began after Dungeon bootstrap and initial residency (399 pages).
- The collector remained armed until admitted page work and persistent-cache
  writes completed.
- Trace clock: browser monotonic nanoseconds, `1_000_000_000` ticks/second.
- Finite buffer: 65,536 records (2.5 MiB); 24,850 records written, zero dropped,
  zero unmatched spans.

Raw evidence:

- `dungeon-vt-unified-telemetry-rtx3090-2026-07-25.agtb` — lossless AGTB batch,
  SHA-256 `f6b8ecc3d8bdf9897b35d35104faf93617d67c3b8af04017d1b32905bc7558ef`.
- `dungeon-vt-unified-telemetry-rtx3090-2026-07-25.json` — aggregate profile.

Durations below are wall-clock async latency. Async totals overlap and must not
be summed as CPU time.

## Frame result

| Metric | Result |
|---|---:|
| Scenario frames | 582 |
| Mean frame interval | 6.962 ms |
| p95 / p99 | 6.955 / 6.955 ms |
| Maximum | 13.900 ms |
| Frames slower than 60 Hz | 0 |
| Loaded / failed pages | 973 / 0 |
| Uploaded pages | 960 |
| Final residency | 1,359 / 2,809 |

## VT pipeline

| Stage | Operations | Mean | p95 | p99 | Maximum |
|---|---:|---:|---:|---:|---:|
| Complete page load | 1,274 | 149.04 ms | 207.90 ms | 231.38 ms | 253.29 ms |
| Bulk batching wait | 1,274 | 56.93 ms | 100.10 ms | 100.16 ms | 100.90 ms |
| Bulk source dispatch | 53 | 3.02 ms | 4.69 ms | 9.01 ms | 9.94 ms |
| Texture queue wait | 1,231 | 81.80 ms | 149.49 ms | 167.77 ms | 187.12 ms |
| Texture transcode | 1,014 | 11.43 ms | 12.53 ms | 14.35 ms | 22.92 ms |
| Atlas/page-table upload | 960 | 0.028 ms | 0.055 ms | 0.110 ms | 0.150 ms |
| Cold cache lookup | 1,274 | 0.320 ms | 0.440 ms | 0.575 ms | 16.76 ms |

The 53 bulk reads transferred 22,947,464 bytes. Page-load status was 973
successful and 301 canceled/stale. The scheduler reported 1,126 stale
cancellations, 115 priority preemptions, and 247 rejected admissions while
settling every bounded queue to zero.

## Interpretation

The sampled bottleneck is latency policy and worker queueing, not I/O or GPU
publication. The quality tier intentionally contributes a ~100 ms bulk window,
and four transcode workers then add an 81.8 ms mean queue wait. Actual bulk I/O
is about 3.0 ms mean and BC7 transcode execution is 11.4 ms mean. Upload and
page-table publication are negligible at 0.028 ms mean.

The cold persistent cache admitted 481 writes and deterministically rejected
492 at its bounded queue/capacity boundary. Rejections did not fail page loads,
but they reduce the next-run cache hit rate. Cache-write wall latency is highly
bimodal because admitted OPFS work serializes while rejected writes complete
immediately. Cache policy and quality batching are the first tuning candidates;
atlas upload is not.

## Instrumentation corrections found by the profile

The first attempted capture exposed two telemetry defects before this accepted
run:

1. the TypeScript clock produced microsecond ticks while descriptors/export
   treated them as nanoseconds;
2. texture queue completion wrote queue duration into the descriptor's `bytes`
   field.

The accepted capture uses nanosecond ticks with a 1 GHz declared rate and keeps
queue duration in timestamps while preserving bytes in the descriptor field.
The Dungeon trace capacity was raised from 16,384 to 65,536 records so this
bounded profile completes without prefix overflow.
