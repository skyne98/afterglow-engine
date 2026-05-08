# DDGI (Dynamic Diffuse Global Illumination) — Comprehensive Research

## What Is DDGI

DDGI is a real-time global illumination technique that extends traditional pre-computed
irradiance probes by using **GPU ray tracing** to dynamically update probe data every frame.
It stores both **irradiance** and **mean distance** per probe, enabling a statistical occlusion
test (Chebyshev inequality) that prevents light leaking through walls.

## Core Algorithm Flow

```
1. Probe grid placement → 3D uniform grid of probes within a volume
2. Each frame, trace N rays from each probe into the scene
3. Accumulate incoming radiance (color) and hit distance
4. Blend new data into probe textures (temporal hysteresis)
5. At shading: trilinear interpolation of 8 nearest probes
   → Chebyshev occlusion test using stored distance
   → Weight probes behind occluders to near-zero
```

### Key Innovation

By storing both **irradiance AND distance**, DDGI can detect occluded probes and prevent
"light leaking" — the classic problem with traditional irradiance probes.

## Probe Grid Architecture

- **Uniform 3D grid** within an axis-aligned bounding volume
- **Probe spacing**: 0.5-4m (finer = higher quality, more memory)
- **Probe states**: ACTIVE / INACTIVE (inside geometry or no nearby geo)
- **Classification**: probes inside geometry get marked inactive, traced rays still fire
  to allow them to relocate out of geometry
- **Relocation**: probes can shift up to 45% of grid cell size to avoid walls

## Ray Tracing

### Ray Count Quality Tiers

| Tier | Rays | Use Case |
|---|---|---|
| Low | 8 | Mobile |
| Medium | 16-32 | Console |
| High | 64-128 | PC |
| Ultra | 256+ | Reference |

### Ray Distribution
- **Spherical Fibonacci** (default in RTXGI) — well-distributed points, rotated each frame
  by random 3×3 rotation matrix (Arvo's method)
- A fixed subset of 32 rays are NOT rotated — used for relocation + classification only

## Probe Texture Storage (Octahedral Mapping)

Each probe stores data in 6 texture arrays using octahedral mapping:

| Texture | Channels | Format | Purpose |
|---|---|---|---|
| Ray Data | RG | F16×2 | Raw ray tracing output |
| Irradiance | RGB | R11G11B10F | Accumulated irradiance per direction |
| Distance | RG | F16×2 | Mean distance + variance per direction |
| Probe Data | XYZW | F32×4 | Relocation offset + state |
| Variability | R | F32 | Coefficient of variation per texel |
| Variability Avg | RG | F32×2 | Reduced average variability |

### Irradiance Encoding
Exponential (γ) encoding: `stored = pow(linear, 1/γ)` with default γ=5.
Improves light-to-dark convergence, allows smaller texture formats.

## The Per-Frame Update Loop

```
1. Compute new random rotation matrix (Arvo method)
2. Compute scroll offsets (if infinite scrolling)

3. Trace Probe Rays (application-owned)
   Dispatch: (numRays, numProbes) in 2D grid
   For each ray: compute probe pos + ray direction, trace, store radiance + distance

4. Blend Probes (SDK: ProbeBlendingCS)
   BlendIrradiance: lerp(old, new, blendFactor) over octahedral texels
   BlendDistance: lerp(old, new, blendFactor)

5. Relocate Probes (optional, SDK: ProbeRelocationCS)
   Move probes out of geometry based on backface hit ratio

6. Classify Probes (optional, SDK: ProbeClassificationCS)
   Mark ACTIVE / INACTIVE

7. Compute Variability (optional, SDK: ReductionCS)
   Track variance across frames for power management

8. Query Irradiance (application-owned)
   For each pixel: 8 nearest probes → trilinear → Chebyshev occlusion test
```

## The Chebyshev Occlusion Test

```hlsl
float variance = abs((mean²) - mean_of_squares);
float chebyshevWeight = 1.0;
if (shadingDist > meanDist) {
    float v = shadingDist - meanDist;
    chebyshevWeight = variance / (variance + v²);
    chebyshevWeight = max(chebyshevWeight³, 0.0);
}
weight *= max(0.05, chebyshevWeight);
```

This prevents light from leaking through walls: if the shading point is farther
from a probe than the mean distance stored for that direction, it's likely
occluded and the weight is crushed.

## Temporal Accumulation

`probe = lerp(probe, newData, blendFactor)`
- Typical blend factor: 1/8 to 1/16
- Convergence: 8-16 frames
- Restart on: probe relocation, new activation, scene lighting changes

On dynamic objects: fast-moving objects leave a ghost trail. Solutions include
multi-rate updates or dynamic-region detection with higher blend factors.

## Modern Improvements

### DDGI2 (Wyman 2020, Digital Dragons)
- 2nd order SH (4 coefficients per color) as an alternative to octahedral maps
- Improved temporal accumulation
- Tighter Chebyshev test
- Shared memory optimizations in blending

### Infinite Bounce
Probe rays can sample other probes at hit points, approximating multi-bounce GI.

### Cascaded Probe Grids
Not in RTXGI SDK 1.x, but multiple DDGIVolumes with different spacings approximate it.

## DDGI vs Other GI Techniques

| Technique | Dynamic | Quality | Cost | Complexity |
|---|---|---|---|---|
| **Lightmaps (baked)** | Static only | Very high | Free | High |
| **DDGI (probes)** | Fully dynamic | Medium-low | Medium | Medium |
| **RTXGI SHaRC** | Fully dynamic | High | High | High |
| **Voxel GI** | Mostly dynamic | Medium | Medium-high | High |
| **Lumen (UE5)** | Fully dynamic | High | High | Very high |
| **SSGI** | Fully dynamic | Low | Low | Low |

DDGI's strength: balance of fully dynamic GI with predictable performance.

## Implementations

### NVIDIA RTXGI SDK (Reference)
- https://github.com/NVIDIAGameWorks/RTXGI-DDGI
- C++/HLSL, D3D12 + Vulkan
- Provides: probe blending, relocation, classification, variability, irradiance gather
- Application owns: ray tracing dispatch, acceleration structures, screen-space gather

### Other Open Source

| Repository | Stars | Language |
|---|---|---|
| NVIDIAGameWorks/RTXGI-DDGI | 834 | C++/HLSL |
| mateeeeeee/Adria | 541 | C++ |
| tippesi/Atlas-Engine | 470 | C++ |
| flwmxd/LuxGI | 338 | C++ (hybrid DDGI + SDF) |
| helenl9098/DDGI-Minecraft | 91 | C++/Vulkan |
| SanYue-TechArt/RTXGI-DDGI-URP | 84 | C#/Unity |
| xuechao-chen/DDGI | 69 | C++ |
| guoxx/DDGI | 26 | C++ |

### Rust/WGPU/Bevy
**None exist.** WGPU's ray tracing support is experimental (wgpu-hal + naga).
DDGI in Rust would require either:
- DXR/Vulkan RT shaders in HLSL/GLSL compiled externally
- Raw Vulkan/D3D12 bindings from Rust
- Waiting for WGPU to mature its ray tracing pipeline

## Presentations & Talks

| Talk | Speaker | Event | Year |
|---|---|---|---|
| DDGI with Ray-Traced Irradiance Fields | Majercik et al. | SIGGRAPH | 2019 |
| Ray-Traced Irradiance Fields | Morgan McGuire | GDC | 2019 |
| DDGI2: Improved DDGI | Chris Wyman | Digital Dragons | 2020 |
| Scaling Probe-Based RT DDGI for Production | Majercik et al. | JCGT + I3D | 2021 |
| DDGI in The Callisto Protocol | Striking Distance | Dev talks | 2022-23 |
| DDGI in Filament | Google | Docs | 2020-23 |
| Radiance Caching (Lumen) | Daniel Wright (Epic) | SIGGRAPH Advances | 2021 |
| Large-Scale GI at Activision | Ari Silvennoinen | SIGGRAPH Advances | 2021 |
| Global Illumination Based on Surfels | EA SEED/Frostbite | SIGGRAPH Advances | 2021 |

## Key Technical Details

### SH Order for DDGI
| Approach | Memory/probe | Quality |
|---|---|---|
| Octahedral 6×6 | 576 B | Very high (36 directions) |
| Octahedral 8×8 | 1024 B | Very high (64 directions) |
| 2nd order SH | 48 B | Low (smooth) |
| 3rd order SH | 108 B | Medium |

Recommendation: start with octahedral maps (6×6 or 8×8 interior).

### Memory Estimate (16×16×8 probes, 8×8 octahedral, 64 rays)
- Ray Data: ~8 MB
- Irradiance: ~1.3 MB
- Distance: ~1.3 MB
- **Total: ~11 MB**

### Indoor vs Outdoor
- **Indoor**: DDGI excels — limited space, walls provide bounce, occlusion works great
- **Outdoor**: Use Infinite Scrolling Volume (ISV) — probe grid follows camera.
  Sky/ambient from miss rays. Less occlusion. Often combined with RTAO.

## References

1. Majercik et al. "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields." JCGT 2019. https://jcgt.org/published/0008/02/01/
2. Majercik et al. "Scaling Probe-Based Real-Time Dynamic Global Illumination for Production." JCGT 2021. https://jcgt.org/published/0010/02/01/
3. RTXGI SDK: https://github.com/NVIDIAGameWorks/RTXGI-DDGI
4. RTXGI v2.x: https://github.com/NVIDIA-RTX/RTXGI
5. Cigolle et al. "A Survey of Efficient Representations for Independent Unit Vectors." JCGT 2014.
6. Google Filament: https://github.com/google/filament
7. LuxGI: https://github.com/flwmxd/LuxGI
