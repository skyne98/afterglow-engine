# Dungeon GPU timestamp audit — 2026-07-21

## Verdict

The historical constrained-atlas **10.63 ms** value is invalid as a per-frame
measurement. It was produced through Three r185 timestamp plumbing while
Afterglow owned the animation loop but did not establish Three frame identity.
It happens to be close to the corrected corner-POM p99 (10.49 ms), but that is
coincidental and does not rehabilitate the original sample.

## Root causes

### All engine frames were Three frame zero

`WebGPUTimestampQueryPool._resolveQueries()` groups query pairs by the `fN`
suffix in each timestamp UID and returns the sum for the final grouped frame.
Three's own `Animation` updates `renderer.info.frame` from `NodeFrame.frameId`.
Afterglow instead drives `RendererHost` from `EngineRuntime`'s rAF loop and did
neither of the required external-loop operations:

- `renderer.info.reset()` once per logical frame;
- assigning a distinct `renderer.info.frame` before the frame's render passes.

Every query therefore ended in `f0`. A resolve summed however many render
contexts had accumulated since the prior resolve, making `gpuTotalMs` dependent
on diagnostic readback cadence rather than one presentation frame.

### `gpuMainMs` selected the output transform

Three r185 uses an internal `RGBA16F` framebuffer whenever tone mapping or color
conversion is active. It renders the scene there, then submits a fullscreen
output-color-transform pass to the null/canvas target.

`VirtualTextureFeedbackCoordinator.resolveGpuTimings()` called
`renderer._renderContexts.get(null)` and labeled that context `gpuMainMs`.
That is the output transform, measured around 1.07 ms in this audit—not the HDR
scene/material pass.

## Measured prototype

The measured prototype changed `RendererHost.render(frame)` to call
`renderer.info.reset()` and assign `renderer.info.frame = frame.frameId` before
`renderer.render()`. This preserved the engine-owned rAF loop while giving
Three's query pool one stable identity per engine frame. The same frame-ownership
rule is now the permanent runtime implementation.

Configuration:

- Ryzen 7 6800U / Radeon 680M, validated Mesa 25.3.4 RADV stack;
- 1440×900 logical / 2880×1800 physical CEF surface;
- constrained 2,809-slot atlas;
- independently resident albedo/normal/mask channels;
- queues drained and feedback disabled;
- 80 timestamp samples per canonical pose and material mode;
- no RGP/SQTT instrumentation;
- zero errors, queue overflow, or post-seal pipelines.

### Corrected scene plus output totals

| Pose | Non-POM mean | Non-POM p99 | POM mean | POM p99 |
|---|---:|---:|---:|---:|
| Forward | 4.190 ms | 5.539 ms | 6.559 ms | 8.909 ms |
| Reverse | 4.283 ms | 5.229 ms | 5.492 ms | 6.493 ms |
| Corner | 5.837 ms | 7.425 ms | 8.288 ms | 10.493 ms |

The null-target output pass remained approximately 1.07 ms in base and POM
runs. Subtracting it gives approximate HDR scene ranges of 3.1–4.8 ms non-POM
and 4.4–7.2 ms POM. Exact scene/output separation should become an explicit API
rather than relying on subtraction from Three's private context IDs.

A 300-frame rAF sample at the final forward/base pose was perfectly locked to
the active 60 Hz presentation cadence: 16.675 ms mean, 16.680 ms p99/max, and
zero misses under the audit threshold.

### Feedback-enabled check

At the forward pose with normal eight-frame feedback cadence, 80 corrected total
samples measured:

| Mode | Mean | p50 | p90 | p99/max |
|---|---:|---:|---:|---:|
| Non-POM | 4.303 ms | 4.406 ms | 5.243 ms | 6.001 ms |
| POM | 6.391 ms | 6.113 ms | 7.114 ms | 8.237 ms |

Feedback did not explain the historical 10.63 ms result.

## Permanent correction

`RendererHost` now establishes one Three frame ID and resets Three's per-frame
counters for each `EngineRuntime` frame before any visible or feedback render
pass. The clean-break diagnostics API exposes `gpuTimingValid`,
`resolvedFrameId`, `gpuSceneMs`, `gpuOutputMs`, `gpuFeedbackMs`, and
`gpuTotalMs`. The misleading `gpuMainMs` field was deleted.

Regression tests model multiple query frames, output/no-output paths, feedback
contexts, malformed keys, and unavailable timing. They fail if queries collapse
under one Three frame ID or if the output context is labeled scene work.

### Permanent implementation GPU gate

A fresh 680M run validated the committed field split at 2880×1800 with POM and
normal eight-frame feedback cadence. All 80/80 resolutions were valid with
strictly increasing frame IDs; 11 samples contained feedback work and every
sample satisfied `scene + output + feedback == total` exactly. Queues drained,
pipeline violations/errors were zero, and results were:

| Scope | Mean | p50 | p90 | p99/max |
|---|---:|---:|---:|---:|
| HDR scene | 5.211 ms | 5.240 ms | 5.955 ms | 7.102 ms |
| Output transform | 1.083 ms | 1.076 ms | 1.109 ms | 1.137 ms |
| Feedback (all frames) | 0.006 ms | 0 ms | 0.046 ms | 0.048 ms |
| Total | 6.301 ms | 6.322 ms | 7.029 ms | 8.238 ms |
