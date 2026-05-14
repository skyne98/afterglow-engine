# Bevy Integration: Rendering Features

## Scope

This note maps the roadmap rendering features onto Bevy `0.18.1` and the current `afterglow-engine` code.

Current local state:

- [lib.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/lib.rs:1) registers demo-free runtime plugins plus perf HUD/tracing.
- [demo.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/demo.rs:1) installs the opt-in demo cell and demo cube/light animation.
- There is no engine render module yet. Rendering work should start as plugins under `src/rendering/` or narrower feature modules.

## Core Bevy Render Shape

Bevy rendering runs in a render sub-app:

- main-world ECS systems produce gameplay/render authoring data
- `ExtractSchedule` copies selected data into the render world
- `RenderSystems::PrepareResources` allocates/uploads GPU resources
- `RenderSystems::PrepareBindGroups` builds bind groups
- `Core3d` render graph nodes execute GPU passes

Important Bevy sources:

- `bevy_render-0.18.1/src/extract_component.rs`
- `bevy_render-0.18.1/src/render_graph/*`
- `bevy_render-0.18.1/src/render_resource/*`
- `bevy_core_pipeline-0.18.1/src/core_3d/mod.rs`
- `bevy_pbr-0.18.1/src/render/*`

Engine rule: gameplay components stay in the main world; compact render records and GPU resources live in the render world.

## Retro PBR

Start with Bevy PBR, not a renderer fork.

Best first integration:

```rust
ExtendedMaterial<StandardMaterial, RetroPbrExtension>
```

Useful Bevy sources:

- `bevy_pbr-0.18.1/src/material.rs`
- `bevy_pbr-0.18.1/src/extended_material.rs`
- `bevy_pbr-0.18.1/src/pbr_material.rs`
- `bevy_pbr-0.18.1/src/render/pbr.wgsl`
- `bevy_pbr-0.18.1/src/render/pbr_fragment.wgsl`

Afterglow extension data should hold:

- low-frequency diffuse mode
- vertex color/Gouraud diffuse weight
- lightmap/probe contribution weights
- material debug mode
- SPOM settings
- virtual texture handles
- specular gating controls

Use Bevy `StandardMaterial` for the base material contract, then add retro lighting constraints through extension shader code and material specialization.

## Deferred And Many Lights

Bevy already has deferred prepass support:

- `bevy_core_pipeline-0.18.1/src/prepass/mod.rs`
- `bevy_core_pipeline-0.18.1/src/core_3d/mod.rs`
- `bevy_pbr-0.18.1/src/deferred/mod.rs`

Implementation path:

1. Add an Afterglow camera preset that inserts `DeferredPrepass`, `DepthPrepass`, `MotionVectorPrepass` when needed.
2. Use Bevy `OpaqueRendererMethod` per material where possible.
3. Keep alpha/SPOM/decal/special materials forward or alpha-mask until tested.
4. Add debug counters for opaque deferred, opaque forward, alpha-mask, transparent.

Do not mix SPOM depth output with the main deferred path until HZB and G-buffer behavior are validated.

## Clustered Or Tiled Lighting

Use Bevy's existing light clustering first.

Useful Bevy sources:

- `bevy_light-0.18.1/src/cluster/mod.rs`
- `bevy_pbr-0.18.1/src/cluster.rs`
- `bevy_pbr-0.18.1/src/render/light.rs`
- `bevy_pbr-0.18.1/src/render/clustered_forward.wgsl`
- `bevy_light-0.18.1/src/point_light.rs`
- `bevy_light-0.18.1/src/spot_light.rs`

Afterglow work:

- author light budgets and debug UI over Bevy clusters
- expose cluster occupancy, visible light count, shadowed light count
- add gameplay tags for horror/stealth lights
- only replace the cluster path if deferred many-light goals require it

## AABB/Hi-Z Occlusion

Do not rely on Bevy's built-in `OcclusionCulling` for the engine plan.

Reason:

- it is experimental
- it requires `DepthPrepass`
- docs/source mark it incompatible with deferred
- it uses Bevy's mesh culling data, while Afterglow wants authored proxy sets

Useful Bevy sources:

- `bevy_render-0.18.1/src/experimental/occlusion_culling/mod.rs`
- `bevy_pbr-0.18.1/src/render/gpu_preprocess.rs`
- `bevy_pbr-0.18.1/src/render/mesh_preprocess.wgsl`
- `bevy_pbr-0.18.1/src/render/occlusion_culling.wgsl`
- `bevy_core_pipeline-0.18.1/src/experimental/mip_generation/mod.rs`

Afterglow path:

1. Author `OccludeeProxy` and `OccluderProxy` sets.
2. Extract proxy records into render world.
3. Rank occluder proxies by projected area and previous-frame bonus.
4. Render selected proxy boxes to dedicated reverse-Z occluder depth.
5. Build engine-owned `R32Float` HZB.
6. Run compute AABB tests.
7. Emit debug visibility flags before changing draw submission.
8. Later feed Bevy preprocess or custom indirect draw path.

Existing detailed notes:

- [gpu-driven-culling-bevy-integration.md](gpu-driven-culling-bevy-integration.md)
- [spartan-engine-gpu-culling.md](spartan-engine-gpu-culling.md)

## SPOM

Bevy has standard parallax concepts in PBR material/shader code, but not silhouette POM as an engine feature.

Useful Bevy sources:

- `bevy_pbr-0.18.1/src/pbr_material.rs`
- `bevy_pbr-0.18.1/src/render/parallax_mapping.wgsl`
- `bevy_pbr-0.18.1/src/material.rs`
- `bevy_pbr-0.18.1/src/extended_material.rs`

Afterglow path:

1. Add normal POM to `RetroPbrExtension`.
2. Add opt-in silhouette clipping.
3. Keep SPOM materials out of occluder depth unless explicitly represented by occluder proxies.
4. Add optional fragment depth output only after deferred and HZB interactions are tested.

Existing note:

- [silhouette-parallax-occlusion-mapping.md](silhouette-parallax-occlusion-mapping.md)

## Software Virtual Texturing

Bevy has normal `Image`, texture, sampler, asset, and GPU texture upload systems, but no virtual texturing system.

Useful Bevy sources:

- `bevy_image-0.18.1/src/image.rs`
- `bevy_render-0.18.1/src/texture/gpu_image.rs`
- `bevy_render-0.18.1/src/texture/texture_cache.rs`
- `bevy_render-0.18.1/src/render_resource/texture.rs`
- `bevy_render-0.18.1/src/gpu_readback.rs`
- `bevy_asset-0.18.1/src/server/mod.rs`

Afterglow path:

1. Define `VirtualTextureAsset` and page metadata.
2. Create physical cache textures in render world.
3. Render low-resolution feedback pass.
4. Read feedback with frame latency.
5. Stream pages through task pools/asset IO.
6. Upload bounded page count in `PrepareResources`.
7. Update page tables before material draws.
8. Sample VT pages in `RetroPbrExtension`.

Existing note:

- [software-virtual-texturing.md](software-virtual-texturing.md)

## Fog

Use Bevy's fog first.

Useful Bevy sources:

- `bevy_pbr-0.18.1/src/fog.rs`
- `bevy_pbr-0.18.1/src/render/fog.rs`
- `bevy_pbr-0.18.1/src/render/fog.wgsl`
- `bevy_pbr-0.18.1/src/volumetric_fog/mod.rs`
- `bevy_light-0.18.1/src/volumetric.rs`

Afterglow should wrap this with authoring components:

- `FogVolume`
- `HorrorFogPreset`
- `StealthVisibilityFog`
- `ChunkFogRegion`

Only build a custom froxel fog path after Bevy's built-in components fail a concrete requirement.

## DDGI And Probes

Bevy has static/light-probe pieces but not full dynamic DDGI.

Useful Bevy sources:

- `bevy_light-0.18.1/src/probe.rs`
- `bevy_pbr-0.18.1/src/light_probe/mod.rs`
- `bevy_pbr-0.18.1/src/light_probe/generate.rs`
- `bevy_pbr-0.18.1/src/lightmap/mod.rs`

Afterglow path:

1. Represent chunk probe data with Bevy `LightProbe` / `IrradianceVolume` where possible.
2. Add chunk load/unload of probe volumes.
3. Blend probes across chunk boundaries.
4. Add DDGI update logic later when render infrastructure is mature.

Existing note:

- [ddgi-global-illumination.md](ddgi-global-illumination.md)

## Implementation Order

1. `RetroPbrExtension` over `StandardMaterial`.
2. Deferred camera/material toggle.
3. Bevy clustered-light debug overlay.
4. Bevy fog authoring wrapper.
5. Engine-owned proxy/HZB debug-only path.
6. SPOM material mode.
7. VT feedback/cache/page-table prototype.
8. Probe/lightmap chunk residency.
9. DDGI update experiments.
