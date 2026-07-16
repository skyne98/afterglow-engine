# Dungeon low-core POM — Radeon 680M — 2026-07-16

## Configuration

- CEF 149, Three.js r185 WebGPU, Vulkan/RADV on Radeon 680M
- 1440×900 logical, DPR 2, 2880×1800 physical
- Dungeon pose `(-5.5, -5.5)`, yaw `0.65` rad, pitch `-0.05` (close angled Rock064 wall)
- 8–32 adaptive layers, height scale 0.012, ratio-2 offset cap,
  fade/cutoff 2.1125–3.25 m
- Resident 1K ambientCG AO physical height (`surfaceDepth = 1-height`);
  8K albedo, normal, roughness, and AO remain virtual and share one marched UV
- 300 consecutive post-warm-up rAF intervals per variant

## Results

| Variant | FPS | p50 | p99 | max | Below 55 FPS |
|---|---:|---:|---:|---:|---:|
| Low-core POM | 59.97 | 16.675 ms | 16.680 ms | 16.680 ms | 0/300 |
| Prewarmed base | 59.97 | 16.675 ms | 16.680 ms | 16.680 ms | 0/300 |

Both corrected variants remained exactly vsync-locked throughout the sample.

## Correctness gates

- Hardware WebGPU adapter reported `amd` / `rdna-2`; no fallback.
- Three independent launches × forward/reverse/corner viewpoints passed.
- Zero WebGPU errors/device loss.
- Seven pipelines prewarmed; zero post-seal render/compute pipeline violations.
- Base CDP screenshot retained the original scanned VT texture.
- POM screenshot retained real texture pixels with no flat fallback/page bands.
- Corrected base and POM captures differed across 4,877,310 pixels (94.08% of
  the frame), proving the toggle changes rendered pixels rather than HUD state
  alone.
- The temporary direct-VT-height design was rejected because non-uniform march
  residency produced page-boundary artifacts. The accepted design uses a small
  resident matching height field and coarser-page/tail fallback only for the
  final displaced PBR samples.
- Twenty-six predictable-shape tests cover planes, analytic ramps, steps,
  ridges, a circular island, direction symmetry, fade/layer bounds, physical
  height clamping, and grazing offset limits. They caught the original
  height/depth inversion before this rerun.
