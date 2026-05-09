# Gouraud Shading Implementation Plan

## Goal

Call Bevy's **existing PBR functions** (`pbr_input_from_world`, `pbr_lighting`, etc.) in the **vertex shader**, then interpolate the result in the fragment shader.

No custom lighting code. No baking. No CPU path. Just move the lighting evaluation from per-pixel to per-vertex.

## Approach

Create a `GouraudMaterial` that:
- **Vertex shader**: imports `bevy_pbr::pbr_functions`, calls `pbr_input_from_world()` with vertex-level data, calls `pbr_lighting()`, outputs the lit color + UV
- **Fragment shader**: samples base color texture, multiplies by interpolated color, outputs

Bevy's PBR functions handle everything — cluster lights, shadows, ambient, directional, fog. They just run at vertex frequency instead of pixel frequency.

## Key Bevy Shader APIs

These are the functions from Bevy 0.18's `bevy_pbr` shader module that we'll call in the vertex shader:

```wgsl
#import bevy_pbr::pbr_functions::{pbr_input_from_world, pbr_lighting}
#import bevy_pbr::pbr_bindings
#import bevy_pbr::pbr_types::{PbrInput, PbrOutput}
```

- `pbr_input_from_world(world_pos, world_normal, ...)` → `PbrInput`
- `pbr_lighting(pbr_input)` → `PbrOutput` (contains `frag_color`)
- `PbrInput` has fields for position, normal, UV, material properties, view direction, etc.

## Vertex Shader (`gouraud_vertex.wgsl`)

```wgsl
#import bevy_pbr::pbr_functions::{pbr_input_from_world, pbr_lighting}
#import bevy_pbr::pbr_bindings
#import bevy_pbr::pbr_types::{PbrInput, PbrOutput}
#import bevy_pbr::mesh_view_bindings
#import bevy_pbr::mesh_bindings
#import bevy_pbr::mesh_functions::{mesh_position_local_to_world, mesh_position_world_to_clip}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    let world_pos = mesh_position_local_to_world(mesh.model, vec4(in.position, 1.0));
    let world_normal = normalize(mat3x3<f32>(mesh.model) * in.normal);

    // Build PBR input from vertex data — same function Bevy uses per-pixel
    let pbr_input = pbr_input_from_world(
        world_pos.xyz,
        world_normal,
        world_pos.xyz,      // view direction computed from vertex position
        in.uv,
        mesh.model,
    );

    // Evaluate full PBR lighting at this vertex
    let pbr_output = pbr_lighting(pbr_input);

    return VertexOutput(
        mesh_position_world_to_clip(world_pos),
        pbr_output.frag_color,
        in.uv,
    );
}
```

## Fragment Shader (`gouraud_fragment.wgsl`)

```wgsl
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

The color is fully computed per-vertex and interpolated across the triangle — pure Gouraud.

## What's Preserved

| Feature | How |
|---|---|
| Cluster lights | Bevy's `pbr_lighting` iterates them |
| Directional / point / spot | All handled by `pbr_lighting` |
| Shadow maps | Sampled in vertex shader via PBR input |
| Ambient light | From scene, included in `pbr_lighting` |
| Fog | Computed per-vertex via PBR functions |
| Base color | Sampled per-pixel in fragment shader... |

Actually, for base color texture: we need to sample it either per-vertex (cheaper, lower quality) or per-pixel (standard). Let's sample it per-pixel in the fragment shader for now.

## Updated Fragment Shader

```wgsl
@group(2) @binding(0) var base_color_texture: texture_2d<f32>;
@group(2) @binding(1) var base_color_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(base_color_texture, base_color_sampler, in.uv);
    return vec4(in.color.rgb * tex_color.rgb, in.color.a * tex_color.a);
}
```

## Files

| File | Purpose |
|---|---|
| `crates/afterglow-engine/src/material.rs` | `GouraudMaterial` struct + `Material` trait impl |
| `assets/shaders/gouraud_vertex.wgsl` | Vertex shader calling `pbr_input_from_world` + `pbr_lighting` |
| `assets/shaders/gouraud_fragment.wgsl` | Fragment shader with texture × interpolated color |

## Registering

Add `app.add_plugins(MaterialPlugin::<GouraudMaterial>::default())` and use `GouraudMaterial` in `setup.rs` instead of `StandardMaterial`.
