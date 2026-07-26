# Dungeon VT no-cache profile — RTX 3090 — 2026-07-25

## Configuration

- Chromium 150 WebGPU/Vulkan, NVIDIA RTX 3090 (`nvidia` / `ampere`).
- 1440×900 window, 144 Hz compositor, fresh browser launch.
- No persistent derived-page cache or storage access.
- Four texture workers, twelve waiting transcodes, sixteen admitted pages,
  2 MiB pending bytes, 1/16 ms bulk deadlines, 55 ms feedback cadence.
- 4.05-second realistic traverse and nine-pose/450 ms hostile teleport.
- Capture remained armed until pending, scheduled, ready, bulk, and transcode
  work drained to zero.

Evidence:

| Scenario | AGTB SHA-256 |
|---|---|
| Traverse | `ed22214e862a5d5f4f219ef616ab7d5d73405f26ebee8c7c039a972a274f7cc1` |
| Teleport | `11ffbd690a0e21a3e492c8d8c382a49a5374a0ccf3f3c7c726ec83d2d472143b` |

Raw `.agtb` and aggregate `.json` files share this report's filename prefix and
scenario name. Async totals overlap and must not be summed as CPU time.

## Results

### Frame timing

| Scenario | Frames | Mean | p99 | Max | Slower than 60 Hz |
|---|---:|---:|---:|---:|---:|
| Traverse | 583 | 6.950 ms | 6.950 ms | 6.950 ms | 0 |
| Teleport | 582 | 6.962 ms | 6.955 ms | 13.895 ms | 0 |

Both runs ended with zero errors, failed loads, pending pages, scheduled
requests, ready uploads, active/queued transcodes, and bulk reads.

### Pipeline latency

| Stage | Traverse mean / p99 / max | Teleport mean / p99 / max |
|---|---:|---:|
| Scheduler wait | 1269.78 / 4461.75 / 4462.15 ms | 554.16 / 2077.54 / 2077.74 ms |
| Admitted page load | 31.21 / 47.95 / 59.10 ms | 42.02 / 58.40 / 61.72 ms |
| Bulk wait | 9.12 / 16.11 / 17.38 ms | 9.11 / 16.45 / 28.88 ms |
| Bulk dispatch | 2.41 / 3.68 / 4.92 ms | 2.62 / 4.27 / 6.87 ms |
| Transcode queue | 8.32 / 30.51 / 34.50 ms | 18.89 / 34.55 / 43.17 ms |
| Transcode execution | 11.56 / 13.71 / 16.27 ms | 11.58 / 14.55 / 24.78 ms |
| Atlas/page-table upload | 0.036 / 0.115 / 0.140 ms | 0.029 / 0.105 / 0.130 ms |

Long scheduler waits are low-priority requests retained without I/O and later
canceled stale; they do not occupy the sixteen-page admitted pipeline. Traverse
admitted 933 pages (907 successful, 26 canceled). Teleport admitted 1,076 pages
(1,031 successful, 45 canceled). The old teleport baseline canceled 301
admitted loads, so expensive post-admission cancellation fell 85.0%.

### Before/after hostile teleport

| Metric | Pre-removal baseline | No-cache result | Change |
|---|---:|---:|---:|
| Admitted page mean | 149.04 ms | 42.02 ms | −71.8% |
| Admitted page p99 | 231.38 ms | 58.40 ms | −74.8% |
| Bulk-wait mean | 56.93 ms | 9.11 ms | −84.0% |
| Bulk-wait p99 | 100.16 ms | 16.45 ms | −83.6% |
| Transcode-queue mean | 81.80 ms | 18.89 ms | −76.9% |
| Transcode-queue p99 | 167.77 ms | 34.55 ms | −79.4% |
| Transcode mean | 11.43 ms | 11.58 ms | +1.3% |
| Upload mean | 0.028 ms | 0.029 ms | effectively unchanged |
| Admitted cancellations | 301 | 45 | −85.0% |
| Bulk source bytes | 22.95 MiB | 19.97 MiB | −13.0% |
| Bulk requests | 53 | 156 | +194% |

The selected 16 ms policy meets every latency, frame, byte, failure, queue,
and trace-correctness target. It wrote 26,861 teleport records and 25,876
traverse records with zero drops and zero unmatched spans.

## Request-count exception

The hostile run used 156 bulk requests versus the old 53-request/100 ms/64-page
baseline: 2.94×, above the plan's provisional 2× request-count gate. Source
bytes fell 13%, and no frame or I/O latency regression resulted.

A temporary 24 ms diagnostic reduced requests to 124 (2.34×) but violated the
selected batching target (24.44 ms p99), raised page-load p99 to 111.15 ms, and
produced a 48.65 ms frame maximum. It was rejected and not committed. A 32 ms
variant was not run because its deadline necessarily violates the <=20 ms
batch-wait gate and moves farther from the latency objective.

The follow-up deterministic trace replay
(`dungeon-vt-trace-replay-rtx3090-2026-07-25.md`) reproduced all 156 requests.
Source sorting reduced modeled adjacent source runs 740 → 511 (−30.9%) but
left requests at 156. Mip-deficit priority and bounded cross-channel affinity
also left their modeled control at 156; those priority numbers are sensitivity
only because AGTB does not record resident fallback mip or every feedback
refresh. The 106-request target therefore requires a different buffering,
prefetch, or source-superpage policy—not a source-order or scheduler tie-break.

Keep the approved 16 ms policy unless the user explicitly prioritizes request
count over measured visible latency. The 2× request-count gate therefore remains
the sole unaccepted RTX criterion pending that decision.

## Remaining gates

- Repeat traverse/teleport and the required soaks on Radeon 680M.
- Run 30-minute traverse and 60-minute thrash plateau tests.
- Native-shell Dungeon validation remains unavailable until its launch gate is
  complete; public-web results are not native-worker evidence.
