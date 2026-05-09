# Early MMO Graphics: 1999–2005 Roundup

> MMOs with standout visual quality or art direction from the early era.

## The Contenders

| Game | Year | Engine | Graphics Highlights | Still Playable? |
|---|---|---|---|---|
| **EverQuest** | 1999 | Proprietary (Verant) | Voxel-like terrain? No — actually polygonal. Fog-based distance culling. Skeletal animation on characters. Water planes. | Yes (live, progression servers) |
| **Asheron's Call** | 1999 | Proprietary (Turbine) | Large seamless world with no loading screens. Dynamic day/night cycle. LOD terrain. | Yes (private server, emulator) |
| **Dark Age of Camelot** | 2001 | NetImmerse (Gamebryo precursor) | Skeletal animation, specular highlights, reflective water, dynamic skyboxes. Realm-vs-realm with 200+ players rendered. | Yes (live, Eden freeshard) |
| **Final Fantasy XI** | 2002 | Proprietary (Square) | **PS2-based**. Pre-dawn global illumination style lighting, baked shadows, layered terrain textures, water with reflection, day/night transitions, weather. Art direction carried the hardware. | Yes (live) |
| **EVE Online** | 2003 | Proprietary (CCP, Carbon later) | **Shader-based nebula/lighting from day one**. Procedural planet rendering, volumetric god rays (added later), high-detail ships, multi-frequency lighting on hulls. The skybox was always the selling point. | Yes (live, parity client) |
| **Star Wars Galaxies** | 2003 | Proprietary (Sony/Verant) | Roamable planet surfaces with terrain elevation. Prodedural flora/rock scattering. Day/night, weather. Real-time shadow maps. | Emulator only (SWGEmu, Legends) |
| **City of Heroes** | 2004 | Proprietary (Cryptic) | Stylized cel-shaded+comic aesthetic. High-detail character creator. Baked AO on buildings. Glow/emissive shaders for powers. | Yes (Homecoming private server) |
| **Guild Wars** | 2005 | Proprietary (ArenaNet) | Per-instanced zones with precomputed lighting bakes. High-detail static geometry (baked from 3ds Max). Lush vegetation, layered water, high-res textures compared to peers. "Beautiful for the time" is understatement. | Yes (live, no sub fee) |
| **World of Warcraft** | 2004 | Proprietary (Blizzard, Warcraft III derived) | Stylized art direction, terrain LOD with horizon system, multi-texture blending, water with Fresnel, dynamic sun, skeletal animation, WMO BSP architecture, M2 model system, LOD for objects. | Yes (live, Classic) |

## Notes by Game

### Final Fantasy XI (2002)

- PS2 hardware target shaped everything: fixed-function transform pipeline, baked vertex lighting on world geometry.
- **Pre-baked sky**: The sky is a static cubemap with time-of-day color interpolation. No dynamic sky rendering.
- **Baked shadows**: All world geometry shadows are precomputed into the terrain and WMO textures.
- **Water**: Opaque animated texture + vertex oscillation for waves. No reflection.
- **Character rendering**: Vertex-lit with palette-based face textures. No normal maps.
- **What made it beautiful**: Cohesive art direction, restrained color palette, european/gothic architecture, meticulous scene composition. It looked great on a CRT.

### EVE Online (2003)

- The **nebula background** technique: procedural gradient textures generated on the GPU based on artist-defined control points. Each system has a unique nebula. This creates stunning vistas for minimal cost.
- **Ship rendering**: Multi-texture with emissive, specular, glow maps. Later added normal maps. The ships are rendered against the nebula + starfield with bloom and later HDR.
- **Fake lighting**: Early EVE didn't have dynamic lights — ships "glow" from emissive textures and a simple directional light.
- **Carbon engine (2012+)**: Modernized with deferred rendering, HDR, volumetric lighting, but the aesthetic was already established.

### Guild Wars (2005)

- **Precomputed lighting**: Zones are baked offline with radiosity. The static geometry has lighting baked into lightmaps. This meant zero runtime lighting cost for the world, allowing very complex scenes.
- **Zone streaming**: Each zone is a separate instance loaded on demand. No seamless open world, but each zone is highly detailed.
- **Layered water**: Multiple translucent layers + reflection map + vertex animation. Considered best water of any MMO at the time.
- **Character shaders**: Skintones with subsurface scattering approximation, metal with cubemap reflection, cloth with anisotropy hints.
- **Art pipeline**: Geometry exported from 3ds Max with full lighting bakes — what you see in Max is what you get in-game.

### World of Warcraft (2004)

- Covered in detail in the WoW rendering tech note. Key innovations for its time: terrain LOD horizon system, WMO BSP architecture for interiors, LOD for objects, multi-texture terrain blending.

## Why these games aged well visually

| Reason | Examples |
|---|---|
| **Strong art direction over fidelity** | WoW, FFXI, EVE, Guild Wars |
| **Baked lighting** | Guild Wars, FFXI, old WoW WMOs |
| **Stylized / non-realistic aesthetic** | WoW, City of Heroes, Guild Wars |
| **Skybox / atmosphere as visual anchor** | EVE, DAoC, WoW |
| **Limited but tasteful color palette** | FFXI, EVE, Dark Age of Camelot |

## References

- [EVE Online: The Making of the Nebula](https://www.eveonline.com/news/view/the-making-of-eves-nebula)
- [Guild Wars 2 Art Design (note: GW1 had similar pipeline)](https://www.arena.net/en/news/blog/the-art-of-guild-wars-2)
- [FFXI: Behind the Scenes (2002 dev interview)](https://www.ffxionline.com/forum/general/community/general-discussion/4203-the-making-of-ffxi)
- [Dark Age of Camelot 20th Anniversary](https://www.darkageofcamelot.com/content/20-years-dark-age-camelot)
- [City of Heroes Art Style Postmortem](https://massivelyop.com/2024/04/28/city-of-heroes-art-style-postmortem/)
