# Spartan-Style AABB Hi-Z Culling in Bevy 0.18

## Scope

This note designs an `afterglow-engine` GPU-driven culling system around Spartan's AABB Hi-Z policy first, then maps that design onto Bevy `0.18.1`. It covers:

- the Spartan-inspired frame order, resources, and conservative box rules
- how Bevy's current render schedule and graph work
- how skinned meshes should become cheap first-party citizens

## Current Bevy Render Shape

Bevy rendering runs in a render sub-app. The integration points we care about are:

- `ExtractSchedule`: copy main-world bounds, transforms, and skin inputs into the render world
- `RenderSystems::PrepareResources`: allocate/upload culling buffers and textures
- `RenderSystems::PrepareBindGroups`: create HZB and culling bind groups
- `Core3d` render graph: insert occluder depth, HZB, and AABB cull nodes before main deferred/G-buffer work

## Bevy Already Has a GPU Culling Path

Bevy `0.18.1` already has a standard-mesh GPU preprocessing system in `bevy_pbr`. It expands `MeshInput` into per-view mesh uniforms, does frustum/HZB culling when enabled, supports early/late preprocessing, and writes indirect metadata. Relevant local source files:

- `bevy_render-0.18.1/src/lib.rs`
- `bevy_core_pipeline-0.18.1/src/core_3d/mod.rs`
- `bevy_core_pipeline-0.18.1/src/experimental/mip_generation/mod.rs`
- `bevy_pbr-0.18.1/src/render/gpu_preprocess.rs`
- `bevy_pbr-0.18.1/src/render/mesh.rs`
- `bevy_pbr-0.18.1/src/render/mesh_preprocess.wgsl`
- `bevy_pbr-0.18.1/src/render/occlusion_culling.wgsl`

Bevy's documented `OcclusionCulling` component is view-level and experimental. It requires `DepthPrepass`, is ignored without it, and is documented as incompatible with deferred shading.

## Design Direction

Primary rule: copy Spartan's box-culling policy, not its whole renderer.

The engine design should be:

1. explicit authored occludee proxy AABBs
2. optional authored occluder proxy AABBs
3. ranked, capped occluder submission
4. dedicated reverse-Z occluder depth
5. always-rebuilt `R32Float` HZB with `min` reduction
6. conservative compute AABB visibility test
7. visibility flags or indirect draw arguments consumed by the draw path

Bevy is the integration target:

1. use Bevy extraction to copy authored bounds into the render world
2. use Bevy render resources to allocate/upload buffers and textures
3. insert our graph nodes before deferred/G-buffer work
4. initially consume culling through engine-owned visibility/debug buffers
5. later wire the result into Bevy's preprocess/phase machinery where hooks allow it

This keeps the algorithm Spartan-shaped while avoiding an immediate renderer rewrite.

## Spartan-First Runtime Plan

### 1. Proxies Are Engine-Owned

Do not depend on Bevy's mesh asset AABB as the authoritative occlusion bound.

Per renderable, the engine owns one or more occlusion proxies:

```rust
pub struct GpuOcclusionProxy {
    pub owner: Entity,
    pub kind: GpuOcclusionProxyKind,
    pub center_ws: Vec3,
    pub half_extents_ws: Vec3,
    pub flags: u32,
}

pub enum GpuOcclusionProxyKind {
    Occludee,
    Occluder,
}
```

Specific rules:

- at least one `OccludeeProxy` is required for `GpuCulled` entities
- missing occludee proxies mean the entity is always visible and emits a debug warning
- multiple occludee proxies are allowed; the object is visible if any proxy is visible
- `OccluderProxy` entries are optional and should cover only solid mass
- multiple occluder proxies are allowed; each proxy is ranked independently
- skinned meshes use authored or animation-aware occludee proxies
- skinned occludee/occluder proxies may be attached to bones and transformed by the current joint pose

### 2. Occluder Selection Matches Spartan

Every frame, per view:

1. gather visible `OccluderProxy` records, including bone-attached records
2. reject transparent, disabled, non-finite, or camera-intersecting proxies
3. project the proxy AABB
4. compute screen-space rectangle area
5. multiply score by `1.5` if it was selected last frame
6. sort descending by score
7. keep `64` proxies initially

Keep these constants explicit:

```rust
pub const MAX_OCCLUDERS_PER_VIEW: usize = 64;
pub const PREVIOUS_OCCLUDER_BONUS: f32 = 1.5;
```

### 3. Occluder Depth And HZB

Per camera view, allocate:

```text
AfterglowOccluderDepth: Depth32Float, full view size
AfterglowOccluderHzb: R32Float, full mip chain, sampled + storage
```

Pass order:

1. clear occluder depth to reverse-Z far depth, `0.0`
2. render selected occluder boxes only
3. copy or shader-blit depth level 0 into HZB level 0
4. downsample mips with `min`

The HZB must be rebuilt every frame. If occlusion is disabled for debugging, still clear/rebuild or force all objects visible. Do not leave stale depth in a reusable texture.

### 4. AABB Compute Test

The compute shader should follow Spartan's conservative shape:

1. load world-space occludee AABB
2. project all 8 corners
3. if no corner is in front of the camera, mark not visible
4. if some corners are behind the near plane, mark visible
5. reject only when the projected rectangle is fully outside screen
6. clamp rectangle to UV space
7. choose mip from projected pixel width/height
8. expand UV rectangle by one texel at that mip
9. sample four corners plus center
10. for reverse-Z, use the minimum sampled depth
11. compare closest projected AABB depth against HZB depth with bias

Initial output:

```rust
pub struct GpuCullResult {
    pub entity: Entity,
    pub visible: bool,
    pub reason: u32,
}
```

Later output:

- compact visible IDs
- update indirect draw counts
- feed Bevy phase or custom draw path

## Engine API

Authoring components:

```rust
#[derive(Component, Clone, Copy, Debug, Reflect)]
pub struct OccludeeProxy {
    pub local_min: Vec3,
    pub local_max: Vec3,
    pub joint: Option<Entity>,
}

#[derive(Component, Clone, Copy, Debug, Reflect)]
pub struct OccluderProxy {
    pub local_min: Vec3,
    pub local_max: Vec3,
    pub joint: Option<Entity>,
}

#[derive(Component, Clone, Copy, Debug, Reflect)]
pub struct GpuCulled;

#[derive(Component, Clone, Copy, Debug, Reflect)]
pub struct SkinnedCullBounds {
    pub mode: SkinnedCullBoundsMode,
}
```

Default policy:

- static meshes get an automatically derived occludee bound
- skinned meshes get conservative animated occludee proxies
- occluder proxies are opt-in only
- skinned meshes are occludees by default, not occluders
- proxy components may appear multiple times per model through child entities or an asset-side proxy list

## Bevy Integration Plan

### Stage 1: Authoring and Extraction

Add a visibility plugin:

```text
src/visibility/
  mod.rs
  components.rs
  extract.rs
  bounds.rs
  skinned_bounds.rs
  debug.rs
```

Register extraction systems in `ExtractSchedule`:

- extract `OccludeeProxy`
- extract `OccluderProxy`
- extract `GpuCulled`
- extract skinned culling metadata

In the render world, store compact engine-owned records:

```rust
pub struct ExtractedGpuCullInstance {
    pub main_entity: Entity,
    pub proxy_index: u32,
    pub occludee_center: Vec3,
    pub occludee_half_extents: Vec3,
    pub flags: u32,
}

pub struct ExtractedOccluderProxy {
    pub main_entity: Entity,
    pub proxy_index: u32,
    pub proxy_center: Vec3,
    pub proxy_half_extents: Vec3,
}
```

Specific Bevy placement:

- main world: author/import bounds as normal components
- `ExtractSchedule`: copy bounds, transforms, visibility opt-ins, skinned metadata
- `RenderSystems::PrepareResources`: write `GpuCullBounds` and occluder candidate buffers
- `RenderSystems::PrepareBindGroups`: create HZB/culling bind groups
- render graph before `Node3d::StartMainPass`: run depth, HZB, and cull nodes

### Stage 2: Debug Before Draw Integration

The first working milestone should not change rendering output. It should produce:

- selected occluder count per view
- occludee count per view
- visible/culled counts
- debug draw for occludee boxes
- debug draw for selected occluder boxes
- optional color overlay by cull reason

This proves the Spartan-style policy before touching Bevy's draw path.

### Stage 3: Feed Bevy Where Possible

Bevy's GPU culling uses `MeshCullingData` from mesh asset AABBs. Our target is to replace that input with authored proxy visibility or proxy unions when hooks allow it. If `bevy_pbr` internals are too private, keep an engine-owned buffer first and integrate later.

### Stage 4: Occluder Proxy Depth

Bevy's built-in occlusion path builds HZB from depth prepass geometry. Our engine wants authored occluder proxies. There are two options.

Preferred first target:

- render `OccluderProxy` boxes into a dedicated depth texture
- build an engine-owned HZB from that texture
- use it for our own cull debug and custom draw path

Later tighter Bevy integration:

- add a proxy prepass phase before Bevy's GPU preprocess
- provide that depth pyramid to a modified or custom preprocess node

The clean graph location is before Bevy's main opaque/deferred work:

```text
AfterglowOccluderProxyDepth
AfterglowBuildHzb
AfterglowGpuCull
Bevy/Afterglow opaque or deferred drawing
```

In Bevy's `Core3d` graph, this belongs before `Node3d::StartMainPass`. If integrating with Bevy's two-phase path, it must line up around:

- `NodePbr::EarlyGpuPreprocess`
- `Node3d::EarlyPrepass`
- `Node3d::EarlyDownsampleDepth`
- `NodePbr::LateGpuPreprocess`
- `Node3d::StartMainPass`

### Stage 5: Draw Integration

Preferred long term: use Bevy's phase/indirect system so PBR materials, batching, skinning, lightmaps, and mesh allocation stay intact. Risk: `bevy_pbr` GPU preprocessing is not a stable public extension surface.

Prototype path: use one custom Afterglow opaque material lane. This is easier to debug and ships authored occluder proxies sooner, but it duplicates draw setup and starts with narrow material support.

Recommended sequence:

1. produce visibility flags without changing rendering
2. use flags in one custom opaque material path
3. validate bounds, HZB, counters, and skinned bounds
4. investigate Bevy preprocess integration for standard PBR
5. keep the custom path only if Bevy hooks are too private or too unstable

## Skinned Meshes

Skinned meshes must be first-party citizens, but they should not become expensive occluders by default.

### What Bevy Does Today

Bevy's skinning component is:

```rust
pub struct SkinnedMesh {
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    pub joints: Vec<Entity>,
}
```

During extraction, Bevy computes joint matrices, packs them into skin buffers, and shaders skin vertices from `current_skin_index`. The important culling problem:

- the mesh culling AABB is still the mesh/model-space AABB
- animation can move vertices outside the bind-pose bounds
- using stale bounds can incorrectly cull animated meshes

### Cheap First-Party Skinned Bounds

Use a tiered strategy:

1. Conservative authored `OccludeeProxy` set: safest fallback, always works, may be loose.
2. Per-clip baked bounds: union sampled animation frames; blended clips use union; procedural/IK falls back.
3. Per-joint spheres or capsules: offline, gather weighted vertices per joint, store a local influence volume, transform current joint volumes at runtime, then union to one world AABB.
4. GPU joint-bound reduction: later crowd optimization, not needed for the first implementation.

### Skinned Occluders

Default rule:

- skinned meshes are occludees, not occluders

Skinned occludee proxies may be bone-attached:

- torso proxy follows spine/chest
- head proxy follows head
- arm/weapon proxy follows hand or forearm
- large creature limb proxies follow their owning bones

The final object visibility is the OR of its occludee proxies. If any bone-attached occludee proxy is visible, draw the skinned mesh.

Allow skinned occluders only with explicit authoring:

- large creature torso proxy
- door-like animated rigid part
- predictable rigid equipment

For skinned occluder proxies, prefer bone-attached rigid proxy boxes:

```rust
pub struct BoneAttachedProxy {
    pub joint: Entity,
    pub kind: GpuOcclusionProxyKind,
    pub local_min: Vec3,
    pub local_max: Vec3,
}
```

That makes each proxy follow a single joint without rasterizing skinned proxy geometry.

## Data Model for Skinned Bounds

Asset-side data:

```rust
pub struct SkinnedCullAsset {
    pub fallback_local_bounds: Aabb,
    pub joint_spheres: Vec<JointCullSphere>,
    pub clip_bounds: Vec<AnimationClipBounds>,
}

pub struct JointCullSphere {
    pub joint_index: u16,
    pub center_in_joint_bind_space: Vec3,
    pub radius: f32,
}
```

Runtime data:

```rust
pub struct AnimatedOccludeeProxy {
    pub world_center: Vec3,
    pub world_half_extents: Vec3,
    pub proxy_index: u32,
    pub frame_index: u32,
}
```

Extraction rule:

- if static mesh: use authored or mesh AABB
- if skinned mesh:
  - transform authored bone-attached proxies when present
  - else use clip bounds when active animation state is known and simple
  - else use per-joint spheres
  - else use fallback authored proxies

## Render Graph Design

Spartan-shaped graph inside Bevy:

```text
ExtractSchedule:
  extract_gpu_cull_components
  extract_skinned_cull_inputs

RenderSystems::PrepareResources:
  prepare_gpu_cull_instance_buffers
  prepare_occluder_proxy_buffers
  prepare_skinned_bounds_buffers

RenderSystems::PrepareBindGroups:
  prepare_gpu_cull_bind_groups
  prepare_hzb_bind_groups

Core3d graph:
  AfterglowSelectOccluders
  AfterglowOccluderProxyDepth
  AfterglowBuildHzb
  AfterglowGpuAabbCull
  AfterglowBuildVisibleList
  Node3d::StartMainPass
```

If CPU-computed skinned bounds are used first, `AfterglowSkinnedBoundsCompute` is not needed.

## Practical Recommendation

Build in this order:

1. Components, import-time auto-fill, and debug draw for `OccludeeProxy` / `OccluderProxy`
2. Extract world-space occludee and occluder AABBs into render-world buffers
3. Implement Spartan-style occluder ranking: projected area, `64` cap, `1.5x` previous-frame bonus
4. Implement CPU-computed skinned occludee bounds from authored fallback bounds
5. Render selected occluder boxes into dedicated reverse-Z depth
6. Build always-fresh `R32Float` HZB with `min` downsample
7. Run conservative compute AABB tests and write visibility flags only
8. Add per-joint sphere asset data and CPU union for tighter skinned bounds
9. Use visibility flags in one custom opaque material path
10. Investigate replacing Bevy `MeshCullingData` with authored/animated bounds in the Bevy PBR preprocess path

This makes Spartan's AABB/HZB behavior the first priority, while Bevy integration remains incremental and reversible.

## Spartan Engine Case Study

Detailed box-only notes are in [spartan-engine-gpu-culling.md](spartan-engine-gpu-culling.md). This plan treats that document as the primary algorithm reference.

## Sources

- Bevy `ExtractSchedule` docs  
  https://docs.rs/bevy/latest/bevy/render/struct.ExtractSchedule.html

- Bevy render systems docs  
  https://docs.rs/bevy/latest/bevy/render/type.RenderSet.html

- Bevy `OcclusionCulling` docs  
  https://docs.rs/bevy/latest/bevy/render/experimental/occlusion_culling/struct.OcclusionCulling.html

- Bevy `SkinnedMesh` docs  
  https://docs.rs/bevy/latest/bevy/mesh/skinning/struct.SkinnedMesh.html

- Bevy custom skinned mesh example  
  https://bevy.org/examples/animation/custom-skinned-mesh/

- Bevy 0.16 release notes, GPU-driven rendering / occlusion culling  
  https://bevy.org/news/bevy-0-16/

- Local Bevy 0.18.1 source in Cargo registry:
  - `bevy_render-0.18.1/src/lib.rs`
  - `bevy_core_pipeline-0.18.1/src/core_3d/mod.rs`
  - `bevy_core_pipeline-0.18.1/src/experimental/mip_generation/mod.rs`
  - `bevy_pbr-0.18.1/src/render/gpu_preprocess.rs`
  - `bevy_pbr-0.18.1/src/render/mesh.rs`
  - `bevy_pbr-0.18.1/src/render/skin.rs`
  - `bevy_pbr-0.18.1/src/render/skinning.wgsl`
  - `bevy_pbr-0.18.1/src/render/mesh_preprocess.wgsl`
  - `bevy_pbr-0.18.1/src/render/occlusion_culling.wgsl`

- Spartan Engine source, inspected at commit `154e15bc71f6d16f8ed39932e6e1b1b9252f296f`  
  https://github.com/PanosK92/SpartanEngine/tree/154e15bc71f6d16f8ed39932e6e1b1b9252f296f
