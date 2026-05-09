# World of Warcraft Rendering Technology

> A breakdown of WoW's engine architecture, file formats, rendering pipeline, and evolution.

## Engine Overview

WoW uses a heavily modified proprietary engine derived from the Warcraft III engine. It has been continuously rewritten over 20+ years. It is **not** Unreal, Unity, or any off-the-shelf engine.

### Rendering Backends (CVar `gxApi`)

| Backend | API | Notes |
|---|---|---|
| `D3D11_LEGACY` | DX11 | Old single-threaded rendering (pre-8.1.5). |
| `D3D11` | DX11 | New multi-threaded DX11 (added 8.1.5). |
| `D3D12` | DX12 | Multi-threaded DX12. Introduced in Battle for Azeroth (8.0). Supports VRS Tier 2. |
| `GLL` | OpenGL | Mac OpenGL (deprecated). |
| `Metal` | Metal | Modern Mac backend. |

Notable: DX12 provides better multi-core utilization and higher minimum FPS, but DX11 can have higher peak FPS in some scenes.

## Render Architecture (DX12)

From Blizzard/Intel's VALAR deep-dive:

> The DirectX 12 Render Architecture for World of Warcraft is **a mix of forward and deferred rendering**.

### Command List Architecture

The renderer separates geometry into multiple parallel command lists:

| Command List | Contents | VRS Applied |
|---|---|---|
| **Default** | Doodads, terrain, world geometry | Yes (screen-space shading mask for doodads, 1x1 for others) |
| **Opaque** | Opaque objects | Partial |
| **Transparent** | Transparent/alpha objects | No |
| **UI** | Interface elements | No |

This split allows different shading rates per object type via VALAR.

### VALAR — Velocity/Luminance Adaptive Rasterization

Introduced with *The War Within* (patch 11.0, 2024). A compute shader that:
1. Runs after each frame renders
2. Generates a **shading rate mask** from the final color output
3. Uses luminance and motion velocity to determine where to reduce shading rate
4. Applies VRS Tier 2 (variable-rate shading) via hardware support

Supports both Intel Arc (built-in) and NVIDIA/AMD GPUs with VRS Tier 2 support.

## File Formats

### M2 (Model Files)

Used for: players, NPCs, creatures, weapons, armor, doodads (decorative objects), skyboxes.

- **Not chunked** in classic format (chunked since Legion).
- Contains: vertices, faces, materials, texture names, animations (bones, sequences), properties.
- Multiple **geosets** per model (togglable body parts).
- `.anim` files for low-priority sequences (emotes) — lazy-loaded.
- **Shader system**: Rendering flags + blend modes + texture layers (diffuse, normal, specular, emissive, env map).
- Uses texture blending for material variety (e.g., team-colored cloth).

### WMO (World Map Objects)

Used for: buildings, architecture, dungeons, instances.

- **Root WMO** (`.wmo`): Header, doodad sets, lights, fog, BSP root, materials, texture paths.
- **Group WMO** (`_001.wmo`, `_002.wmo`, etc.): Actual geometry, split into:
  - **Interior batches**: Lit via prebaked vertex colors (MOCV chunk).
  - **Exterior batches**: Receive dynamic directional light.
  - **Transparent batches**: Alpha/glass surfaces with env mapping.
- **BSP Tree**: Collision + rendering culling per WMO group. Axis-aligned splits (YZ, XZ, XY planes).
- **Antiportals**: Named groups starting with `antiportal` — create occluders, clear BSP, skip rendering.
- **Doodad sets**: References to M2 models placed within the WMO (filtered by set).
- **Vertex color alpha**: Interior = 255, Exterior = 0, Transparent = intact (controls render pass).
- Lighting is **prebaked into vertex colors** for interior groups.

### ADT (Terrain Tiles)

Used for: ground terrain, map tile placement.

- Splits the world into square tiles (usually 533.33m × 533.33m).
- **MCNK chunks**: Per-corner heightmap, alpha layers for ground textures, low-resolution shadow map, holes.
- **Texture layering**: Multiple alpha-blended ground textures per tile.
- **Object placement**:
  - `MDDF` — M2 model placement (doodads) with position, rotation, scale.
  - `MODF` — WMO placement with position, rotation, scale, bounding box.
  - `MCRF` — References which objects are visible from each tile (culling).
- LOD: Objects not referenced in current tile's MCRF are not drawn.
- **SMC (Server Map Chunk)**: Used for server-side object visibility from distance.

### WDT (World Definition)

Specifies which ADT tiles exist. Can reference a **global WMO** (e.g., for underground dungeons that replace the entire world).

### WDL (Low-resolution Terrain)

A simplified heightmap for rendering the world at extreme distances (far LOD).

## Graphics Evolution by Expansion

| Expansion | Major Rendering Changes |
|---|---|
| **Vanilla (2004)** | DX9, pixel/vertex shaders 1.1, WMO + M2 + ADT, specular, spikey water |
| **The Burning Crusade (2007)** | Better asset quality (textures + models), HDR skyboxes? |
| **Wrath of the Lich King (2008)** | Shader model 2.0/3.0, improved water, more complex terrain blending |
| **Cataclysm (2010)** | Full water rewrite, sunshafts, DX11 support, improved fog/atmosphere, lava glow |
| **Mists of Pandaria (2012)** | More detailed terrain textures, improved shadows, higher-res assets |
| **Warlords of Draenor (2014)** | DX11.1+ features, upgraded character models, PBR-like materials, improved specular |
| **Legion (2016)** | Further PBR refinement, enhanced antialiasing, improved particle systems |
| **Battle for Azeroth (2018)** | DX12 backend added, multi-threaded rendering, improved lighting |
| **Shadowlands (2020)** | DX12 maturity, ray-traced shadows (limited), improved volumetric effects |
| **Dragonflight (2022)** | Further DX12 optimization, dynamic resolution scaling? |
| **The War Within (2024)** | **VALAR** — VRS Tier 2 integration, hybrid forward+deferred, improved shaders |

## Rendering Pipeline Details

### Lighting

- **Exterior**: Single dynamic directional light (sun). Attenuation fixed at (0, 0.7, 0.03).
- **Interior (WMO)**: Prebaked vertex lighting. Lightmaps stored in `MOCV` chunk vertex colors.
- **M2 models**: Shader-based lighting with diffuse, specular, emissive, env map layers.
- Multiple lights can be defined per WMO group; only one directional is active at render time.

### Shadows

- Cascaded shadow maps (CSM) for sun.
- RTX shadow support added in Shadowlands (limited scope).
- SSAO (screen-space ambient occlusion) optional.

### Water

- Completely rewritten in Cataclysm.
- Vertex-based wave simulation + reflection/refraction.
- Fresnel term for reflection/transparency blend.

### Terrain

- Heightmap per MCNK corner (129×129 vertices per tile).
- Multi-texture alpha blending (up to 4+ layers per tile).
- Base color layer + detail textures.
- Low-res shadow map per tile.

### Culling Pipeline

1. **WMO BSP trees**: Frustum + occlusion culling per group.
2. **ADT MCRF**: Per-tile object reference culling.
3. **Antiportals**: Manually placed occluders in WMO groups.
4. **Draw distance / fog**: Horizon-based fade.
5. **VRS (VALAR)**: Post-process shading rate reduction in low-detail regions.

## External Tools & Ecosystem

| Tool | Purpose |
|---|---|
| [WoW Model Viewer](https://wowmodelviewer.org) | View/extract M2 and WMO files |
| [M2Mod Redux](https://bitbucket.org/suncurio/m2mod) | Export/edit M2, WMO, ADT; extract full map tiles with ground + objects |
| [WoW Blender Studio](https://gitlab.com/skarnproject/blender-wow-studio) | Blender addon for WMO, M2, ADT import/export |
| [WoWbjectImporter](https://github.com/ThatAsherGuy/WoWbjectImporter) | Blender importer with game-accurate shaders via node trees |
| [FFNx](https://github.com/julianxhokaxhiu/FFNx) | Modding platform (FFVII/VIII, not WoW, but related RE work) |

## References

- [wowdev.wiki — WMO](https://wowdev.wiki/WMO)
- [wowdev.wiki — WMO/Rendering](https://wowdev.wiki/WMO/Rendering)
- [wowdev.wiki — ADT/v18](https://wowdev.wiki/ADT/v18)
- [wowdev.wiki — M2](https://wowdev.wiki/M2)
- [Intel: VALAR in World of Warcraft](https://www.intel.com/content/www/us/en/developer/articles/technical/velocity-luminance-adaptive-rasterization.html)
- [Wowpedia: CVar gxApi](https://wowpedia.fandom.com/wiki/CVar_gxApi)
- [Engadget: WoW's Evolving Engine](https://www.engadget.com/2013-10-21-world-of-warcrafts-evolving-engine.html)
