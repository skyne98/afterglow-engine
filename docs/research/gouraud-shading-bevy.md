# Gouraud Shading in Bevy 0.18

## What It Is

Gouraud shading computes lighting at triangle **vertices** (using per-vertex normals) and
linearly interpolates the resulting color across each triangle in the fragment shader.

Contrast with **Phong shading**, which interpolates normals and computes lighting per-pixel.

## Quality Considerations

| Issue | Cause | Mitigation |
|---|---|---|
| **Mach bands** | Linear interpolation creates derivative discontinuities at triangle edges | Use higher tessellation, or dither the color gradient |
| **Missing specular highlights** | Highlight falls in triangle interior, never reaches a vertex | Subdivide mesh, add vertex near highlight, or use tessellation |
| **Highlight pulsing** | Highlight intensity peaks when it aligns with a vertex, fades between | Subdivide evenly, or accept as a stylistic choice |
| **Perspective distortion** | Naive linear interpolation in screen space is wrong for perspective | GPUs do **perspective-correct interpolation** (hyperbolic) automatically for all varyings — bevy/wgpu uses this by default |

"High-quality Gouraud" means:
- Well-distributed vertex normals (properly averaged from neighboring faces)
- Sufficient tessellation so highlights don't miss triangles
- Perspective-correct interpolation (automatic in wgpu)
- Optional: smoothstep or dither to hide Mach bands
- Optional: specular cutoff threshold to avoid weak spread highlights

## Implementation Approaches in Bevy 0.18

### Approach 1: Custom `Material` with vertex shader

The cleanest path. Create a new type implementing `bevy::pbr::Material`, provide a
custom vertex shader that does the lighting computation at the vertex level.

**Vertex shader** (`gouraud.wgsl`):
```
#import bevy_pbr::mesh_functions  (mesh_position_local_to_world, etc.)
#import bevy_pbr::lighting        (point_light, directional_light, etc.)
#import bevy_pbr::mesh_bindings   (mesh)
#import bevy_pbr::view_bindings   (view)

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

// TODO see the specifics in a custom uv normal approach
// The point is compute final shaded color in vertex shader,
// pass as @location(0) color, and use it in fragment shader directly.
```

**Fragment shader**:
```
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

### Approach 2: `ExtendedMaterial` wrapping StandardMaterial

For rapid prototyping: use `ExtendedMaterial<StandardMaterial, MyData>` and override
the vertex shader. Less flexible but gets you PBR lighting for free.

### Approach 3: Custom `MaterialPipeline` / `SpecializedMeshPipeline`

Full control but requires more boilerplate (pipeline specialization, extraction, batching).

## Key Bevy APIs

- `bevy::pbr::Material` trait — implement for custom materials
- `bevy::pbr::ExtendedMaterial` — extend StandardMaterial with custom data
- `Mesh::ATTRIBUTE_POSITION`, `ATTRIBUTE_NORMAL`, `ATTRIBUTE_UV_0` — built-in vertex attributes
- `bevy_pbr::mesh_bindings` — shader import for mesh uniforms
- `bevy_pbr::view_bindings` — shader import for view uniforms
- `bevy_pbr::lighting` — shader import for light structs and compute functions

## Lighting to Compute at Vertex Level

The full PBR equation is expensive per-pixel; for Gouraud we evaluate at vertices:

1. **Ambient** — constant term
2. **Diffuse (Lambertian)** — `max(dot(N, L), 0.0)` per light
3. **Specular (Blinn-Phong)** — `pow(max(dot(N, H), 0.0), shininess)` — this is where
   artifacts show on low-poly meshes
4. **Distance attenuation** — for point lights

Bevy's `bevy_pbr::lighting` shader module provides `point_light`, `directional_light`
functions that compute these — but they are designed for per-pixel use. For Gouraud,
we'd call them once per vertex and pass the result as an interpolated color.

## Performance

- **Gouraud**: N lights × M vertices lighting computations per frame
- **Phong (default Bevy PBR)**: N lights × screen-pixels lighting computations

For low-poly scenes Gouraud is **significantly cheaper**. For high-poly scenes the
difference narrows. On an integrated GPU where pixel fill rate is the bottleneck,
vertex shading can be a net win.

## Future Directions

- Implement as `GouraudMaterial` plugin with a toggle
- Support mixed scene: Gouraud for far/cheap objects, Phong for near/hero
- Investigate compute-shader-based vertex lighting for skinned meshes
