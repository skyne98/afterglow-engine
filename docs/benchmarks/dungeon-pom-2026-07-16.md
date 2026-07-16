# Dungeon corrected POM + self-shadow — Radeon 680M — 2026-07-16

## Configuration

- CEF 149, Three.js r185 WebGPU, Vulkan/RADV on Radeon 680M (`amd` / `rdna-2`)
- 1440×900 logical, DPR 2, 2880×1800 physical
- Official resident ambientCG 1K, 16-bit displacement
- Geometric TBN; VT `NormalGL` sampled with `(1,-1)` orientation correction
- 8–32 view-adaptive POM layers, scale 0.05, ratio-2 offset cap
- 8-step point-light self-shadow, bias 0.01, strength 0.82
- No radial per-fragment fade
- VT feedback every 8 frames
- Post-warm-up rAF timestamp intervals; pages settled unless marked moving

## Results

| Scenario | Frames | FPS | p50 | p99 | max | Below 55 FPS |
|---|---:|---:|---:|---:|---:|---:|
| Fixed close angled wall, corrected POM | 600 | 59.87 | 16.675 ms | 16.680 ms | 33.355 ms | 1/600 |
| Same pose, prewarmed base | 600 | 59.77 | 16.675 ms | 16.680 ms | 33.350 ms | 2/600 |
| Moving along wall with active streaming | 300 | 59.97 | 16.675 ms | 16.680 ms | 16.680 ms | 0/300 |

The isolated fixed-view miss is not POM saturation: the base missed two vsyncs
under the same conditions. Ten explicit WebGPU timestamp samples measured main
render work at **1.02–1.09 ms**, feedback at **0.015–0.017 ms**, total timestamp
work at **5.49–7.25 ms**, and authored frame CPU at **0.59–1.16 ms**.

## Measured dip investigation

This was measured rather than inferred:

1. With the old 4-frame VT feedback cadence, POM measured 3/300 below 55 FPS.
2. With feedback disabled after residency, POM and base each measured 1/600.
3. Feedback cadence was changed to 8 frames.
4. Corrected POM then measured 1/600 fixed and 0/300 while moving.
5. Timestamp tracking had been left enabled continuously even though queries
   were not resolved, eventually producing Three's “maximum queries exceeded”
   warning. It is now disabled by default and enabled only around explicit
   diagnostics.

## Correctness gates

- Three independent launches × forward/reverse/corner viewpoints passed.
- Nine PBR VT channels loaded; every scenario settled with zero pending pages.
- Seven pipelines prewarmed; zero post-seal pipeline violations.
- Zero WebGPU errors or device loss.
- Generated WGSL gate proved exactly one geometric-TBN POM march before three
  linked displaced VT samples.
- Matched-pose compositor captures had virtually identical mean luminance:
  base `0.311836`, POM `0.311914` (0.025% difference), eliminating the previous
  close-range darkening while retaining pixel-level displacement differences.

## Superseded result

The earlier 59.97 FPS integration used AO pseudo-height, a normal/POM dependency
cycle, no light self-shadow, and a visible radial fade. It is invalid as a
correctness result and must not be cited.
