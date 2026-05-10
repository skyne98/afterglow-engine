# Spartan Engine AABB Hi-Z Culling Case Study

## Source Snapshot

Repository: `PanosK92/SpartanEngine`  
Inspected commit: `154e15bc71f6d16f8ed39932e6e1b1b9252f296f`

This note intentionally focuses only on Spartan's AABB-based visibility pieces:

- renderable bounding boxes
- occluder selection
- occluder depth/HZB construction
- AABB Hi-Z testing
- skinned/deforming fallback implications

Other GPU-driven details in Spartan are out of scope here.

## Why This Matters For Afterglow

Our intended first culling system is proxy-box-based:

- one or more conservative occludee AABBs per model
- optional one or more occluder AABBs per model
- optional bone attachment for skinned/deforming proxies
- dedicated occluder depth
- HZB generation
- compute visibility test

Spartan's AABB path is a useful reference because it already applies a conservative renderable-level box test against a reverse-Z HZB, and it uses that path when finer static bounds are not trustworthy.

## Frame Placement

Spartan runs Hi-Z before its main geometry output:

1. build occluder HZB
2. run indirect culling
3. render depth prepass
4. render G-buffer

For our deferred renderer, the key lesson is the same: if culling is meant to save G-buffer work, the occluder depth and visibility pass must happen before the G-buffer pass.

References:

- [`Renderer_Passes.cpp:147-158`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Passes.cpp#L147-L158)
- [`Renderer_Passes.cpp:559-586`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Passes.cpp#L559-L586)
- [`Renderer_Passes.cpp:670-712`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Passes.cpp#L670-L712)

## Renderable AABB Generation

Spartan computes a mesh-space bounding box from mesh vertices when a renderable is initialized. The render component then updates a world-space bounding box during `Tick()`.

For non-instanced renderables:

```text
world_aabb = mesh_aabb * entity_transform
```

For instanced renderables:

```text
world_aabb = union(mesh_aabb * instance_transform * entity_transform)
```

The same world-space AABB feeds CPU frustum/distance visibility and the GPU AABB fallback data.

References:

- [`Render.cpp:199-218`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/World/Components/Render.cpp#L199-L218)
- [`Render.cpp:221-235`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/World/Components/Render.cpp#L221-L235)
- [`Render.cpp:540-557`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/World/Components/Render.cpp#L540-L557)
- [`Render.cpp:560-579`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/World/Components/Render.cpp#L560-L579)

## AABB Upload

Spartan writes renderable AABBs into a bindless AABB array. The prepass AABBs and the indirect-renderable AABBs share the same array layout.

Each uploaded AABB stores:

- world-space min
- world-space max
- occluder marker

This maps directly to our planned `OccludeeProxy` upload path, except our first version should use authored local-space proxy boxes transformed to world space during extraction. Multiple proxies for one model should be uploaded as separate records.

References:

- [`Renderer.cpp:1262-1275`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer.cpp#L1262-L1275)
- [`Renderer.cpp:1277-1312`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer.cpp#L1277-L1312)

## Occluder Selection

Spartan does not use every object as an occluder. It selects a small set of large, stable occluders from the prepass draw list:

- material must exist
- material must not be transparent
- renderable must not be instanced
- draw call must be camera-visible
- score is projected screen-space AABB area
- previous-frame occluders receive a `1.5x` score bonus
- maximum occluders: `64`

This is a good engine policy even if we author occluder proxies. We can still rank eligible proxies by projected area and cap how many are drawn into the occluder depth buffer.

References:

- [`Renderer.cpp:1510-1567`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer.cpp#L1510-L1567)
- [`Renderer_Definitions.h:400-410`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Definitions.h#L400-L410)

## Occluder Depth And HZB

Spartan creates two occlusion textures:

- `depth_occluders`: `D32_Float`
- `depth_occluders_hiz`: `R32_Float`, UAV/SRV, full mip chain, per-mip views

The Hi-Z pass:

1. clears the occluder depth buffer to `0.0`
2. optionally renders selected occluders
3. blits depth into the HZB texture
4. downsamples with `Min`

The comments explain why the pass always clears and rebuilds the HZB even when occlusion is disabled or suppressed: stale depth can cause nondeterministic false culling.

The `Min` downsample is the conservative reduction for reverse-Z, where `0.0` is the far plane.

References:

- [`Renderer_Resources.cpp:443-450`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Resources.cpp#L443-L450)
- [`Renderer_Passes.cpp:420-483`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Passes.cpp#L420-L483)
- [`Renderer_Definitions.h:392-397`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_Definitions.h#L392-L397)
- [`Renderer_ConsoleVariables.cpp:190-191`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_ConsoleVariables.cpp#L190-L191)

## AABB Hi-Z Test

The core AABB visibility function is `aabb_hiz_visible()`.

Its conservative behavior:

- project all 8 world-space AABB corners
- if no corner is in front of the camera, reject
- if the box partially crosses behind the camera, keep it visible
- reject if the projected rectangle is fully outside screen NDC
- clamp the projected rectangle to UV space
- choose a mip level from the projected pixel size
- refine one mip lower if the rectangle still fits in a small texel footprint
- expand UV bounds by one mip texel before sampling
- sample four corners plus center
- for reverse-Z, take the minimum sampled depth
- compare the AABB's closest projected depth against the HZB depth with bias

The important policy is the near-plane rule: partial-behind AABBs skip occlusion and remain visible. That is the right direction for avoiding false culls.

References:

- [`indirect_cull.hlsl:59-146`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/data/shaders/indirect_cull.hlsl#L59-L146)

## Deforming And Skinned Meshes

Spartan's AABB fallback is the relevant part for skinned/deforming meshes.

Spartan disables its tighter static local-bound path for skinned meshes by default because those local bounds are inexact after skinning. The draw data then marks skinned draws with a flag when that path should be avoided, and the culling shader uses the renderable AABB fallback.

For our box-based design, this reinforces the same rule:

- skinned meshes should be culled by a conservative renderable-level animated AABB first
- smaller static submesh bounds are not valid after deformation unless they are generated from animation/skinning-aware data

References:

- [`Renderer_ConsoleVariables.cpp:190-191`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer_ConsoleVariables.cpp#L190-L191)
- [`Renderer.cpp:1463-1484`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/source/runtime/Rendering/Renderer.cpp#L1463-L1484)
- [`indirect_cull.hlsl:24-29`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/data/shaders/indirect_cull.hlsl#L24-L29)
- [`indirect_cull.hlsl:186-199`](https://github.com/PanosK92/SpartanEngine/blob/154e15bc71f6d16f8ed39932e6e1b1b9252f296f/data/shaders/indirect_cull.hlsl#L186-L199)

## Afterglow Translation

For `afterglow-engine`, the Spartan-inspired box-only design should be:

1. Build world-space occludee proxy AABBs.
2. Build a list of eligible occluder proxy AABBs.
3. Rank occluder proxies by projected screen area.
4. Keep a cap, initially `64`.
5. Add hysteresis for previous-frame occluders.
6. Render selected proxies into a dedicated reverse-Z depth target.
7. Clear the target every frame to far depth.
8. Build an `R32Float` HZB with `min` reduction.
9. Run compute AABB tests against the HZB.
10. Treat near-plane/camera-intersecting boxes as visible.

Skinned meshes:

1. Use authored fallback proxy AABBs immediately.
2. Support bone-attached occludee proxies for rotating limbs/torso/head.
3. Add per-clip or per-joint animated AABB generation later.
4. Do not use static tight bounds for skinned occlusion tests unless they cover the full deformation range.

## What Not To Copy Yet

Do not begin with a full custom draw pipeline. The useful Spartan idea for us is the box-based visibility policy:

- selected occluders
- always-valid HZB
- conservative AABB projection
- explicit fallback for deformation

That is enough to design a robust first implementation inside Bevy.
