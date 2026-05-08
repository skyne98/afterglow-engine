# SHaRC (Spatial Hash Radiance Caching) — Deep Dive

## What It Is

SHaRC is NVIDIA's replacement for the uniform DDGI probe grid. Instead of a fixed 3D array
of probes, it stores radiance in a **spatial hash table** keyed by world position + normal +
LOD level. From RTXGI v2.x, distributed as a shader-only library.

## Architecture

```
UPDATE pass (ray trace, ~4% of pixels):
  Trace paths → every bounce → hash hit position → atomic insert into hash table

RESOLVE pass (compute, O(table_size)):
  Blend new data with temporal history → evict stale entries

RENDER pass (full res ray trace):
  At each bounce → hash position → if found and valid → use cached radiance
  If not found → continue tracing → update cache
```

## Hash Function

Bob Jenkins' 32-bit integer hash, XOR'd across 64-bit key halves:

```
key = pack(quantizeX(17b), quantizeY(17b), quantizeZ(17b), lodLevel(9b), normalOctant(3b), flags(1b))
hash = Jenkins32(key.low32) ^ Jenkins32(key.high32)
slot = hash % capacity
```

**Normal bits** (3-bit octant classification) mean surfaces at the same position with
different facing directions map to **different slots** — this is SHaRC's main defense
against light leaking, replacing DDGI's Chebyshev distance test.

## Hash Table Layout

```
hashEntries[]:  8B each  → 64-bit packed key (or 0 = empty)
accumBuffer[]:  16B each → accumulated radiance (u32×3) + sample count (f32)
resolveBuffer[]: 16B each → resolved radiance (fp16×3) + frame metadata
```
**Total: 40 bytes per slot.** Default table size: 2²² = 4,194,304 slots → ~160 MB.

## Collision Handling: Fixed Bucket Linear Probing

Each hash maps to a **base slot** within a 16-entry bucket (linear probe window).
Insertion uses `InterlockedCompareExchange` (64-bit atomic CAS):

```
baseSlot = min(slot, capacity - 16)
for offset in 0..15:
    prev = AtomicCAS(hashEntries[baseSlot + offset], EMPTY, newKey)
    if prev == EMPTY or prev == newKey:
        return baseSlot + offset  // slot claimed or already ours
// All 16 slots occupied → insert silently fails (~10-20% occupancy is normal)
```

No cuckoo, hopscotch, or Robin Hood — just a simple linear probe up to 16 slots.

## Probe Generation: Sparse Path Tracing

Each frame, SHaRC traces paths from **~4% of pixels** (1 random pixel per 5×5 block).
Every bounce hit point becomes a probe candidate:

```
For each path bounce:
    hashKey = ComputeSpatialHash(hitPos, hitNormal, lodLevel)
    slot = HashGridInsertEntry(hashKey)         // atomic CAS insert
    SharcUpdateHit(slot, radiance, sampleWeight) // atomic accumulate
```

Paths are **back-propagated**: when a path reaches an emitter (sky or emissive surface),
radiance is propagated backward through all prior bounces in a circular buffer
(`SHARC_PROPAGATION_DEPTH`, default 2-4). Each path updates multiple cache entries.

## Temporal Accumulation

Two-buffer approach:
- **accumBuffer**: per-frame, cleared each frame, written atomically during UPDATE
- **resolveBuffer**: cross-frame, exponential moving average

```
accumulatedRadiance = (prevSamples × prevRadiance + newSamples × newRadiance)
                      / (prevSamples + newSamples)
```

History is clamped to a configurable max frame count (default ~64 frames).
When luminance is consistently decreasing (fading), fade acceleration (v1.6.5+)
detects this via a 32-frame bitmask and resets the history early.

## Eviction: Frame-Count Staleness

Each entry tracks `staleFrameNum`. If an entry receives **zero samples** in a frame,
`staleFrameNum++`. When it exceeds `staleFrameNumMax` (configurable 8-1024, default ~64),
the entry is zeroed and evicted.

There is **no distance-based eviction** and **no LRU** — purely frame-count staleness.
Probes naturally die when the camera looks away or objects move.

## Query (no interpolation!)

SHaRC does **NOT** interpolate between probes. On query:

```
key = ComputeSpatialHash(hitPos, normal, lodLevel)
slot = HashGridFindEntry(key)
if slot found and sampleCount > threshold:
    return resolveBuffer[slot].radiance  // single value, no blending
else:
    return CACHE_MISS  // path continues tracing
```

The only blending-like operation is **adjacent LOD level blending** during resolve
when the camera moves (first 2 frames after movement blend adjacent LODs together).

**Guard conditions for accepting cache:**
- Path segment must be longer than `voxelSize × √3` (prevents self-illumination)
- For glossy surfaces: ray cone footprint must exceed voxel size

## Dynamic Objects

Handled **naturally**: each frame traces new paths from current camera view.
Moving objects create probes at their new positions. Old probes at the former
position stop receiving samples → stale → evicted within `staleFrameNumMax` frames.
Fast movers cause a transient increase in hash table occupancy but stabilize quickly.

## Relation to DDGI

| Aspect | DDGI (grid) | SHaRC (hash) |
|---|---|---|
| Memory per probe | ~1.3 MB (SH + distance octahedral) | 40 bytes per slot |
| Grid setup | Manual volume placement | None (automatic) |
| Coverage | Within volume bounds | Anywhere rays hit |
| Level of detail | Fixed resolution | Logarithmic (LOD in hash key) |
| Normal-aware | No | Yes (3-bit normal in key) |
| Directional | Yes (irradiance SH) | No (single radiance) |
| Light leak defense | Chebyshev distance test | Normal-based slot separation |
| Interpolation | Trilinear (8 probes) | None (single voxel lookup) |
| Baking | Yes | No (purely dynamic) |
| Implementation complexity | Medium | High (atomic CAS, resolve passes, LOD) |

## Performance

| Parameter | Value |
|---|---|
| Default table size | 2²² = 4,194,304 slots |
| Memory | ~160 MB at 2²² |
| Occupancy (static) | 10-20% |
| Coverage per frame | ~4% of pixels |
| Cost | UPDATE ~4% of full path trace + hash inserts, RESOLVE cheap compute pass |

## Open Source

- **NVIDIA SHaRC library**: https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache
- **RTXGI v2.x**: https://github.com/NVIDIA-RTX/RTXGI
- No third-party open source implementations exist.

## References

- RTXGI SDK source: HashGridCommon.h, SharcCommon.h, SharcTypes.h, HashGridTypes.h
- Integration guide: https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/docs/Integration.md
- SharcGuide.md in RTXGI SDK Docs/
