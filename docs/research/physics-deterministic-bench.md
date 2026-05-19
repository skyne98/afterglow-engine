# Physics: Avian Deterministic Performance Characterization

**Date:** 2026-05-19
**Crate:** `prototype-physics-bench` at `crates/prototypes/prototype-physics-bench`

## Configuration

| Setting | Value |
|---|---|
| Engine | Avian 3D 0.6.1 / Bevy 0.18.1 |
| Features | `3d`, `f32`, `parry-f32`, `enhanced-determinism` |
| Threading | Single-threaded (`parallel` feature **off**) |
| Compiler flags | `-C target-cpu=native` |
| Timestep | 1/60s fixed |
| Workload | Dynamic boxes + `SphericalJoint` chains (10 links each) |
| Steps per measurement | 100 |

## Results

| Bodies | Step time | Hz | vs 144 Hz target |
|---|---|---|---|
| 10k | 0.18 ms | 5,620 | 39× headroom |
| 50k | 0.91 ms | 1,096 | 7.6× |
| 100k | 1.79 ms | 557 | 3.9× |
| 200k | 3.58 ms | 279 | 1.9× |
| 300k | 5.36 ms | 187 | 1.3× |
| **380k** | **6.82 ms** | **147** | **≈ at limit** |
| **390k** | **6.96 ms** | **144** | **144 Hz floor** |
| 400k | 7.14 ms | 140 | below |
| 500k | 8.94 ms | 112 | below |

## Takeaways

- ~390k ragdoll-like bodies saturate a single thread at 144 Hz with `enhanced-determinism`.
- Scaling is near-linear in this range: ~1.8 μs per body per step.
- The bottleneck is Parry's collision detection and constraint solving, forced to scalar mode by `enhanced-determinism` (SIMD is mutually exclusive with it).
- Without `enhanced-determinism` + `simd` feature enabled, performance would be higher (Parry SIMD via `wide` crate: 4-wide f32 SIMD).
- `target-cpu=native` had negligible impact (~1%) because glam already auto-detects SSE2+ and Parry is scalar anyway.

## Determinism Verification

The benchmark self-verifies determinism by running the full simulation twice and comparing a hash of all final `Transform` positions and rotations (raw IEEE-754 bit patterns via `f32::to_bits()`). The hash uses `DefaultHasher` (SipHash-2-4).

**Result:** Bit-identical across repeated runs at 10k and 100k bodies, 500 steps each. Confirmed.

## Reproduce

```sh
RUSTFLAGS="-C target-cpu=native" cargo run --release --package prototype-physics-bench <body_count> <steps>
```
