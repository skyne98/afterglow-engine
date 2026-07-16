# Dungeon low-core POM — Radeon 680M — 2026-07-16

## Configuration

- CEF 149, Three.js r185 WebGPU, Vulkan/RADV on Radeon 680M
- 1440×900 logical, DPR 2, 2880×1800 physical
- Dungeon pose `(-5.5, -5.5)`, yaw `0.5` rad (close angled Rock064 wall)
- 8–32 adaptive layers, height scale 0.012, fade/cutoff 2.1125–3.25 m
- Resident 1K ambientCG AO height; 8K albedo remains virtual
- 300 consecutive post-warm-up rAF intervals per variant

## Results

| Variant | FPS | p50 | p99 | max | Below 55 FPS |
|---|---:|---:|---:|---:|---:|
| Low-core POM | 59.97 | 16.675 ms | 16.675 ms | 16.675 ms | 0/300 |
| Prewarmed base | 59.77 | 16.675 ms | 16.675 ms | 33.350 ms | 1/300 |

The isolated base miss was not repeated by POM and is scheduler/compositor
noise, not evidence that POM improves rendering cost.

## Correctness gates

- Hardware WebGPU only; no fallback.
- Zero WebGPU errors/device loss.
- Seven pipelines prewarmed; zero post-seal render/compute pipeline violations.
- Base CDP screenshot retained the original scanned VT texture.
- POM screenshot retained real texture pixels with no flat fallback/page bands.
- Base and POM captures differed across 4,045,000 pixels (78.0% of the frame),
  proving the toggle changes rendered pixels rather than HUD state alone.
- The temporary direct-VT-height design was rejected because non-uniform march
  residency produced page-boundary artifacts. The accepted design uses a small
  resident matching height field and coarser-page/tail fallback only for the
  final displaced albedo sample.
