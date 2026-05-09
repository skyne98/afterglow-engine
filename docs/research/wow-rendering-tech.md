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

## Open World Architecture

### ADT Tile Grid

The world is partitioned into a square grid of ADT files (terrain tiles), each covering **533.33m × 533.33m** in-game. The grid layout:

```
Tile: 33×33 verts  (one ADT file, ~533m²)
Chunk: 8×8 tiles   (64 ADTs, ~4.3km²)
Continent: 64×64 chunks (4096 ADTs, ~273km²)
```

Each ADT file contains:
- **Heightmap** (129×129 per-corner heights for the full-resolution mesh)
- **Texture alpha layers** (up to 4+ blended ground textures)
- **Low-resolution shadow map** (precomputed static shadows on terrain)
- **Object placement arrays** (MDDF for M2 doodads, MODF for WMO buildings)
- **MCRF references** (which objects are visible from this tile — a per-tile visibility list)
- **Liquid data** (water/lava planes with height and render flags)

### Tile Streaming

The client streams ADT tiles dynamically around the player:

1. **Always loaded**: The tile the player occupies + the 8 surrounding tiles (3×3 grid).
2. **Ring loading**: As the player crosses a tile boundary, new tiles are loaded and distant tiles are unloaded — the 3×3 window slides.
3. **Priority queue**: Tiles closer to the player load first; distant tile LOD loads later.
4. **Async I/O**: ADT files are loaded from disk (or SSD) on background threads, decompressed, and parsed into renderable geometry.

### Level of Detail (LOD)

#### Terrain LOD

| Distance | Technique |
|---|---|
| **Near** (0–~200m) | Full-resolution terrain mesh (per-corner heightmap from ADT MCNK). Multi-texture alpha blending. Detail textures. |
| **Mid** (~200–~800m) | Simplified mesh (fewer verts per tile). Mipmapped terrain textures. |
| **Far** (~800–~1500m) | Coarse terrain approximation. Horizon terrain (WDL) blended in. |
| **Horizon** (>~1500m) | **WDL** — a separate low-resolution heightmap covering the entire continent. No texture, rendered in fog color as a silhouette/shadow. |

The transition between these LOD levels is smoothed by **fog blending**. WoW uses distance fog not just as an atmospheric effect, but as a **culling mechanism** — the fog color matches the horizon terrain color, so the seam between LOD levels is invisible.

Horizon rendering uses a **second, extremely low-poly terrain layer** with no texture — just fog-colored geometry. By blending the near terrain out and the horizon terrain in, WoW achieves seemingly unlimited view distances at negligible GPU cost.

#### Object LOD (M2 + WMO)

- **M2 models** (trees, doodads, NPCs): Have multiple baked LOD meshes at decreasing polygon counts. Models fade in/out at distance thresholds using alpha blending.
- **WMO buildings**: Fade out at distance. Size category in ADT controls max rendering distance.
- **MCRF-based culling**: Objects not referenced in the current tile's MCRF chunk are never drawn — this is a per-tile explicit visibility list authored by the world designers.

#### Draw Distance Settings (CVars)

| CVar | Effect |
|---|---|
| `farclip` | Controls detailed draw distance — where fog starts. Beyond this, only terrain is visible. |
| `horizonfarclip` | Controls how far the horizon terrain (WDL) extends. |
| Environment Detail | LOD bias for objects — higher = further before LOD switches. |
| View Distance | Master multiplier for all draw distances. |

Legion (2016) significantly improved draw distance by overhauling terrain and water rendering at range plus adding proper LOD for trees and buildings.

### Phasing

Phasing is a **server-driven** system that allows different versions of the same world tile to coexist. The server tells the client which "phase" the player is in based on quest progress, story milestones, etc.

- Each phase can have different ADT data, different object placements, different WMO/BSP state.
- The client phases are swapped seamlessly as the player crosses phase boundaries.
- Phasing changes are communicated via the game protocol (not a rendering technique per se, but critical for how the open world is presented).

### Open World Culling Pipeline (Full)

```
1. ADT page grid → which tiles are within farclip + horizonfarclip
2. Per-tile WDT check → does the tile exist? Load it.
3. ADT MCRF → per-tile object visibility reference (which M2/WMO instances to consider)
4. WMO BSP tree → frustum + occlusion culling per building group
5. WMO antiportal → manually placed occluders cull geometry behind them
6. Object LOD → fade out distant M2/WMO based on size category + distance
7. Terrain LOD → switch from full ADT mesh → simplified mesh → WDL horizon
8. Fog blending → mask LOD transitions, cap final visible distance
9. VALAR (VRS) → post-process shading rate reduction in low-detail regions
```

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
- [wowdev.wiki — WDT](https://wowdev.wiki/WDT)
- [wowdev.wiki — WDL](https://wowdev.wiki/WDL)
- [Intel: VALAR in World of Warcraft](https://www.intel.com/content/www/us/en/developer/articles/technical/velocity-luminance-adaptive-rasterization.html)
- [Wowpedia: CVar gxApi](https://wowpedia.fandom.com/wiki/CVar_gxApi)
- [Engadget: WoW's Evolving Engine](https://www.engadget.com/2013-10-21-world-of-warcrafts-evolving-engine.html)
- [Rock Paper Shotgun: WoW Legion Draw Distance](https://www.rockpapershotgun.com/world-of-warcraft-legion-draw-distance)
