# Low-end fallbacks for parallax / relief surface detail

**Date:** 2026-07-15  
**Status:** low-core tier integrated and hardware-validated in Dungeon
**Verdict:** A material-LOD chain ending in **normal mapping**, with an optional
**single-height-sample, offset-limited parallax** tier, is the only clearly
low-cost fallback path worth evaluating first. Precomputed ray-intersection
methods (cone/distance maps) are possible middle/high-quality experiments, not
low-end defaults. Virtual texturing solves residency, not POM's per-pixel march.

## Question

Can the engine use a much cheaper fallback than full parallax occlusion mapping
(POM) on slow hardware while retaining some surface-depth cue?

## Measured constraint: Radeon 680M

The temporary CEF/WebGPU evaluation scene ran at 1440×900 logical pixels with
DPR 2 (2880×1800 physical pixels) on fox-laptop's Radeon 680M. Hardware WebGPU
was positively verified (`amd` / `rdna-2`); runs with a CEF GPU-process crash or
WebGL2 fallback were rejected. GNOME's compositor throttles rAF to approximately
1 Hz while the screen is locked, so the session must be **unlocked only for the
short measurement at normal brightness and locked again immediately afterward**;
locked-screen timing is invalid.

| Surface mode | Result |
|---|---:|
| Normal map | 59.6 FPS |
| Full low POM (8–32 layers + silhouette + self-shadow + relief shadows) | 53.4 FPS |
| Full medium POM (16–96 layers) | 35.2 FPS |
| Full high POM (32–160 layers) | 24.3 FPS |
| Low-core POM (8–32 layers; no silhouette, self-shadow, or relief shadows) | **60.0 FPS**, p99 16.68 ms, 0/300 below 60 |

The workload is fragment-bound: POM's per-pixel height intersections dominate
for the large nearby wall/floor coverage. Reducing mesh triangles alone does not
meaningfully reduce that shaded pixel count.

## Candidate fallback chain

### 0. Normal mapping — universal minimum tier

Sample a precomputed tangent-space normal (and ordinary PBR textures) without
any height-ray intersection. It gives lighting detail but no UV shift,
self-occlusion, or silhouette change.

This is the correct baseline for slow hardware, distant surfaces, small
screen-space coverage, and all materials that do not need visible depth. The
680M normal-map scene was essentially vsync-limited, whereas full POM was not.

### 1. Offset-limited parallax mapping — the cheap depth-cue tier

Classic parallax mapping samples the height **once**, offsets UV in tangent-view
direction, then uses the ordinary material samples at that offset. Clamp the
maximum offset and fade it to normal mapping at grazing angles, low projected
size, or high motion.

GPU Gems 2 describes this as a surprisingly useful approximation requiring only
three extra shader instructions. Its limitations are intentional and define this
tier: it cannot represent high-frequency/large displacement, self-occlusion,
self-shadowing, or a true silhouette. It should therefore be used for subtle
plate seams, brick/grain, and shallow grooves—not deep holes or overhangs.

**Expected cost relative to POM:** one height fetch plus arithmetic rather than
8–32 or more height-loop fetches. This is the only candidate expected to be
*dramatically* cheaper than low-core POM while preserving a parallax cue. This
is an inference from its algorithmic work, not a benchmark of this engine.

### 2. Low-core POM — close/high-coverage opt-in tier

The measured viable POM setting is the already-tested low-core configuration:
8–32 view-adaptive layers, with no silhouette discard, self-shadow march, or
relief-shadow/depth pass. It is not a universal material default; it needs an
explicit screen-coverage/distance material-LOD gate and a normal-map fallback.

### 3. Precomputed cone or distance acceleration — investigate only after tiers 0–2

Relaxed cone stepping (RCS) stores an offline cone ratio per height texel,
leaps through empty space, then binary-refines an intersection. Distance mapping
uses a precomputed 3D distance field to choose safe ray steps. Both can reduce
intersection iterations for detailed height fields, but add precomputation,
texture memory/bandwidth, quality corner cases, and still execute a per-fragment
ray intersection.

They are **not** the slow-hardware fallback. Benchmark them only if the visual
need for more accurate close-up depth cannot be met by low-core POM or authored
geometry. In particular, do not presume they outperform the current 8–32-layer
path on an integrated GPU.

### 4. Maximum-mipmap height hierarchy — strongest high-quality analytical candidate

Tevs, Ihrke, and Seidel's *Maximum Mipmaps* method constructs an implicit
bounding-volume hierarchy from a maximum-height mip chain. A ray begins coarse,
skips cells whose maximum height cannot intersect it, and descends only near a
possible hit. It provides view-dependent termination: once a height-field cell
projects below a pixel, traversal can stop at that level. The auxiliary data is
mipmap-like rather than the large 3D distance field or costly static cone map.

This is the most promising **high-quality middle-tier experiment** for large or
dynamic height fields. It can reduce average intersection work substantially at
distance and on maps with empty space. It is not a free replacement for POM:
nearby, dense, grazing, or highly irregular surfaces still traverse hierarchy
levels; divergent fragment paths and hierarchy texture reads may perform poorly
on a WebGPU integrated GPU. Its published results are from 2008 hardware and
are not a performance prediction for the 680M.

A useful negative result from the same paper: a closed-form analytic
ray/bilinear-patch intersection was slower on GPU than a small iterative
refinement because of branching. “Analytical” therefore means a different
cost/quality trade, not automatically lower cost.

### 5. Authored geometry / trims for major silhouette features

For doors, deep cracks, flanges, and other features where silhouette or cast
shadow is materially important, use actual low-poly geometry/trim/decal assets
near the camera and ordinary normal maps farther away. This exchanges repeated
fragment ray marching for bounded geometry that the mesh LOD system can reduce
or remove with distance. It is a content/render-policy option, not a claim that
more triangles are always cheaper.

## Why there is no universal analytic replacement

For arbitrary scanned height fields, exact view-dependent visibility requires
finding a ray/height intersection. A technique can only move the cost between:

- repeated runtime height samples (POM);
- hierarchy/auxiliary samples plus branches (maximum mipmaps, cone maps);
- precomputation and memory (cone maps, distance fields, view-dependent
  displacement lookup data);
- actual geometry and its raster/LOD cost; or
- temporal/low-resolution reconstruction artifacts.

The appropriate high-quality strategy is consequently a tiered policy, not one
shader: normal map → one-tap parallax → low-core POM or a measured hierarchical
method only while projected coverage warrants it → authored geometry for
important silhouettes.

## Integrated implementation and reference audit (2026-07-16)

The renamed Dungeon combines low-core POM with VT using the official ambientCG
1K, 16-bit displacement maps. The first integration incorrectly substituted AO
for height; AO encodes occluded lighting and produced weak, incorrect relief.
The corrected path uses adaptive 8–32 layers, scale 0.05, ratio-2 offset
limiting, and no radial per-fragment fade. The fade was removed because its
moving contour was plainly visible as a wave of flattening. Ray depth intersects
`1-height`.

The equations now match LearnOpenGL's known-working implementation: geometric
tangent-space view ray, `view.xy/view.z`, subtracting the layer delta, first
height crossing, then before/after linear refinement. Three's official
`parallaxUV` confirms the un-negated direction convention. Three r185's
`parallaxDirection` cannot be used directly here because it includes the
normal-mapped `normalView`, while that normal map itself requires the POM UV.
That cycle previously sampled uninitialized normal/AO state and darkened the
wall as the distance fade enabled.

The corrected graph computes its ray from the geometric TBN, marches once in
the first material flow, and publishes initialized UV/mip values for albedo,
normal, roughness, and AO. It also restores the prototype's light-source relief
shadow as a bounded 8-sample ray that attenuates direct diffuse/specular at 82%
strength. A bootstrap generated-WGSL gate requires one march before three linked
VT samples and rejects a material-normal TBN dependency.

Corrected 680M timing at 2880×1800: fixed POM+self-shadow 59.87 FPS, p99 16.68
ms, 1/600 below 55; base 59.77 FPS, p99 16.68 ms, 2/600 below 55; moving POM
59.97 FPS, 0/300 below 55. GPU main work was 1.02–1.09 ms and timestamp total
5.49–7.25 ms. Isolated misses occur in base too. Changing VT feedback from every
4 frames (3/300 misses) to every 8 brought POM into the baseline range.

## What existing engine systems can and cannot do

### Mesh LOD

`AssetStore` currently creates 100/50/25/10-percent mesh LOD data, but
`RenderAdapter` has no automatic distance/projected-size material-LOD selector.
Mesh LOD can lower vertex, draw, and shadow cost. It does not remove POM cost on
a wall that occupies the same pixels. Radial per-fragment fading was rejected
because it made a moving flattening wave. A future policy must select or blend
whole materials using object bounds/projected coverage without a screen-visible
per-pixel distance contour.

### Virtual texturing

The VT system selects resident texture mips/pages and bounds streaming work. It
does not reduce a POM loop's number of ray-height samples. Marching directly
through an asynchronously resident height VT produced page-boundary instability,
so the integrated tier keeps a compact matching height field resident. The
final displaced PBR lookups remain virtual and use bounded coarser-page/tail
fallback with stable base-UV gradients.

VT makes the 8K materials feasible but is not itself a POM loop optimization.
Corrected hardware validation must validate the composition rather than
assuming the independent POM and VT results add linearly.

## Completed evaluation plan

1. **Complete:** compared normal, one-tap, low-core POM, and optional contact
   shadow modes at target physical resolution.
2. **Complete:** measured post-warm-up p50/p99/max/drop counts and fail-closed
   WebGPU state on the 680M.
3. **Rejected after integration:** radial 65–100% distance fade produced a
   visible moving flattening wave. Whole-surface projected-coverage policy
   remains future work.
4. **Not pursued:** low-core POM passed visual/performance review, so RCS
   preprocessing is unnecessary.
5. **Complete:** VT and POM were first measured separately, then combined using
   a resident matching height field and bounded displaced-page fallback.

## Sources

1. Joey de Vries, **LearnOpenGL: Parallax Mapping** (known-working adaptive
   POM loop and linear intersection refinement):
   https://learnopengl.com/Advanced-Lighting/Parallax-Mapping
2. Natalya Tatarchuk, **Practical Parallax Occlusion Mapping for Highly
   Detailed Surface Rendering**:
   https://advances.realtimerendering.com/s2006/Tatarchuk-POM.pdf
3. William Donnelly, **GPU Gems 2, Chapter 8: Per-Pixel Displacement Mapping
   with Distance Functions** (parallax mapping as one height-sample UV offset;
   its cost and limitations):
   https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-8-pixel-displacement-mapping-distance-functions
4. Fabio Policarpo and Manuel M. Oliveira, **GPU Gems 3, Chapter 18: Relaxed
   Cone Stepping for Relief Mapping** (precomputed cone-map ray acceleration,
   benefits and costs):
   https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-18-relaxed-cone-stepping-relief-mapping
5. Art Tevs, Ivo Ihrke, and Hans-Peter Seidel, **Maximum Mipmaps for Fast,
   Accurate, and Scalable Dynamic Height Field Rendering** (hierarchical height
   traversal and view-dependent termination):
   https://pure.mpg.de/rest/items/item_1325622_5/component/file_3590464/content
6. Engine API: [`../api/virtual-texturing.md`](../api/virtual-texturing.md)
   (VT page-table/fallback behavior and current scope).
7. Engine VT audit:
   [`../audits/virtual-texture-vertical-slice-2026-07-15.md`](../audits/virtual-texture-vertical-slice-2026-07-15.md).
