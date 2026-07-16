# Lossless 16-bit displacement plan

**Date:** 2026-07-15
**Status:** implemented; NVIDIA runtime validation complete, Radeon 680M rerun pending

## Problem

Three.js `TextureLoader` decodes a 16-bit PNG through the browser image path.
The resulting default `RGBAFormat + UnsignedByteType` texture maps to WebGPU
`rgba8unorm`, so the Dungeon previously sampled only 8-bit displacement.

## Contract

1. Preserve every normalized u16 source level offline and at runtime.
2. Use a single GPU channel and bypass browser image decoding.
3. Fail closed rather than silently change precision, format, or renderer.
4. Complete conversion/upload before renderer sealing.

## Implemented path

- [x] `afterglow-pipeline height-r16 <source.png> <output.r16>`.
- [x] Versioned `AGR16LE` payload: width, height, then exact little-endian u16 samples.
- [x] Runtime parser with exact magic/version/dimension/length validation.
- [x] Bootstrap conversion of each normalized u16 to a distinct float32 value.
- [x] Single-channel `RedFormat + FloatType` / WebGPU `r32float` upload.
- [x] Require `float32-filterable`; assert the post-warm-up GPU format is exactly `r32float`.
- [x] Replace runtime PNGs with reproducible `.r16` source payloads.
- [x] Rust and TypeScript precision, corruption, feature, configuration, and shipped-asset tests.
- [x] NVIDIA/CEF hardware validation with no WebGPU errors after correction.
- [ ] Rerun visual correctness and timing on Radeon 680M while briefly unlocked.

## Why runtime is R32F rather than R16_UNORM

Hardware creation proved that WebGPU exposes `r16unorm` as
`unfilterable-float`. Three r185 also generates an incompatible filtering bind
layout for this custom WGSL path; forcing nearest leaves the explicit sampler
unbound. Patching vendored Three is brittle, and manual bilinear interpolation
would turn each POM height lookup into four texture reads.

Float32 has enough mantissa precision for all 65,536 normalized u16 levels to
remain distinct. `r32float` therefore preserves the 16-bit source exactly while
retaining one filtered shader sample. It requires `float32-filterable` and fails
closed when unavailable.

## Memory and performance

The maps contain 2,621,440 texels. Their committed R16 payload is 5,242,880
texel bytes; the resident R32F GPU payload is 10,485,760 bytes (10 MiB). This is
the same byte count as the former RGBA8 GPU upload, but now one channel carries
full source precision. Conversion and allocation happen only during bootstrap.
POM sample count is unchanged.

Historical Radeon measurements used runtime RGBA8 and must not be cited for the
new path. The target rerun must report hardware WebGPU, `float32-filterable`,
actual `r32float`, zero errors/device loss, visual captures, and frame timing.
