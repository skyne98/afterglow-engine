# The Ideal 1999–2005 Game Engine

> Synthesized from WoW, FFXI, Quake/TrenchBroom, Arx Fatalis, and the Dark Engine.

No single engine from the era did everything well. The "perfect" one would cherry-pick the best ideas from each.

## World Architecture

### Outdoors: ADT Tile Grid (WoW)

The world is partitioned into square tiles (533m × 533m), streamed in a 3×3 window around the player. Each tile contains:

| Layer | Data | Technique |
|---|---|---|
| **Heightfield** | Per-corner elevation (33×33 or 129×129 verts) | Grid-based terrain mesh |
| **Texture** | 4+ alpha-blended ground layers | Texture splatting with mipmaps |
| **Static shadows** | Precomputed low-res shadow map per tile | Baked at build time |
| **Object references** | M2 model + WMO placements | Explicit per-tile visibility list (MCRF) |
| **Liquids** | Water/lava planes | Vertex height + render flags |

LOD cascade: full mesh → simplified → horizon silhouette (WDL). Fog masks the transitions.

### Indoors: Portal System (Dark Engine + Arx Fatalis)

Cells connected by portals. Renderer traverses visible cells recursively, clipping frustum through each portal. This is the most efficient indoor rendering system of the era — it naturally culls everything behind walls.

```
camera room → for each portal visible in frustum:
  compute sub-frustum clipped to portal
  recurse into adjacent room
  render surfaces in room
```

### Hybrid

Outdoor zones use the ADT grid. Indoor zones (dungeons, buildings, caves) use the portal system. The transition is seamless — portals connect the outdoor tile to the indoor cell.

## Geometry System

### Level Geometry: Convex Brushes (Quake/TrenchBroom)

All level geometry is built from convex brush primitives defined by plane intersections. This is the Quake model:

```
Brush = ∩(half-spaces of all face planes)
Face = Plane + Material + UV
```

**CSG operations**: Merge (convex union), Subtract (emulated as multiple convex brushes), Hollow. The editor (TrenchBroom) provides vertex/edge/face manipulation tools that translate back to plane changes.

### Static Objects: WMO + M2 (WoW)

| Type | Purpose | Features |
|---|---|---|
| **WMO** | Large architecture (buildings, walls) | BSP tree per group, prebaked vertex lighting, interior/exterior batch separation, antiportal occluders |
| **M2** | Models (props, doodads) | Skeletal animation, LOD meshes, material flags, emissive/specular layers |

### Modular Equipment (FFXI)

Characters are assembled from layered model parts:
1. Base body (race/gender)
2. Head, Body, Hands, Legs, Feet gear
3. Main/off-hand/ranged weapons

Each piece attaches to skeleton bones. This allows thousands of combinations from a small number of source models.

### Skeletal Animation (FFXI + WoW + Dark Engine)

- Bone hierarchy (30–50 bones per character)
- Keyframe-based per-bone animation
- CPU skinning (VU1 on PS2, software on PC)
- LOD animations (lower bone count at distance)
- .anim files for lazy-loaded emotes

## Lighting

### Strategy: Mostly Baked, Minimally Dynamic

| Technique | Source | Cost |
|---|---|---|
| **Vertex colors** (prebaked AO + directional) | FFXI, WoW WMO interior | Free at runtime |
| **Lightmaps** (precomputed radiosity) | Dark Engine | Free at runtime |
| **Dynamic directional** (sun) | WoW exterior | 1 light |
| **Dynamic point lights** (limited count) | Arx Fatalis, Dark Engine | CPU vertex lit |
| **Shadows** | Prebaked in lightmaps | Free |

**No shadow maps**, **no stencil shadows**, **no specular** (or minimal via texture). The entire lighting model is designed to run on fixed-function hardware.

### Light Budget

| Context | Max Dynamic Lights | Notes |
|---|---|---|
| Per-vertex | 8–16 | CPU accumulated |
| Per-cell/zone | 32–128 | Beyond this, baking required |
| Sun | 1 | Directional, always active |

Lighting equation (Gouraud per-vertex):

```
color = ambient + Σ(lights[i].diffuse * N·L * attenuation)
```

No specular term. Attenuation is linear (start distance → end distance).

## Rendering Pipeline

### Vertex Processing (CPU)

```
for each visible object:
  for each vertex:
    transform by bone matrix (skinning)  // only for animated
    accumulate light from up to 8 sources
    project to screen space
    output triangle to rasterizer
```

### Rasterization (Fixed-Function GPU)

```
for each triangle:
  texture lookup (1–2 stages)
  modulate with vertex color
  alpha test (discard transparent)
  fog blend (linear distance)
  write to framebuffer
```

No pixel shaders, no render-to-texture, no post-processing. HDR = 8-bit per channel, no gamma correction.

### Render Order

1. Opaque world geometry (batched by material)
2. Opaque entities (batched by material)
3. Blob/decalshadows
4. Transparent geometry (sorted by material, then distance)
5. Water/lava (multitexture special case)
6. Particles / effects (additive blend)
7. UI

### Platform Paths

| Platform | Transform | Lighting | Rasterization | Special |
|---|---|---|---|---|
| **PC (DX8/9)** | CPU or D3D fixed-function | D3D light or vertex color | D3D fixed-function | Bump mapping optional |
| **PS2** | VU1 microcode | VU1 accumulate | GS registers | Custom alpha/blend modes |
| **Software** | CPU | CPU | Span rasterizer (G2) | Fallback for unsupported HW |

## Spatial Indexing

### Outdoor: ADT Grid + Object Ref Lists

- 2D tile grid (160×160 or similar)
- Each tile stores explicit list of which objects are visible from it
- Objects not referenced from current tile → not drawn
- This is author-time culling, not runtime

### Indoor: BSP Trees (WMO) + Portal Graph

- Each WMO group has an axis-aligned BSP tree for collision + culling
- Portal system automatically culls non-visible cells
- Antiportal occluders for manual optimization

### Combined Pipeline

```
1. ADT tile grid → which tiles are in range
2. Per-tile MCRF → candidate objects for this tile
3. WMO BSP → frustum + occlusion per building
4. Portal graph → visible cells (indoor)
5. Antiportal → manual occluders
6. Fog → final distance cap
```

## File System

### DAT-Style Archive (FFXI)

All assets stored in numbered archive files:

| Range | Content |
|---|---|
| `0–9` | Global data, UI, fonts |
| `10+` | Zone terrain, objects, collisions |
| `1/*` | Models, equipment |
| `2/*` | World maps |
| `3/*` | Effects, particles |
| `4/*`–`5/*` | Audio |

Each archive contains:
```
Header: magic + offset table
Sub-blocks (model):
  - Skeleton (bones, hierarchy)
  - Vertices (pos, normal, UV, color)
  - Indices (triangle strips)
  - Materials (flags, blend mode)
  - Animation (per-bone keyframes)
  - Attachment slots (equipment)
```

## Level Editor: TrenchBroom

The ideal editor follows TrenchBroom's model:

| Feature | Implementation |
|---|---|
| **Geometry** | Brush-based, convex hulls from planes |
| **Tools** | Clip, Vertex, Extrude, Rotate, Scale, CSG |
| **Entities** | Point entities (position only) + Brush entities (geometry-based) |
| **Properties** | Key-value pairs with FGD-defined type editors |
| **Layers/Groups** | Named layers + arbitrary group hierarchy |
| **Materials** | Face-based texture assignment with UV editor |
| **Culling tools** | BSP tree generation, antiportal marking, MCRF authoring |
| **Build pipeline** | Launch external BSP/light/vis compilers |

## Why This Engine Would Work

| Challenge | Solution |
|---|---|
| **Low-end hardware** | All lighting is pre-baked or CPU vertex. No shader requirements. |
| **Large worlds** | Tile streaming + portal culling. Only visible geometry enters the pipeline. |
| **Art pipeline** | Artists bake lighting in Maya/3ds Max → export vertex colors + lightmaps. No runtime lighting setup. |
| **Modularity** | Equipment layering, entity system with FGD definitions, mod priority system. |
| **Cross-platform** | Separate render paths for PC (DX), PS2 (VU1+GS), software fallback. Same assets. |

## Summary Table

| System | Best Approach | Source |
|---|---|---|
| **World (outdoor)** | ADT tile grid, 3×3 streaming, horizon LOD | WoW |
| **World (indoor)** | Portal cells, recursive traversal, frustum clip | Dark Engine, Arx |
| **Level geometry** | Convex brushes from planes | Quake, TrenchBroom |
| **Models** | M2-style: skeletal, LOD, material flags | WoW |
| **Architecture** | WMO: BSP tree, vertex-lit, batch types | WoW |
| **Equipment** | Layered model attachments per bone | FFXI |
| **Lighting** | Baked vertex + lightmaps, few dynamic lights | FFXI, Dark Engine |
| **Shadows** | Prebaked only. Blob decals for dynamic. | Arx, FFXI |
| **LOD** | Terrain cascade + object fade + MCRF culling | WoW |
| **Render API** | DX8/9 fixed-function + PS2 GS + software | All |
| **File format** | DAT-style archives with offsets | FFXI |
| **Editor** | Brush-based, CSG, entity system, FGD | TrenchBroom |
| **Animations** | Skeletal keyframe, CPU skin, LOD | FFXI, WoW |
