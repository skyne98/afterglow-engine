# Final Fantasy XI: Rendering Deep Dive

> Architecture, file formats, PS2 vs PC pipeline, and the DAT model system.

## Engine Overview

FFXI uses a **proprietary engine** built in-house by Square for the PlayStation 2, later ported to Windows. It is *not* a middleware engine (no Unreal, RenderWare, etc.). The PS2 and PC versions share the same asset files (.DAT) but use completely different rendering paths.

### Platform Differences

| Aspect | PS2 | PC |
|---|---|---|
| **Render API** | Graphics Synthesizer (GS) registers + VU microcode | DirectX 8.0a (later DX9) |
| **Shader support** | None — fixed-function GS pipeline | Pixel/vertex shaders optional (bump mapping added later) |
| **Resolution** | 640×480 interlaced, 512×448 common | Independent 3D resolution (configurable via registry) |
| **VRAM** | 4MB embedded eDRAM (no expansion) | System RAM dependent |
| **Texture format** | PS2 swizzled (GS-specific layout) | DXT1/3/5, raw 8/16/32-bit |
| **Special effects** | Black flame, custom GS blending modes, GS alpha test tricks | Missing some PS2 effects (e.g., Shadowlord crawling darkness) |
| **Lights** | Fixed-function GS lighting (max ~8 lights per scene) | DirectX fixed-function or shader-based |
| **Fog** | GS hardware fog (per-pixel, distance-based) | DX fog (vertex or pixel) |

The PS2 version had visual effects that were never replicated on PC because they relied on **GS-specific hardware features** (custom alpha blending configurations, GS microprogram routines). The Shadowlord cutscene effect ("crawling black flame") is the most famous example.

## Hardware Target: PlayStation 2

### Emotion Engine + Graphics Synthesizer

PS2's architecture was famously idiosyncratic:

| Component | Role | Details |
|---|---|---|
| **EE Core** | Main CPU | MIPS R5900, 294 MHz, 2-way superscalar |
| **VU0** | Vector unit 0 | 128-bit SIMD, shared with EE |
| **VU1** | Vector unit 1 | 128-bit SIMD, geometry transformation, microprograms |
| **GS** | Graphics Synthesizer | 147 MHz, 4MB eDRAM, no shaders |
| **RDRAM** | System RAM | 32MB @ 800 MHz |

The GS was an unusual GPU for its era:
- **No programmable shaders** — everything was done via GS register settings
- **4MB eDRAM** — extremely fast but tiny. Texture uploads had to be carefully managed
- **Swizzled textures** — GS-specific texture format for optimal cache performance
- **Triangle setup + rasterization only** — transform and lighting handled by VU1
- **Alpha blending** — multiple blending modes via GS `ALPHA` register, some unique
- **Fog** — hardware distance fog applied during rasterization
- **Antialiasing** — none natively (GS renders to 640×448, then output is interlaced)

### VU1 Microcode

Square wrote custom VU1 microprograms for FFXI that handled:
- Vertex transformation (world → view → projection)
- Lighting calculations (up to 8 directional + ambient)
- Skeletal animation skinning (bone matrix palette)
- Possibly clipping and culling

This is why the PS2 version could run at all — the VU1 acted as a programmable geometry engine before GPUs had vertex shaders.

## DAT File System

All FFXI assets are stored in `.DAT` files — a proprietary container format. The game's `ROM/` directory contains numbered DAT files per zone/category.

### DAT File Categories

| Range | Content |
|---|---|
| `0.dat`–`9.dat` | Global data, fonts, menus, UI |
| `10.dat`+ | Zone-specific terrain, objects, collisions, lights |
| `ROM/1/*.dat` | Character models, equipment, monsters |
| `ROM/2/*.dat` | World maps, terrain tiles |
| `ROM/3/*.dat` | Effects, particles, spells |
| `ROM/4/*.dat` | Sounds |
| `ROM/5/*.dat` | Music |
| `ROM/6/*.dat`–`9/*.dat` | Expansions (Zilart, Promathia, etc.) |

### Model Format (Inside .DAT)

The model format was originally reverse-engineered by "Tomato" (Japanese, 2006) and later by galkareeve and the AltanaViewer project.

Each model DAT contains:

```
Header:
  - Magic/signature
  - Offset tables for sub-blocks

Sub-blocks:
  Skeleton: bone hierarchy, parent indices, initial transforms
  Vertices: position (x,y,z), normal (nx,ny,nz), UV (u,v), color (RGBA)
  Indices: triangle strips / indexed triangles
  Texture references: pointers to texture DATs
  Materials: flags, blend mode, double-sided, glow
  Animation: bone keyframes (position, rotation, scale)
  Equipment slots: attachment points for gear
```

Key characteristics:
- **Very low polygon counts**: player characters ~500–800 triangles, monsters ~1000–3000
- **Skeletal animation**: bones with per-frame keyframe interpolation
- **Vertex colors**: pre-baked ambient occlusion and lighting stored per-vertex
- **Texture sharing**: equipment uses atlas textures mapped via UV coordinates
- **No normal maps in original**: later PC bump mapping added via optional shader path

### Texture Format

| Platform | Format | Notes |
|---|---|---|
| PS2 | GS swizzled (4-bit/8-bit paletted, 16-bit, 32-bit) | Texture cache-optimized layout, paletted for VRAM savings |
| PC | DXT1/3/5, raw 32-bit | Standard D3D formats. Same pixel data, different layout |

The actual pixel data is the same across platforms — only the storage layout differs. Tools like Noesis convert between them.

## Terrain System

### Zone-Based Maps

FFXI is **not seamless open world** — it uses **zone-based loading**. Each zone (area) is loaded independently:

1. Player approaches a zone line
2. Client triggers a loading screen
3. Zone DAT files are loaded from disk
4. Server tells client which entities are present in the zone

Each zone DAT contains:

| Data | Description |
|---|---|
| **Heightfield** | Grid-based terrain elevation |
| **Terrain textures** | Multiple blended ground textures via alpha masks |
| **Static objects** | Buildings, walls, rocks (DAT model references + placement) |
| **Collision mesh** | Simplified mesh for movement / line-of-sight |
| **Lightmap** | Pre-baked directional lighting on static geometry |
| **Water planes** | Height + oscillation parameters |
| **Sound box triggers** | Ambient audio regions |
| **NPC/monster spawn points** | Server-managed but client knows positions |
| **Pathing nodes** | For NPC/monster movement |

### Terrain Rendering

- **Grid-based heightfield** tessellated at varying density
- **Texture splatting**: base texture + detail texture(s) blended via alpha layer
- **Baked vertex lighting**: each terrain vertex has a precomputed color for ambient + directional
- **Fog planes**: zone-specific fog color and distance to mask LOD transitions
- **Sky**: static cubemap or dome with time-of-day color interpolation (not dynamic)

## Character / Creature Rendering

### Pipeline (PS2)

```
VU1 microprogram:
  1. Load bone matrices from main RAM
  2. Transform skinned vertices (bone weighting)
  3. Apply lighting (vertex colors from prebaked + directional)
  4. Project to screen space
  5. Output triangle strips to GS

GS (fixed-function):
  1. Texture lookup (sampled from GS local memory)
  2. Alpha testing + blending (GS register modes)
  3. Fog blending (hardware distance fog)
  4. Write to frame buffer (swizzled format)
```

### Pipeline (PC)

```
DirectX 8/9 fixed-function:
  1. CPU or D3D transforms vertices (D3DTS_WORLD, D3DTS_VIEW, D3DTS_PROJECTION)
  2. D3D lighting or vertex color passthrough
  3. Texture sampling (DXT1/3/5 from system RAM)
  4. Alpha test + blend (D3DRS_ALPHABLENDENABLE, etc.)
  5. Fog (D3DRS_FOGENABLE)
```

Later PC patches added **bump mapping** (optional, via register combiners or shaders).

### Equipment System

FFXI renders equipment by **layering models**:

1. Base body model (race + gender specific)
2. Head equipment (helmet, hat, headband — can be full or partial overlay)
3. Body equipment (armor, robe — replaces torso mesh)
4. Hands equipment (gloves, gauntlets)
5. Legs equipment (pants, skirt)
6. Feet equipment (boots, sandals)
7. Main hand weapon
8. Off-hand weapon/shield
9. Ranged weapon (quiver, bow, gun)

Each equipment piece is a **separate model DAT** with its own skeleton attachment points. The client assembles the full character by:
1. Loading the base body
2. For each equipped slot, loading the model
3. Attaching it to the correct bone in the skeleton
4. Blending textures (some equipment recolor via palette swap)

This modular approach allowed thousands of equipment combinations without storing full character models.

### Animation System

- **Bone hierarchy**: ~30-50 bones per character model
- **Keyframe animation**: stored per-bone position/rotation keyframes
- **Idle/running/casting/attack/magic stances** are animation sequences
- **Morph targets** for facial expressions (limited, mostly static faces)
- **Emote animations** stored as separate `.DAT` references

## Lighting Model

### Original PS2 Pipeline

- **Ambient**: Single zone-wide ambient color (baked into vertex colors)
- **Directional**: Pre-baked per-vertex lighting (stored in vertex color channel)
- **No dynamic shadows**: All shadows are pre-baked into terrain and WMO lightmaps
- **No dynamic lights**: All scene lighting is static. Player torch/mining light = circle texture overlay on screen, not a real light

### PC Additions

- **Dynamic directional light**: Simulated sun via D3D light
- **Specular highlights**: On equipment via texture-based specular maps
- **Bump mapping**: Late-addition, optional on PC

The entire lighting model was **pre-baked by the developers** in Maya/3ds Max and then exported as vertex colors and lightmaps.

## Sky System

- **Static cubemap** per zone (6 faces)
- **Time-of-day color interpolation**: The cubemap colors are blended between dawn/midday/dusk/night palettes
- **Stars**: Animated alpha layer on night sky
- **Clouds**: A scrolling texture layer between the sky dome and the scene
- **Sun**: A sprite/quad with additive blending, positioned in the sky dome
- **Moon**: Similar sprite, visible at night

There is **no dynamic atmospheric scattering** or real-time sky rendering.

## Water

- **Vertex-animated plane**: Low-poly grid with sinusoidal vertex displacement
- **Texture**: Animated scrolling alpha texture (additive blend over surface)
- **Reflection**: None real-time. Cubemap-based static reflection approximation
- **Fresnel**: Simulated via vertex alpha (angle-based opacity)
- **Depth**: Solid color below water plane with distance-based alpha fade
- **PS2 advantage**: GS alpha blending modes created a unique shimmer effect not replicated on PC

## Particles & Effects

- **Spell effects**: Animated billboard sprites + model overlay
- **Weapon skills**: Combination of sprite animation, model fragments, screen shake
- **Weather**: Rain/snow as particle systems, fog as distance-based color blend
- **PS2 effects**: Some used GS alpha blending tricks impossible on DX8 fixed-function

## Model Viewers & Reverse Engineering

| Tool | Author | Year | Features |
|---|---|---|---|
| **FFXI Tool** | Tomato (Japanese) | 2006 | Original DAT RE, model viewing, extraction. Basis for all later tools. |
| **Noesis** | Rich Whitehouse | ~2008+ | Universal model viewer with FFXI plugin. Can export to OBJ, DAE, SMD. |
| **MapViewer** | galkareeve | 2016 | OpenGL DAT viewer, zone rendering, C++. |
| **AltanaViewer** | voliathon | 2020+ | Full model viewer with CSV-based DAT mapping. Most maintained modern option. Open-source. |
| **XiEvents** | atom0s | 2020+ | Zone cutscene/event data reverse engineering. Python/RE tools. |

### AltanaViewer Architecture

AltanaViewer is the most complete open-source FFXI model viewer:

- C++ application reading DAT files directly
- CSV dictionaries mapping DAT IDs to human-readable names
- Renders models via OpenGL with game-accurate shader approximations
- Supports equipment layering (body + each slot)
- Animation playback
- Zone terrain rendering
- Continuously updated (new equipment DATs mapped as SE adds them)

## Why FFXI Still Looks Good

| Factor | Explanation |
|---|---|
| **Art direction** | Square's artists designed for a cohesive gothic/medieval/asian-fantasy aesthetic |
| **Baked lighting** | Pre-computed vertex colors give soft, natural shading with no runtime cost |
| **Color palette** | Restrained, warm tones with strategic bright accents (spells, weapons) |
| **Stylized proportions** | Characters are not realistic — they have exaggerated features that hide low polygon counts |
| **Texture quality** | Small by modern standards (128×128–256×256 common) but artistically painted, not photo-sourced |
| **Consistent camera** | Fixed third-person perspective hides geometry limitations |

## References

- [AltanaViewer GitHub](https://github.com/voliathon/AltanaViewer)
- [galkareeve/xigaze DAT RE](https://github.com/galkareeve/ffxi)
- [atom0s/XiEvents](https://github.com/atom0s/XiEvents)
- [FFXI Modding Blog (Noesis, Tomato, tools)](http://ffximodding.blogspot.com/)
- [FFXIclopedia: Graphics Guide](https://ffxiclopedia.fandom.com/wiki/Graphics)
- [Reverse Engineering FFXI DAT files (Reddit)](https://www.reddit.com/r/ffxi/comments/1icfkpk/reverse_engineering_ffxis_dat_files/)
- [PC vs PS2 Graphics Discussion](https://www.ffxionline.com/forum/ffxi-game-related/ffxi-frequently-asked-questions/10197-pc-to-ps2-graphics)
