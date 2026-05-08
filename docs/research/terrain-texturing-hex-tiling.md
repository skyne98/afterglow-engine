# Terrain Texturing & Hex Tiling in Bevy 0.18

## 1. Mesh Blending / Terrain Texturing

### Built-in Support: None

Bevy 0.18 has **no built-in terrain blending** — no splat map material, no triplanar mapping,
no terrain shader. It provides the plumbing via `ExtendedMaterial` and custom shaders.

### Approaches

#### ExtendedMaterial<StandardMaterial, E>

Wrap StandardMaterial with a custom extension that adds blend maps:

```rust
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct TerrainExtension {
    #[texture(100)]
    #[sampler(101)]
    blend_map: Handle<Image>,          // RGBA: R=layer0, G=layer1, B=layer2, A=layer3
    #[texture(102)]
    #[sampler(103)]
    layer0: Handle<Image>,
    #[texture(104)]
    #[sampler(105)]
    layer1: Handle<Image>,
    #[texture(106)]
    #[sampler(107)]
    layer2: Handle<Image>,
    #[texture(108)]
    #[sampler(109)]
    layer3: Handle<Image>,
}

impl MaterialExtension for TerrainExtension {
    fn fragment_shader() -> ShaderRef { "shaders/terrain_blend.wgsl".into() }
}
```

In WGSL:
```wgsl
@group(3) @binding(100) var blend_map: texture_2d<f32>;
@group(3) @binding(101) var blend_sampler: sampler;
@group(3) @binding(102) var layer0: texture_2d<f32>;
// ... etc

fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let blend = textureSample(blend_map, blend_sampler, in.uv);
    let c0 = textureSample(layer0, blend_sampler, in.uv);
    let c1 = textureSample(layer1, blend_sampler, in.uv);
    let c2 = textureSample(layer2, blend_sampler, in.uv);
    let c3 = textureSample(layer3, blend_sampler, in.uv);
    return c0 * blend.r + c1 * blend.g + c2 * blend.b + c3 * blend.a;
}
```

#### Splat Maps (Texture Arrays)

For >4 layers, use texture arrays indexed by blend channel:

```
Blend map → decoded per-pixel → index into texture array
```

#### Height-Based Blending

Improve splat maps by also sampling a height per layer:
```
weight = blend_channel * (height - max(heights_except_this, 0)) + bias
weight = max(weight, 0) / (sum_of_weights + epsilon)
```

#### Triplanar Mapping

Project texture from 3 axes, blend by surface normal:
```
weight_i = abs(dot(normal, axis_i))^sharpness
weight_i /= (weight_x + weight_y + weight_z)
```

### Crates

| Crate | Approach |
|---|---|
| **plumesplat** | `ExtendedMaterial` + texture arrays (256 layers), triplanar, height blending, stochastic tiling, full PBR |
| **bevy_regions** | u16 biome ID texture, CPU-based region painting |
| **saddle-rendering-stochastic-texturing** | Anti-repetition: hex tiling, texture bombing, triplanar |

## 2. Hex Tiling

### What It Is

Hex tiling breaks visible rectangular repetition patterns by placing texture tiles
on a hexagonal grid and blending between neighbors. Instead of one repeating square
grid, each fragment blends samples from 3 adjacent hexagonal tiles with random
offsets and rotations.

### Algorithm

1. **Skew UV to hexagonal coordinates:**
   ```
   u_skew = u - v / sqrt(3)
   v_skew = v * 2 / sqrt(3)
   grid_x = floor(u_skew + 0.5)
   grid_y = floor(v_skew + 0.5)
   ```

2. Find the **3 nearest hex centers** in barycentric space (always the same 3 neighbors
   of the containing hexagon)

3. For each hex center, compute a **per-tile random offset + rotation** via a hash
   function of the integer tile coordinates:
   ```
   hash = hash2(grid_x, grid_y)  // pseudo-random, deterministic
   offset = hash.xy * some_scale
   rotation = hash.z * 2π
   ```

4. **Sample the texture** at the perturbed UV for each tile:
   ```wgsl
   let sampled_uv = barycentric_uv + offset;
   // Apply rotation matrix
   let rotated = rotate(sampled_uv, rotation);
   ```

5. **Blend** the 3 samples by their barycentric weights, with optional contrast
   correction to avoid blurring:
   ```
   result = sum(w_i * sample_i) / sum(w_i)
   // Contrast correction:
   result = pow(result, 1.0 + contrast)  // where contrast ~ 0.3-0.5
   ```

### WGSL Implementation (Core)

```wgsl
fn hex_tiling(uv: vec2<f32>, tex: texture_2d<f32>, samp: sampler) -> vec4<f32> {
    let skew = vec2(uv.x - uv.y * 0.57735027, uv.y * 1.1547005);
    let grid = floor(skew + 0.5);
    let local = skew - grid;

    // Barycentric coordinates for the 3 nearest hex centers
    let w1 = local.x;
    let w2 = local.y;
    let w3 = 1.0 - w1 - w2;

    // Determine which 3 hexagons
    let hex_offsets = // ... based on which barycentric region we're in

    // Per-tile hash for offset + rotation
    var color = vec4(0.0);
    var total_w = 0.0;
    for (var i = 0; i < 3; i++) {
        let tile_coord = grid + hex_offsets[i];
        let hash = hash3(tile_coord);
        let offset = hash.xy * vec2(0.3, 0.3);
        let angle = hash.z * 6.28318;
        let sampled_uv = rotate(uv + offset, angle);
        let s = textureSampleGrad(tex, samp, sampled_uv, ddx(uv), ddy(uv));
        let w = hex_weights[i];
        color += s * w;
        total_w += w;
    }
    return color / total_w;
}
```

### Performance

- **3 texture samples per layer** vs 1 for standard tiling (~3× texture bandwidth)
- Use `textureSampleGrad` with explicit derivatives for correct mipmapping
- Quality tiers:
  - **Fast**: Skip lowest-weight tile (2 samples)
  - **Balanced**: Threshold cull when weight < 0.05
  - **HighQuality**: Full 3 samples
- Height-aware: weight tiles by height similarity to avoid blending across seams

### Existing Implementations

| Source | Format | Notes |
|---|---|---|
| **saddle-rendering-stochastic-texturing** | Rust/Bevy/WGSL | Full hex tiling + triplanar + height blending |
| **three-hex-tiling** | Three.js/GLSL | Reference impl with configurable params |
| **Neyret's Shadertoy** | GLSL | Original algorithm |

## References

- https://crates.io/crates/plumesplat
- https://crates.io/crates/bevy_regions
- saddle-rendering-stochastic-texturing (GitHub)
- https://www.shadertoy.com/view/4t2XWh (Neyret's original hex tiling)
- https://iquilezles.org/articles/hexagons/ (hex grid math)
