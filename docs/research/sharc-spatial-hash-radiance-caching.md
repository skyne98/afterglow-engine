# SHaRC (Spatial Hash Radiance Caching) — Deep Dive

Source: [github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache)
Files: `include/HashGridCommon.h`, `include/HashGridTypes.h`, `include/SharcCommon.h`, `include/SharcTypes.h`
Version: v1.7.2

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

Bob Jenkins' 32-bit integer hash, XOR'd across 64-bit key halves
([HashGridCommon.h:90-97](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridCommon.h#L90-L97)):

```
key = pack(quantizeX(17b), quantizeY(17b), quantizeZ(17b), lodLevel(9b), normalOctant(3b), flags(1b))
```

Key composition ([HashGridCommon.h:51-68](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridCommon.h#L51-L68)):
- 64-bit mode: 17 bits per position axis, 9 bits LOD level, 3 bits normal octant
- 32-bit compact mode: 8 bits per axis, 5 bits LOD, 3 bits normal
- Key assembly: `HashGrid_ComputeSpatialHash()` ([HashGridCommon.h:159-168](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridCommon.h#L159-L168))

Hash:
```
hash = Jenkins32(key.low32) ^ Jenkins32(key.high32)     // HashGridCommon.h:90-97, 102-103
slot = min(hash % capacity, capacity - BUCKET_SIZE)       // HashGridCommon.h:130-132
```

**Normal bits** (3-bit octant, [HashGridCommon.h:172-174](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridCommon.h#L172-L174)):
```
bit0 = normal.x + NORMAL_BIAS >= 0 ? 0 : 1       // +X or -X
bit1 = normal.y + NORMAL_BIAS >= 0 ? 0 : 2       // +Y or -Y
bit2 = normal.z + NORMAL_BIAS >= 0 ? 0 : 4       // +Z or -Z
```

This means surfaces at the same position but with different facing directions map
to **different slots** — SHaRC's main defense against light leaking, replacing
DDGI's Chebyshev distance test.

## Hash Table Layout

```
hashEntries[]:  8B each  → 64-bit packed key (or 0 = empty)
accumBuffer[]:  16B each → accumulated radiance (u32×3) + sample count (f32)
resolveBuffer[]: 16B each → resolved radiance (fp16×3) + frame metadata
```
**Total: 40 bytes per slot.** Default table size: 2²² = 4,194,304 slots → ~160 MB.

## Collision Handling: Fixed Bucket Linear Probing

Defined in [HashGridTypes.h:14-16](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridTypes.h#L14-L16):
```c
#define HASH_GRID_HASH_MAP_BUCKET_SIZE  16
#define HASH_GRID_INVALID_HASH_KEY      0
#define HASH_GRID_INVALID_CACHE_INDEX   0xFFFFFFFF
```

Each hash maps to a **base slot** within a 16-entry bucket. Insertion uses
`InterlockedCompareExchange` ([HashGridCommon.h:215-221](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridCommon.h#L215-L221)):
```
baseSlot = min(slot, capacity - 16)
for offset in 0..15:
    prev = AtomicCAS(hashEntries[baseSlot + offset], EMPTY, newKey)
    if prev == EMPTY or prev == hashKey:
        return slot claimed or already ours       // line 256-258
// All 16 slots occupied → insert silently fails
```

No cuckoo, hopscotch, or Robin Hood — just a simple linear probe up to 16 slots.

## Probe Generation: Sparse Path Tracing

Each frame, SHaRC traces paths from **~4% of pixels** (recommended: 1 per 5×5 block).
Every bounce hit point becomes a probe candidate. Paths are **back-propagated**
([SharcCommon.h:346](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L346)):

The back-propagation depth is controlled by `SHARC_PROPAGATION_DEPTH`
([SharcCommon.h:87-91](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L87-L91)):
- With resampling: 2 (default)
- Without resampling: 4

When a path reaches an emitter, radiance is propagated backward through the circular
buffer of `cacheIndices[PROPAGATION_DEPTH]` and `sampleWeights[PROPAGATION_DEPTH]`
([SharcCommon.h:171-172](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L171-L172)).

## Temporal Accumulation

Two-buffer approach ([SharcResolveEntry, line 554](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L554)):
- **accumBuffer**: per-frame, cleared each frame, written atomically during UPDATE
- **resolveBuffer**: cross-frame, exponential moving average

Blending ([SharcCommon.h:666-667](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L666-L667)):
```
accumulatedRadiance = (prevSamples × prevRadiance + newSamples × newRadiance)
                      / (prevSamples + newSamples)
```

History clamped to `accumulationFrameNum` ([SharcCommon.h:656-663](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L656-L663)):
```c
if (accumulatedFrameNum > accumulationFrameNum) {
    float normalizationScale = float(accumulationFrameNum) / float(accumulatedFrameNum);
    accumulatedFrameNum = accumulationFrameNum;
    sampleNumPrev *= normalizationScale;   // clamp history
}
```

### Fade Acceleration ([SharcCommon.h:670-685](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L670-L685))

Enabled via `SHARC_ENABLE_FADE_ACCELERATION` ([SharcCommon.h:99-100](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L99-L100)):
```c
// Track luminance direction in a 32-frame bitmask
float lumaCur = SharcLuma(accumulatedRadiance);
float lumaPrev = SharcLuma(accumulatedRadiancePrev);
bool fading = lumaCur < lumaPrev;
sharcVoxelData.sampleDataExt = (sharcVoxelData.sampleDataExt & ~bit) | (fading ? bit : 0u);
uint fadingFrameNum = countbits(sharcVoxelData.sampleDataExt);
if (fadingFrameNum == 32)
    sampleNumPrev = sampleNum;  // Reset history — force fast convergence
```

### Adjacent Level Blending on Camera Movement ([SharcCommon.h:688-718](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L688-L718))

Enabled via `SHARC_BLEND_ADJACENT_LEVELS` ([SharcCommon.h:95-96](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L95-L96)):
```c
// Only in first 2 frames after camera movement
if (dot(cameraOffset, cameraOffset) > 1e-6f && accumulatedFrameNum <= 2) {
    adjacentKey = SharcGetAdjacentLevelHashKey(hashGridKey, params, prevCameraPos);
    if (Find(adjacentKey, &adjacentData)) {
        // Blend adjacent LOD data into current
        blendWeight = rcp(adjacentSamples + currentSamples);
        accumulatedRadiance = adjacentSamples * blendWeight * adjacentRadiance
                            + currentSamples * blendWeight * currentRadiance;
    }
}
```

## Eviction: Frame-Count Staleness

Eviction in `SharcResolveEntry()` ([SharcCommon.h:585-600](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L585-L600)):

```c
staleFrameNum = (sampleNum != 0) ? 0 : staleFrameNum + 1;   // line 587
if (staleFrameNum >= staleFrameNumMax) {
    // Zero out the entry — evicted
    hashEntries[entryIndex] = HASH_GRID_INVALID_HASH_KEY;
    accumulationBuffer[entryIndex] = zeroAccumulationData;
    resolvedBuffer[entryIndex] = zeroPackedData;
    return;
}
```

`staleFrameNumMax` clamped to [8, 1024] ([SharcCommon.h:107-108](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L107-L108)):
```c
#define SHARC_STALE_FRAME_NUM_MIN   8
#define SHARC_STALE_FRAME_NUM_MAX   1024
```

## Query (no interpolation between probes!)

`SharcGetCachedRadiance()` ([SharcCommon.h:463-520](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L463-L520)):

```c
key = ComputeSpatialHash(hitPos, normal)                    // HashGridCommon.h:159-168
slot = HashGridFindEntry(key)                                // HashGridCommon.h:267-280
if slot found and sampleCount > SHARC_SAMPLE_NUM_THRESHOLD:  // SharcCommon.h:470
    return resolveBuffer[slot].radiance  // single voxel, no blending
else:
    return CACHE_MISS  // path continues tracing
```

No interpolation between probes. Each voxel stores a single radiance value.
The only blending-like operation is **adjacent LOD blending** during resolve
(see above, first 2 frames after camera movement).

## Dynamic Objects

Handled naturally: each frame traces new paths from current camera view.
Moving objects create probes at their new positions. Old probes stop receiving
samples → `staleFrameNum` increments → evicted within `staleFrameNumMax` frames.

## Responsive Lighting

`SHARC_ENABLE_RESPONSIVE_LIGHTING` creates a paired entry with the MSB of the hash key
set ([SharcCommon.h:120, 302-317](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L120))
for fast-changing light sources. These entries use a separate shorter accumulation
window (`responsiveFrameNum`, [SharcCommon.h:204](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L204))
and probe range (`SHARC_RESPONSIVE_ENTRY_PROBE_RANGE = 16`).

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

| Parameter | Value | Source |
|---|---|---|
| Default table size | 2²² = 4,194,304 slots | configurable |
| Per-slot memory | 40 bytes (8 + 16 + 16) | `HashGridKey` + `SharcAccumulationData` + `SharcPackedData` |
| Total memory at 2²² | ~160 MB | |
| Occupancy (static camera) | 10-20% | typical |
| Coverage per frame | ~4% of pixels | recommended in integration guide |
| Radiance scale | ~1e3 | [SharcCommon.h:162](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L162) |
| Stale frame min/max | [8, 1024] | [SharcCommon.h:107-108](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L107-L108) |
| Propagation depth | 2 (resampling) / 4 (no resampling) | [SharcCommon.h:87-91](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L87-L91) |
| Bucket size | 16 | [HashGridTypes.h:14](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/HashGridTypes.h#L14) |
| Version | 1.7.2 | [SharcCommon.h:12-14](https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/include/SharcCommon.h#L12-L14) |

## Open Source

- **NVIDIA SHaRC library**: https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache
- **RTXGI v2.x**: https://github.com/NVIDIA-RTX/RTXGI
- No third-party open source implementations exist.

## References

- **SHaRC library**: https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache (shader-only)
- **RTXGI v2.x SDK**: https://github.com/NVIDIA-RTX/RTXGI (C++/HLSL integration)
- **Key files**:
  - `include/HashGridCommon.h` — Jenkins hash, key construction, insert/find, LOD, debug
  - `include/HashGridTypes.h` — constants: BUCKET_SIZE=16, INVALID_HASH_KEY=0
  - `include/SharcCommon.h` — SharcInit (line 279), SharcUpdateHit (line 346), SharcUpdateMiss (line 321), SharcGetCachedRadiance (line 463), SharcResolveEntry (line 554), fade acceleration (line 670), adjacent level blend (line 688), responsive lighting (line 302)
  - `include/SharcTypes.h` — `SharcAccumulationData`, `SharcPackedData` formats
- **Integration guide**: https://github.com/NVIDIAGameWorks/Spatial-Hash-Radiance-Cache/blob/main/docs/Integration.md
