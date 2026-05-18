# Hi-Z Depth / HZB Occlusion Culling Research

## Scope

This note focuses on GPU-side occlusion culling with a hierarchical depth buffer (Hi-Z / HZB), with emphasis on:

- how the depth pyramid works
- how it interacts with deferred shading
- how to implement it conservatively and efficiently
- how to support an authored two-bounds workflow per model:
  - a larger **occludee bound** for compute visibility tests
  - a smaller **occluder proxy bound** for the occluder depth/HZB pass

This is written for `afterglow-engine`, but most of the underlying material is renderer-agnostic.

## Executive Summary

- HZB occlusion culling works by building a mip pyramid of a depth buffer and testing projected object bounds against that pyramid instead of rasterizing the full object.
- The pyramid must be **conservative**:
  - standard Z (`near=0`, `far=1`, compare `LESS`): store the **farthest** depth per texel footprint, typically `max`
  - reversed Z (`near=1`, `far=0`, compare `GREATER`): store the **farthest** depth under that convention, typically `min`
- HZB only saves same-frame deferred shading work if you produce usable depth **before** the expensive opaque/G-buffer pass. That implies either:
  - a full or selective depth prepass, or
  - a two-phase pipeline that starts from last frame's visibility/HZB and retests against a newly built HZB in the same frame
- Your proposed dual-box workflow is sound **if kept conservative**:
  - a **larger occludee bound** is safe, but less efficient
  - a **smaller occluder proxy** is safe, but less effective
  - a loose box used as an occluder is **not** safe if it extends into empty space or seals real openings
- For an engine-first implementation, the most practical starting point is:
  1. render authored occluder proxies in a depth-only pass
  2. build a single-sample HZB
  3. frustum-cull + HZB-cull authored occludee bounds in compute
  4. draw only visible instances into the main opaque/deferred pass

## Terminology

- **Occluder**: geometry that writes depth and can hide other things.
- **Occludee**: geometry that may be hidden by occluders.
- **Hi-Z / HZB / depth pyramid**: a mip chain of depth values used for coarse visibility queries.
- **Conservative**: biased toward "visible" if uncertain. False visible is acceptable; false culled is not.

## How HZB Works

The original HZB paper by Greene, Kass, and Miller combines object-space hierarchy with an image-space Z pyramid to reject hidden geometry quickly. Modern real-time engines usually keep the image-space side and pair it with GPU-driven bounds testing rather than an octree traversal.

The practical idea is simple:

1. render some occluding depth
2. downsample that depth into a pyramid
3. project an object's bound into screen space
4. choose a mip level that covers that screen-space footprint
5. compare the object's nearest possible depth against the HZB depth for that footprint
6. if the bound is fully behind the conservative depth already in the pyramid, skip the object

### What Each Mip Stores

The pyramid must store the depth extreme that lets a coarse texel conservatively represent all finer texels under it.

| Depth convention | Depth meaning | Conservative reduction |
|---|---|---|
| Standard Z | larger depth = farther | `max` |
| Reversed Z | smaller depth = farther | `min` |

This is the single most important correctness rule in the system.

If the reduction is wrong, the pyramid stops being conservative and false occlusion becomes possible.

### Why Conservative Reduction Matters

For occlusion culling, each coarse texel must answer the question:

> "What is the farthest depth already occupied anywhere in this footprint?"

If even one pixel in the footprint is closer than the candidate object, the object might still be visible through that pixel. The HZB therefore must not overstate coverage or depth.

This is why conservative HZB systems usually err toward:

- conservative depth reduction
- conservative projected bounds
- conservative nearest-depth tests

That combination produces missed culling opportunities, not disappearing objects.

## Building the Depth Pyramid Correctly

### Base Buffer

The base of the pyramid is usually one of:

- a dedicated **occluder depth pass**
- an **early depth prepass**
- a reused **previous-frame depth/HZB**

For this engine and your proposed workflow, a dedicated occluder depth pass using authored occluder proxies is the cleanest starting point.

### NPOT and Odd Dimensions

Non-power-of-two dimensions are an easy place to make the pyramid incorrect.

At the right and bottom edges, a parent texel may need to absorb a `2x3`, `3x2`, or `3x3` footprint rather than a clean `2x2`. If you ignore those extra children, the pyramid becomes non-conservative near the edges.

Implementation rule:

- every parent texel must conservatively cover **all** child texels in its footprint, including odd edge texels

### MSAA

If the source depth is multisampled, you need a conservative resolve before or during HZB construction.

Practical rule:

- reduce all samples for a pixel conservatively first
- then reduce across pixels conservatively into the next mip

If this is not done, the base mip can already be non-conservative.

### Integer Addressing

Use texel-space integer math for downsampling and lower-mip indexing. Several practical writeups call out float-coordinate math as a source of subtle lower-mip errors.

### Efficient Pyramid Construction

There are two practical paths:

- **simple path**: one compute dispatch per mip
- **optimized path**: a single-pass or reduced-pass downsampler such as AMD FidelityFX SPD

For a first implementation, the simple path is easier to validate. Once the system is stable, SPD-style downsampling is a sensible optimization.

## Testing Bounds Against the HZB

### Recommended Bound Type

For this engine, use **AABBs or OBB-derived AABBs** for authored occludee bounds, because your content goal is explicit bound authoring.

Spheres are cheaper and sometimes good enough, but boxes fit long or irregular models better and map more naturally to an authored pipeline.

### Compute Test Outline

Per instance:

1. frustum-cull the bound first
2. reject near-plane-straddling or camera-inside edge cases conservatively as visible
3. project the bound to screen space
4. compute its screen-space rectangle
5. estimate a mip level from the rectangle size
6. sample the HZB conservatively over that rectangle
7. compare against the bound's nearest depth
8. append visible instances to an output list

### Use the Bound's Nearest Depth, Not Its Center

Testing against center depth is not conservative. The right comparison is based on the point on the bound that can be nearest to the camera over the sampled footprint.

For boxes, that means the projected rectangle plus a conservative nearest-depth estimate for the box under the current view.

### Mip Selection

Common heuristics:

- choose the lowest mip where the screen rect fits in roughly `2x2` texels
- some modern systems use a coarser threshold such as `4x4` for efficiency, then refine if needed

What matters is consistency:

- too fine: more bandwidth and more samples
- too coarse: weaker culling and more misses on thin occluders

For a first version:

- pick the coarsest mip where the projected rect is at most `2x2` texels
- sample the covered texels conservatively

### Near-Plane and Camera-Inside Cases

If the projected bound crosses the near plane or the camera is inside the bound, just mark it visible.

Trying to "fix" those cases inside the same coarse HZB test often causes the most obvious false culls in practice.

## Deferred Shading: What Changes

HZB does not magically help deferred shading unless depth exists early enough to prevent expensive G-buffer work.

That leads to four practical pipeline shapes.

### 1. Full Depth Prepass -> HZB -> G-buffer/Base Pass

This is the cleanest mental model:

1. render depth
2. build HZB
3. cull objects
4. render visible opaque geometry into the G-buffer or base pass

Pros:

- straightforward
- robust
- culls same-frame opaque work

Cons:

- the full prepass is not free

### 2. Selective Occluder Prepass -> HZB -> G-buffer/Base Pass

This is closer to your intended design:

1. render only authored occluder proxies
2. build HZB
3. HZB-cull all occludees
4. render visible opaque geometry normally

Pros:

- cheaper than a full prepass
- matches the dual-box authoring model
- easy to control artistically

Cons:

- misses occlusion from non-proxy geometry
- requires care around hollow assets, doors, and thin structures

### 3. Previous-Frame HZB -> Render -> Current-Frame HZB -> Retest

This is the modern two-phase temporal approach used by GPU-driven systems such as Nanite-style pipelines:

1. cull against previous-frame HZB
2. render survivors
3. build current-frame HZB
4. retest previously occluded content

Pros:

- avoids requiring a full same-frame depth prepass for everything
- handles temporal coherence well

Cons:

- more complex
- needs careful handling for camera cuts and fast disocclusion

### 4. G-buffer First -> Build HZB Later

This is valid, but it does **not** save the opaque work that already filled the G-buffer.

It can still help later passes such as:

- SSAO / SSR style screen-space work
- deferred light or volume culling
- later visibility-dependent passes

It is not the right answer if the goal is to cull the expensive opaque/deferred geometry pass itself.

## Deferred Shading Constraints

### Transparent and Forward-Only Materials

Deferred renderers generally keep true transparency out of the G-buffer path and render it later in a forward pass. That means:

- transparent materials should generally be **occludees only**
- they should usually **not** act as occluders

### Alpha-Tested / Masked Materials

Masked materials can participate in depth, but only if the prepass and the shading pass have matching coverage behavior.

If the depth pass and main pass disagree on:

- alpha clipping
- dithering
- vertex displacement
- wind / WPO-style deformation

then the HZB becomes a poor representation of the real opaque surface.

### Early-Z

HZB and Early-Z are complementary:

- HZB removes entire objects before rasterization
- Early-Z removes failing fragments inside rasterization

Early-Z can be weakened or disabled by shader behavior such as writing depth or using unordered access operations. That matters because some prepass and material designs that look "the same" visually do not actually behave the same to the hardware.

### Decals and Prepass Coupling

In some deferred renderers, features like decals already force or strongly encourage a prepass. In those cases, the incremental cost of HZB may be easier to justify because part of the prerequisite depth work already exists.

### Bevy-Specific Note

As of Bevy `0.18.1`, Bevy's experimental built-in `OcclusionCulling` is documented as requiring `DepthPrepass` and is currently documented as incompatible with deferred shading.

That is a useful reminder: renderer integration details matter. HZB is not just a math problem; it is a pass-ordering problem.

## Your Proposed Dual-Box Authoring Scheme

This is the right shape **if we keep the safety rules strict**.

### Box 1: Large Occludee Bound

Purpose:

- used only for frustum + HZB visibility tests in compute

Safety rule:

- it must fully contain every visible pose/state of the model that can appear at runtime

Effects:

- larger bound -> safer, but weaker culling
- smaller bound -> better culling, but risk of false disappearance

For skinned or vertex-deformed content, this bound must include:

- imported animation extremes
- additive animations
- runtime bone manipulation
- cloth / ragdoll / WPO if applicable

### Box 2: Small Occluder Proxy

Purpose:

- rendered into the occluder depth buffer that feeds the HZB

Safety rule:

- it must never produce screen-space coverage that the real opaque asset would not produce from relevant viewpoints

That means:

- if the proxy is smaller than the real opaque body, it is often safe but less effective
- if the proxy sticks into empty space, bridges gaps, or seals real openings, it is unsafe

This is the core rule that kills most naive "just use bounding boxes as occluders" schemes.

### When the Proxy Box Is Safe

Good candidates:

- solid rocks
- pillars
- monolithic walls
- terrain chunks
- large props with no relevant holes

Bad candidates:

- arches and door frames
- windows
- fences
- foliage
- hollow buildings you can enter
- animated doors unless the proxy follows the door state correctly

A proxy that is fully inside the model's overall volume can still be wrong if it becomes visible through a door, window, arch, or other opening. Safety is about conservative **screen-space coverage**, not just local-space containment.

### Do Not Use Loose AABBs as Occluders

NVIDIA explicitly warns against using a loose bounding box itself as the occluder, because a box can cover screen space that the real enclosed geometry does not actually occlude.

That is the distinction between:

- **occludee bounds**: can be loose, because looseness causes false visible
- **occluder proxies**: must not over-cover, because looseness causes false culled

### One Proxy Box vs Two or More

You asked for optionally one or two boxes per model. That is reasonable as a first authoring model, but there are limits:

- one proxy box is fine for many solid props
- some assets need no proxy at all
- some structures really need multiple boxes or a dedicated proxy mesh

If a single proxy box must either:

- protrude outside real opaque coverage, or
- close an opening that should remain visible

then a single box is the wrong representation.

## Recommended Renderer Design for `afterglow-engine`

### First Shipping Target

Implement this first:

1. **Authoring**
   - `occludee_bounds`: optional authored local-space AABB, else derived from mesh bounds
   - `occluder_proxy`: optional local-space box; if absent, object is not used as an occluder proxy

2. **Per-frame instance data**
   - world transform
   - occludee bound in world space
   - occluder proxy transform/bounds if present

3. **Occluder depth pass**
   - render only proxy boxes
   - depth-only
   - single-sampled
   - opaque only

4. **HZB build**
   - compute downsample the proxy depth into mip chain
   - standard Z: `max`
   - reversed Z: `min`

5. **Cull compute pass**
   - frustum first
   - HZB test second
   - append visible instance IDs to a list

6. **Main opaque/deferred pass**
   - draw only visible instances

This is the simplest version that matches your content model and provides same-frame savings.

### Resolution Recommendation

Start with:

- a **single-sample depth texture**
- same resolution as the main depth buffer or a modestly reduced resolution such as half-res

Do **not** start with an aggressive quarter-res or tiny buffer just to be clever. Low resolution is safe if conservative, but it reduces occluder usefulness, especially for thin walls and narrow pillars. Measure first.

### Conservative Rasterization

If the target API/platforms support overestimated conservative rasterization cleanly, it can improve the safety of the occluder proxy pass by ensuring partial coverage is not missed.

It is an enhancement, not a requirement.

### Static First, Dynamic Later

Start with:

- static occluder proxies
- static or dynamic occludees

Delay dynamic occluders until the base system is stable. Dynamic/skinned occluders are possible, but they make validation harder and authoring less obvious.

### Hollow Structures

For enterable buildings and similar hollow geometry:

- avoid a single interior-filling occluder box
- either use a better proxy representation or disable occluder behavior for that asset

If the camera can enter a space that the proxy treats as solid, the proxy can over-occlude badly.

## Operational Checklists And Sources

Correctness checks, optimization order, debug views, engine policy, and source
links live in [hiz-depth-occlusion-culling-checklists.md](hiz-depth-occlusion-culling-checklists.md).

## Bottom Line

For this engine, the most practical design is:

- authored **large occludee bounds**
- authored **small static occluder proxies**
- **same-frame occluder depth pass**
- **conservative HZB**
- **compute visibility**
- then **deferred or forward opaque rendering only for visible instances**

That gives you a conservative system that matches your content intent and can evolve later into a fuller GPU-driven visibility pipeline.
