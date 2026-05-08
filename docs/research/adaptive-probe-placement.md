# Adaptive Probe Placement for Global Illumination — Deep Dive

## The Uniform Grid Problem

Standard DDGI uses a uniform 3D grid. Problems:
- **Wasted probes** inside geometry or empty space
- **Uniform density** — complex corners get same count as empty rooms
- **Grid artifacts** — banding from fixed probe positions
- **Memory inefficiency** — 32×32×32 = 32,768 probes regardless of scene

## Techniques

### 1. Probe Relocation (RTXGI Classic)

Probes shift within their grid cell to avoid being inside geometry.

**How it works**: Trace a fixed subset of rays (e.g. 32). If >50% hit backfaces,
the probe is inside geometry. Move the probe opposite the average backface
normal direction, clamped to ±45% of cell size. Temporal lerp for smoothness.

**Limits**: 45% clamp means probes can't escape geometry larger than a cell.
No reduction in total probe count. Oscillation near thin geometry.

**Used by**: RTXGI SDK, The Callisto Protocol, Filament, Atlas Engine, LuxGI

### 2. Surfel-Based (Frostbite / EA SEED)

Probes are placed on **surface elements** (surfels) — small patches of geometry.

**How it works**: Sample the scene surface using ray tracing. Each surfel =
position + normal + radius + accumulated radiance. Surfels trace rays to gather
light, then splat results onto visible surfaces at shading time.

**Pros**: Zero wasted probes — only exist where there's geometry. Naturally
adaptive density. Normals improve hemisphere sampling. No "probe inside wall."

**Cons**: Dynamic geometry is hard (surfels move/regenerate). Surfel pool
management complexity. Ghosting. Query is more expensive than trilinear grid.

**Used by**: EA Frostbite (research), Doom Eternal (idTech), Lumen (UE5, partial)

### 3. Spatial Hash Radiance Caching (SHaRC)

Probes stored in a **spatial hash table** instead of a grid (RTXGI v2.x).

**How it works**: `hash = hash3(worldPos) % numSlots`. Hash table with
cuckoo/hopscotch hashing. Candidates generated each frame from camera ray
hit points, assigned to slots, aged/evicted with LRU.

**Pros**: Memory proportional to scene complexity (not grid). Naturally
adaptive. Scales to large scenes. Configurable probe count.

**Cons**: Hash collisions cause probe conflicts. No natural interpolation
structure. Temporal stability harder. Complex eviction policy.

**Used by**: RTXGI v2.x, "The Cavern" RTXGI 2.0 demo

### 4. Neural Radiance Caching (NRC)

Neural network replaces the probe grid entirely.

**How it works**: Small MLP (32-128 neurons, 2-4 layers) maps
(position + normal + view) → (irradiance + distance). Trained online
each frame with ray-traced samples. ~50-500 KB for weights vs ~11 MB for
a probe grid.

**Pros**: Very smooth interpolation, no grid artifacts, tiny memory.

**Cons**: Training cost (~0.5-2ms GPU per frame). Catastrophic forgetting on
lighting changes. Scene change adaptation takes time. Needs careful tuning.

**Used by**: RTXGI v2.x NRC (research/experimental), Müller et al. SIGGRAPH 2021

### 5. Cascaded / Adaptive Grids

Multiple concentric grids at different resolutions around the camera.

**How it works**: Inner grid (dense, small spacing, e.g. 0.5m). Outer grids
(progressively sparser, e.g. 2m, 8m, 32m). Like cascaded shadow maps.

**Geometric adaptivity**: Subdivide cells where surface complexity, normal
variance, or occlusion count is high.

**Pros**: Better quality-per-probe ratio. Naturally handles indoor+outdoor.

**Cons**: Complex data structure. Harder interpolation. Temporal stability
when adaptivity changes.

**Used by**: Enlighten, The Witness, CryEngine SVOGI, Lumen

### 6. Octree / BVH-Based

Probes stored in leaf nodes of a spatial subdivision tree.

**How it works**: Subdivide based on surface complexity, visibility, or
lighting variance. Each leaf gets a probe. Two-level for dynamics.

**Pros**: Perfect adaptivity. Hierarchical LOD. Natural occlusion query.

**Cons**: Expensive dynamic updates. Irregular interpolation (no trilinear).
Memory overhead from tree nodes. Unpredictable probe count.

**Used by**: The Witness, Lumen (Feedback octree), Dreams (Media Molecule)

### 7. Importance-Driven

Probes placed where lighting importance is highest.

**Metrics**: Light source proximity, contrast/variance, geometric edges,
camera view importance.

**How it works**: Compute importance per probe, sort, keep top K, or
subdivide cells above threshold. Adapt over time with temporal smoothing.

**Pros**: Optimal use of probe budget. Focus quality where it matters.

**Cons**: Importance computation overhead. Temporal instability. Scene-
dependent metric design. Hard to guarantee quality everywhere.

**Used by**: Enlighten, Lumen, Bakery GPU Lightmapper

### 8. Infinite Scrolling Volumes (ISV)

Fixed-size grid slides with the camera, recycling probes.

**How it works**: Probes at trailing edge recycled to leading edge. Scroll
offset incremented as camera moves. New probes fade in over 8-16 frames.

**Pros**: Constant memory regardless of world size. Simple. Predictable.

**Cons**: Edge artifacts at leading edge. Requires forward camera motion.
Probes never fully converge if camera moves too fast.

**Used by**: RTXGI SDK, The Callisto Protocol, Filament (experimental), Atlas Engine

### 9. Portal-Based

Probes placed only in visible regions using cell-portal decomposition.

**How it works**: Partition scene into cells (rooms) connected by portals
(doorways). Place probes per cell. Update only visible cells each frame.
Portal probes capture radiance flow between rooms.

**Pros**: Dramatic reduction in active probes. No light leaking through walls.
Deterministic budget. Natural multi-bounce through portals.

**Cons**: Requires precomputed cell-portal graph. Not suitable for outdoor
environments. Hard to automate decomposition.

**Used by**: Zelda: Breath of the Wild, Source Engine (baked), Quake/idTech,
Enlighten, Activision's Call of Duty GI

### 10. Gradient-Based

Use lighting gradients to determine where more probes are needed.

**How it works**: Screen-space: render low-res GI, compute irradiance
gradient |dI/dx|, |dI/dy|, subdivide at high-gradient regions.
World-space: compute directional variance per probe, subdivide between
probes with large irradiance differences.

**Pros**: Directly measures quality. No scene pre-analysis. Works for
dynamic scenes. Computable on GPU.

**Cons**: Noisy gradient needs temporal filtering. Threshold tuning is
subjective. Screen-space only catches visible errors.

**Used by**: Lumen (partial), academic research

### 11. Cluster-Based

Geometry grouped into clusters, one probe per cluster.

**How it works**: k-means on surface positions (optionally including
normals). Cluster radius = probe influence. Hierarchical clustering for LOD.

**Pros**: Captures scene structure (rooms/corners get own probes). Direct
budget control. No grid artifacts.

**Cons**: Must recompute for dynamic scenes. k-means is iterative. Cluster
boundaries cause seams. Hard to bound worst-case time.

**Used by**: The Witness (manual), Dreams (automatic), academic research

### 12. Hardware Occlusion Query-Based

Probes placed at camera-visible surface locations.

**How it works**: Cast camera rays, record hit positions. Accumulate over
frames, cluster into probe positions. Keep probes with high "visibility score."

**Pros**: Zero waste for invisible areas. Naturally view-adaptive.
Probe budget proportional to screen coverage.

**Cons**: Occluded surfaces get no probes (missing bounce light). Temporal
lag when camera turns. Probe flicker.

**Used by**: Academic research only

### 13. Edge-Aware

Probes placed near geometric edges and corners.

**How it works**: Detect edges via G-buffer (normal/depth discontinuity) or
ray hit distance variance. Place probes at geometric edge intersections.
Subdivide probe-probe edges that cross geometric edges.

**Pros**: Places probes exactly where lighting errors are worst. Greatly
reduces light leaking compared to uniform grid at same count.

**Cons**: Edges can be numerous. Must distinguish important from
unimportant edges. Probes wobble on dynamic scenes.

**Used by**: Visionaries RTGI (Unity asset), academic research

## Comparative Summary

| Technique | Real-Time | Dynamic | Adaptivity | Complexity | Quality | Shipped |
|---|---|---|---|---|---|---|
| Relocation (RTXGI) | ✓ | Fair | Per-cell shift | Low | Moderate | Callisto, Filament |
| Surfel-based | ✓* | Hard | Surface-bound | High | High | Doom, Frostbite(exp) |
| SHaRC (hash) | ✓ | ✓ | Non-uniform | Medium | High | RTXGI 2.0 |
| Neural Radiance Cache | ✓* | Fair | Learned | High | Very High | RTXGI 2.0(exp) |
| Cascaded grids | ✓ | ✓ | View-distance | Medium | Moderate | Multiple |
| Octree/BVH | ✓* | Hard | Subdivision | High | High | Lumen, Dreams |
| Importance-driven | ✓ | ✓ | Metric | Medium | High | Enlighten, Bakery |
| Infinite Scrolling | ✓ | ✓ | Camera-sliding | Low | Moderate | RTXGI, Callisto |
| Portal-based | ✓ | Fair | Visibility | Medium-High | Very High | Quake, COD |
| Gradient-based | ✓ | ✓ | Error-driven | Medium | High | Lumen(partial) |
| Cluster-based | ✓* | Fair | Grouping | High | High | Witness, Dreams |
| Occlusion query | ✓ | ✓ | Visibility | Medium | Moderate | Academic |
| Edge-aware | ✓ | ✓ | Geometric edge | Medium | High | Visionaries RTGI |

## Recommendations for Bevy Engine

**Start with**: uniform grid + relocation (RTXGI classic) — simplest, well-
understood, most reference implementations exist.

**Add**: cascaded grids for view-dependent density.

**Consider**: SHaRC for memory efficiency at scale.

**Explore**: edge-aware placement — a few extra probes at detected edges
greatly reduces light leaking.

**Essential for open worlds**: Infinite Scrolling Volumes.

**Avoid initially**: full octree/BVH or neural approaches — significant
complexity for uncertain benefit.

## References

1. Majercik et al., "DDGI," JCGT 2019
2. Majercik et al., "Scaling Probe-Based RT DDGI," JCGT 2021
3. Müller et al., "Neural Radiance Caching," SIGGRAPH 2021
4. Delalandre et al., "Global Illumination Based on Surfels," SIGGRAPH Advances 2021
5. Wyman, "DDGI2," Digital Dragons 2020
6. Silvennoinen, "Large-Scale GI at Activision," SIGGRAPH Advances 2021
7. Wright, "Radiance Caching in Lumen," SIGGRAPH 2021
8. Tawara et al., "Importance-Driven Radiance Caching," EGSR 2005
9. Ritschel et al., "Clustered Radiance Caching," 2006
10. Krivanek et al., "Radiance Caching for Efficient GI," SIGGRAPH 2005
11. RTXGI SDK: https://github.com/NVIDIA-RTX/RTXGI
12. RTXGI-DDGI: https://github.com/NVIDIAGameWorks/RTXGI-DDGI
