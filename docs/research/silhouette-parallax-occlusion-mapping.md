# Silhouette Parallax Occlusion Mapping

## Scope

This note covers silhouette-capable parallax occlusion mapping for `afterglow-engine`.

Terms are inconsistent in engines and papers:

- POM: ray-marched height-field intersection in tangent space
- silhouette POM / SPOM: POM that also affects the apparent mesh outline
- relief mapping: closely related ray-height-field method, often with depth writes
- cone step / relaxed cone step mapping: accelerated ray-height-field intersection
- Prism POM: a more exact silhouette method using extruded prisms

The useful goal for this engine is a practical retro-PBR material feature for bricks, carved stone, panels, grates, roots, bone, ornate metal, dungeon floors, and horror set dressing.

## Baseline POM

Classic POM treats each rendered triangle as a portal into a tangent-space height field.

Shader outline:

1. transform view vector into tangent space
2. march along the view ray through the height map
3. find the ray-height-field intersection
4. refine the hit with linear interpolation or binary search
5. use the displaced UV to sample albedo, normal, ORM, emissive, etc.
6. optionally march toward the light for self-shadowing

This changes texture lookup, not the mesh. Standard POM therefore fails at the original polygon silhouette: the surface appears deep in the middle but still cuts off at the flat triangle edge.

## What "Silhouette" Can Mean

There are three levels of silhouette support.

### 1. UV-Out-Of-Bounds Clipping

The simplest SPOM approach:

1. run POM
2. compute displaced UV
3. discard/alpha-clip if the displaced UV leaves the material tile or a silhouette mask

This is what many practical engine/user shaders mean by silhouette clipping. It is easy and works best for flat cards, trim sheets, blocky surfaces, and high-contrast height fields.

Limitations:

- it only removes pixels from the original polygon
- it cannot create pixels outside the original triangle
- it needs alpha clipping, which hurts early-Z and can interact badly with deferred paths
- it is view-dependent and can sparkle at grazing angles

This is still the best first implementation.

### 2. Depth-Correct Relief/POM

Relief mapping and some POM implementations compute the 3D hit point and write an adjusted pixel depth. GPU Gems 3 notes that updating the z-buffer with the projected hit point gives proper occlusion against other scene objects and supports shadow-map composition.

This improves intersections with other geometry, but it still cannot rasterize new pixels outside the original triangle. It should be viewed as depth correction, not full geometric silhouette expansion.

In Bevy/wgpu this means using fragment depth output. That must be optional because it can affect early-Z, depth prepass assumptions, and deferred rendering performance.

### 3. Prism POM

Prism Parallax Occlusion Mapping with Accurate Silhouette Generation is the most relevant academic answer to correct silhouettes.

The idea:

- extrude the base triangle into a prism along its normal/displacement range
- split the prism into tetrahedra
- ray-intersect the prism volume and height field
- generate fragments for the displaced silhouette more accurately than plain POM

This solves the missing-outside-triangle problem more directly, but it is much more complex than shader-only POM. It changes the draw representation and is not the right first step for Afterglow.

## Acceleration Options

### Linear Search + Binary Refinement

Basic robust POM:

- linear march finds the first interval where the ray crosses the height field
- binary search refines the intersection

Use adaptive step counts:

```text
steps = mix(min_steps, max_steps, 1 - abs(dot(normal, view)))
```

More steps at grazing angles, fewer when looking straight at the surface.

### Dynamic POM

Tatarchuk's dynamic POM work emphasizes:

- tangent-space view/light rays
- high-precision height-field intersection
- dynamic sample count based on viewing angle
- LOD fade based on mip level or distance
- optional approximate soft self-shadows

This is a good practical baseline for game materials.

### Cone Step Mapping

Cone step mapping precomputes extra data per texel so the shader can skip empty height-field space faster.

Conservative cone stepping avoids missing intersections but can stop early and distort the result. Relaxed cone stepping allows the ray to pierce the surface at most once, then uses binary search to refine. GPU Gems 3 reports much better quality for the same rough iteration budget.

Cost:

- needs a precomputed cone map
- cone map cannot use normal mip filtering casually
- authoring/import pipeline becomes more complex

This is a later quality/perf option, not MVP.

## Self-Shadowing

POM self-shadowing marches a second ray from the surface hit point toward the light.

Use it carefully:

- useful for close-up hero materials
- expensive with many lights
- does not naturally integrate with shadow maps for displaced silhouettes
- can be replaced by AO/cavity maps for most retro-PBR materials

For Afterglow, prefer:

1. normal POM without self-shadowing
2. material AO/cavity and screen-space contact shadowing
3. optional one-light self-shadowing for hero surfaces

## Deferred Rendering Concerns

POM works in deferred shading if the material pass writes the final material attributes using displaced UVs.

Hard parts:

- alpha/silhouette clipping must happen in the G-buffer pass
- optional pixel depth offset must update G-buffer depth
- motion vectors become approximate unless displaced hit depth is accounted for
- shadow caster pass will not match the displaced silhouette unless it also runs compatible logic
- decals and SSR/SSGI may see the adjusted depth but not true geometry

For the first implementation, treat SPOM as a material-local visual detail and do not promise physically perfect shadow casting.

## Bevy / wgpu Fit

Bevy has normal material shaders and custom shader extension paths. It does not provide built-in SPOM.

Implementation points:

- add material flags for `ParallaxMode`
- add height map channel to retro-PBR material
- run POM in the material fragment shader before sampling other textures
- use `textureSampleGrad` or explicit derivatives so mips remain stable after UV displacement
- support alpha clipping for UV-out-of-bounds SPOM
- add optional fragment depth output only after the base path is stable

Suggested API:

```rust
pub enum ParallaxMode {
    Off,
    Offset,
    Occlusion,
    SilhouetteClip,
}

pub struct ParallaxSettings {
    pub height_scale: f32,
    pub min_steps: u32,
    pub max_steps: u32,
    pub silhouette_clip: bool,
    pub depth_write: bool,
    pub fade_distance: f32,
}
```

## Practical Afterglow Plan

### Phase 1: Standard POM

- height map in material
- tangent-space view vector
- adaptive linear search
- small binary refinement
- displaced UV used for all PBR channels
- distance/mip fadeout
- debug view for step count and displaced UV

Target materials:

- stone floors
- bricks
- carved trim
- metal panels
- wood planks

### Phase 2: Silhouette Clip

- discard when displaced UV leaves tile bounds
- optional author-provided silhouette mask
- alpha-test pipeline variant
- strong warning/debug overlay for expensive surfaces

Use only on:

- cards
- trims
- grates
- broken edges
- close-up wall/floor features

Avoid on:

- large screen-covering terrain
- thin animated objects
- foliage-like dense alpha areas

### Phase 3: Depth Offset

- compute displaced world/view position
- output fragment depth
- validate with deferred G-buffer
- validate with HZB/occlusion and screen-space effects

This is where interaction with our GPU culling/deferred path needs careful testing.

### Phase 4: Accelerated Or Accurate Variants

Only after the base feature proves useful:

- relaxed cone stepping for heavy hero materials
- per-material cone map import
- Prism POM research prototype for select geometry

## Authoring Rules

Good SPOM needs better height maps than normal parallax.

Rules:

- height should be monotonic enough for stable ray marching
- avoid noisy high-frequency height at large amplitude
- keep amplitude small for most materials
- fade out by distance or mip level
- use silhouette clipping only where losing pixels at edges makes visual sense
- provide non-parallax fallback for low quality settings

Recommended defaults:

```text
height_scale: 0.02 - 0.06 meters
min_steps: 6 - 8
max_steps: 24 - 48
binary_refinement_steps: 3 - 5
fade_start: material dependent
fade_end: before aliasing dominates
```

## Recommendation

Do not start with Prism POM.

Start with a robust, boring POM implementation and add UV-out-of-bounds silhouette clipping as an opt-in material mode. Then add optional depth output for close-up surfaces after deferred rendering and HZB culling are stable.

For Afterglow's retro immersive-sim look, SPOM is best as a controlled hero-material feature:

- dungeon stone relief
- engraved magical surfaces
- vents and grates
- carved doors
- wet cobbles
- metal panels

It should not become the default surface detail method. Normal maps, vertex color/Gouraud diffuse, lightmaps/probes, and cheap geometry still carry the main visual language.

## Sources

- CRYENGINE documentation, "Silhouette POM"  
  https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1048726

- Dachsbacher and Tatarchuk, "Prism Parallax Occlusion Mapping with Accurate Silhouette Generation", I3D 2007  
  https://citeseerx.ist.psu.edu/document?doi=d78ec8a2b13e333e3df2576655eca62fd4e35e60&repid=rep1&type=pdf

- Tatarchuk, "Dynamic Parallax Occlusion Mapping with Approximate Soft Shadows", I3D 2006  
  https://citeseerx.ist.psu.edu/document?doi=59d442bc81055bdf4a9e4a1806e2c2b982e93489&repid=rep1&type=pdf

- Tatarchuk, "Practical Dynamic Parallax Occlusion Mapping"  
  https://www.gamedevs.org/uploads/practical-dynamic-parallax-occlusion-mapping.pdf

- Policarpo, Oliveira, and Comba, "Real-Time Relief Mapping on Arbitrary Polygonal Surfaces", I3D 2005  
  https://www.inf.ufrgs.br/~comba/papers/2005/tog-2005.pdf

- Policarpo and Oliveira, "Relief Mapping of Non-Height-Field Surface Details", I3D 2006  
  https://www.inf.ufrgs.br/~oliveira/pubs_files/Policarpo_Oliveira_RTM_multilayer_I3D2006.pdf

- GPU Gems 3, Chapter 18, "Relaxed Cone Stepping for Relief Mapping"  
  https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-18-relaxed-cone-stepping-relief-mapping

- Unity Shader Graph documentation, "Parallax Occlusion Mapping Node"  
  https://docs.unity.cn/Packages/com.unity.shadergraph%4010.2/manual/Parallax-Occlusion-Mapping-Node.html

- Valve Developer Community, "Parallax mapping"  
  https://developer.valvesoftware.com/wiki/Parallax_mapping
