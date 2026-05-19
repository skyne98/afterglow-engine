# Rollback Buffer Design: RunDelta Chain

**Date:** 2026-05-19
**Relevant prototypes:**
- `prototype-physics-bench` — Avian determinism benchmark
- `prototype-physics-serialize` — PhysicsSnapshot format + round-trip
- `prototype-delta-encoding` — RunDelta byte-level diffing

## Architecture

A fixed-size ring buffer of 240 frames (2 seconds at 120 Hz) stores a chain of:
- 1 full `PhysicsSnapshot` at a known anchor frame (every 240 frames)
- 239 `RunDelta` blobs for the intervening frames

```
Frame 0:     [Full snapshot bytes]       ← anchor
Frame 1:     [RunDelta: offset+data]     ← diff against frame 0
Frame 2:     [RunDelta: offset+data]     ← diff against frame 1
...
Frame 239:   [RunDelta: offset+data]     ← diff against frame 238
Frame 240:   [Full snapshot bytes]       ← new anchor (overwrites frame 0)
```

## Memory

| Entity count | Full snapshot | Per-frame delta (typical) | 240-frame ring |
|---|---|---|---|
| 10k | 550 KB | ~71 bytes | ~567 KB |
| 100k | 5.5 MB | ~720 bytes | ~5.7 MB |
| 390k (saturation) | ~21 MB | ~2.8 KB | ~22 MB |

At 10k bodies the entire ring fits in L2/L3 cache.

## Rollback Latency

To restore frame `T`, find the nearest preceding full snapshot, clone its bytes, then chain-apply deltas forward:

```
Clone full snapshot bytes:   ~1 μs  (Vec::clone)
Apply N RunDeltas:           N × ~0.2 μs  (sequential memcpy)
Deserialize to struct:       ~160 μs  (postcard)

Worst case (T=239, N=239):  ~48 μs apply + 160 μs deser ≈ 210 μs before resim
Typical (near anchor):       ~2 μs apply + 160 μs deser ≈ 162 μs
```

## Bandwidth

RunDelta is 800× smaller than full serde for sparse changes (1 body changed out of 10k: 10 bytes vs 550 KB). For frequent snapshots sent over network, this matters more than the ~2× CPU overhead.

## Pipeline Costs (10k bodies, 10 changed)

| Step | Time (μs) |
|---|---|
| Encode: ser(old) + ser(new) + diff | 330 |
| Decode: apply + deser | 180 |
| **Total delta round-trip** | **510** |
| Baseline: ser(new) + deser | 230 |

Delta costs ~2.2× baseline CPU but produces 800× less data. The bandwidth savings dominate end-to-end latency for any real-world network.

## Determinism

The full pipeline has been verified at scale:
- RunDelta: 5 unit tests pass (identical, field change, multiple changes, added body, raw bytes)
- PhysicsSnapshot round-trip: 10k bodies + 1.8k joints, 60 post-restore steps, bit-identical
- Avian determinism: bit-identical across independent runs with `enhanced-determinism` + single-threaded

## Usage in PVP Rollback

1. Each tick: serialize `PhysicsSnapshot` → `postcard::to_allocvec` → diff against cached previous bytes → store `RunDelta` in ring buffer
2. On rollback: clone anchor bytes → chain-apply deltas → `postcard::from_bytes` → write to Avian ECS components
3. Resimulate deterministic physics from that point using buffered player inputs
4. Works for any entity count that fits the per-tick budget (~390k at 144 Hz, ~8k at 540 Hz for fast-forward)
