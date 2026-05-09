# Arx Fatalis Rendering Engine — Complete Technical Analysis

Sources: Arx Fatalis GPL source code (2002, Arkane Studios), Arx Libertatis cross-platform port.

## 1. Graphics API

| Aspect | Detail |
|--------|--------|
| **Original (2002)** | Direct3D 7 immediate mode |
| **Arx Libertatis** | OpenGL 1.5+ fixed-function only (no shaders) |
| **OpenGL ES** | 1.0+ supported |
| **Extensions tracked** | `ARB_texture_non_power_of_two`, `ARB/EXT_texture_filter_anisotropic`, `ARB_vertex_buffer_object`, `ARB_map_buffer_range`, `ARB_buffer_storage`, `ARB_draw_elements_base_vertex`, `NV_fog_distance`, `ARB_sample_shading`, `ARB_ES2_compatibility` |
| **GLEW/libepoxy** | Either supported |

## 2. Shading Model

**Pure per-vertex Gouraud shading.** All lighting computed on CPU, baked into vertex colors, interpolated across triangles by fixed-function rasterization. No pixel shaders, no normal maps, no specular.

### Lighting Equation

```
tempColor = ambientColor
for each light l:
    vLight = normalize(light.pos - position)
    cosangle = dot(normal, vLight)
    if cosangle > 0:
        distance = dist(position, light.pos)
        if distance <= fallstart:
            cosangle *= intensity * 0.85
        else:
            p = (fallend - distance) * falldiffmul
            cosangle *= p * intensity * 0.85
        cosangle *= materialDiffuse
        tempColor += light.rgb255 * cosangle

finalColor = tempColor * factor + term
```

- **Only Lambertian diffuse** — no specular term at all
- **Linear distance attenuation** — falloff between `fallstart` and `fallend`
- **Ambient** — default `Color3f(0.09, 0.09, 0.09)`, NPC items `35/255`
- **Global light factor** — `0.85f`
- **Material diffuse** — scalar multiplier (default 1.0)
- **ColorMod** — per-entity `factor` (multiplicative) + `term` (additive) for special_color, infracolor, highlightColor

## 3. Vertex Structures

```cpp
// Pre-transformed (UI/2D/sprites) — 28 bytes
struct TexturedVertex {
    Vec3f    p;        // position (xyz)
    float    w;        // 1/w for fog coordinate
    ColorRGBA color;   // vertex color
    Vec2f    uv;       // texture coords
};

// Background geometry — 20 bytes
struct SMY_VERTEX {
    Vec3f    p;
    ColorRGBA color;
    Vec2f    uv;
};

// Water/lava (3 UV sets) — 36 bytes
struct SMY_VERTEX3 {
    Vec3f    p;
    ColorRGBA color;
    Vec2f    uv[3];
};

// Mesh vertex with normal for CPU skinning
struct EERIE_VERTEX {
    Vec3f v;      // position
    Vec3f norm;   // normal
};
```

Transformation modes:
- `TexturedVertex` → `GL_NoTransform` (identity, already in screen-space, D3DTLVERTEX compat)
- `SMY_VERTEX` → `GL_ModelViewProjectionTransform`

## 4. Lighting System

| Property | Value |
|----------|-------|
| **Light type** | Point lights only (no directional, no spot) |
| **Max dynamic lights** | 500 (`g_dynamicLightsMax`) |
| **Max lights per vertex** | 16 (`MAX_LLIGHTS`, configurable 6–16) |
| **Attenuation** | Linear: full at ≤ `fallstart`, zero at ≥ `fallend` |
| **Storage** | `g_staticLights` (level) + `HandleArray<LightHandle, EERIE_LIGHT, 500>` (runtime) |

### Light struct fields
`pos`, `fallstart`, `fallend`, `falldiffmul` (precomputed `1/(fallend - fallstart)`), `intensity`, `rgb`/`rgb255`, `ex_flicker`/`ex_radius`/`ex_frequency` (flicker params), `extras` flags.

### Light extras flags
`SEMIDYNAMIC`, `EXTINGUISHABLE`, `STARTEXTINGUISHED`, `SPAWNFIRE`, `SPAWNSMOKE`, `OFF`, `COLORLEGACY`, `NOCASTED`, `FIXFLARESIZE`, `FIREPLACE`, `FLARE`.

## 5. Texturing

| Aspect | Detail |
|--------|--------|
| **Formats loaded** | PNG, JPG, BMP, TGA, PCX |
| **GPU formats** | L8, A8, L8A8, R8G8B8, B8G8R8, R8G8B8A8, B8G8R8A8 |
| **Mipmaps** | Auto-generated via `GL_GENERATE_MIPMAP` |
| **Mipmap LOD bias** | Underwater: +10.0, Interactive objects: −0.6, Default: −0.3 |
| **NPOT** | Supported via `ARB_texture_non_power_of_two`; fallback: pad to POT + UV adjustment via `TextureContainer::uv` (image_size / stored_size) |
| **Anisotropic** | Via `EXT_texture_filter_anisotropic`, configurable |
| **Color key** | Magenta key → alpha test via `glAlphaFunc(GL_GREATER, 0.5f)` or `GL_SAMPLE_ALPHA_TO_COVERAGE` |
| **Wrap modes** | Repeat, Mirror, Clamp (NPOT forces Clamp) |
| **Filter modes** | Nearest, Linear |

### Texture operations (fixed-function combiner)
| Op | GL mapping |
|----|------------|
| `OpModulate` | `GL_MODULATE`, scale=1 |
| `OpModulate2X` | `GL_MODULATE`, scale=2 |
| `OpModulate4X` | `GL_MODULATE`, scale=4 |
| `OpSelectArg1` | `GL_REPLACE` |

### Texture flags
`NoMipmap`, `NoInsert`, `Level`, `NoColorKey`, `Intensity`.

## 6. Fog

- **Linear fog only** (`glFogi(GL_FOG_MODE, GL_LINEAR)`)
- **Per-pixel for TexturedVertex** — `w` field used as fog coordinate via `GL_FOG_COORDINATE_ARRAY`
- **Per-vertex for SMY_VERTEX** — fragment depth serves as fog coordinate
- **Fog start** = `0.3 * cdepth`, **fog end** = `0.5 * cdepth`
- **Default Z clip** = 6400, minimum = 1200
- **Fog color** = interpolated from `depthcolor` RGB
- **NV_fog_distance** detected but incomplete (TODO comment about needing view-space coords)

## 7. Alpha / Blending

### Blending factors
`Zero`, `One`, `SrcColor`, `SrcAlpha`, `InvSrcColor`, `InvSrcAlpha`, `SrcAlphaSaturate`, `DstColor`, `DstAlpha`, `InvDstColor`, `InvDstAlpha`.

### Alpha test
- Strict: `glAlphaFunc(GL_GREATER, 0.5f)`
- Conservative: `glAlphaFunc(GL_GREATER, 0.0f)`
- MSAA paths: `GL_SAMPLE_ALPHA_TO_COVERAGE` (fuzzy) or `GL_ARB_sample_shading` (crisp)

### Render order
1. Opaque background geometry
2. Blob shadows
3. Thrown objects
4. Opaque entities (batched by `RenderMaterial`)
5. Player (forced overdraw in 1st person)
6. Eyeball (flying eye spell)
7. Decals
8. Flushed transparency batches (blended → multiplicative → additive → subtractive)
9. Transparent background geometry
10. Water (multitexture)
11. Lava (multitexture)
12. Halos/glows

## 8. Materials

```cpp
struct RenderMaterial {
    Texture *   m_texture;
    bool        m_depthTest;
    BlendType   m_blendType;   // Opaque, Additive, AlphaAdditive, Screen, Subtractive, Subtractive2
    Layer       m_layer;       // Decal, Effect, EffectForeground, FullscreenEffect, HUDEffect
    WrapMode    m_wrapMode;
    int         m_depthBias;
    bool        m_cullBackfaces;
};
```

Used as key in `std::map<RenderMaterial, VertexBatch>` for render batching.

### Surface type flags (per-polygon)
`POLY_NO_SHADOW`, `POLY_DOUBLESIDED`, `POLY_TRANS`, `POLY_WATER`, `POLY_GLOW` (self-lit), `POLY_IGNORE`, `POLY_QUAD`, `POLY_TILED`, `POLY_METAL` (modulate2x), `POLY_HIDE`, `POLY_STONE`, `POLY_WOOD`, `POLY_GRAVEL`, `POLY_EARTH`, `POLY_NOCOL`, `POLY_LAVA`, `POLY_CLIMB`, `POLY_FALL` (waterfall), `POLY_NOPATH`, `POLY_NODRAW`, `POLY_LATE_MIP`.

- `POLY_METAL` → `OpModulate2X` (vertex color × 2× texture)
- `POLY_GLOW` → full white vertex color

## 9. Meshes

### Mesh struct (`EERIE_3DOBJ`)
- `vertexlocal` — local-space vertices
- `vertexlist` — transformed vertices (pos + normal)
- `vertexWorldPositions` — world-space positions
- `vertexClipPositions` — clip-space positions
- `vertexColors` — per-vertex colors
- `facelist` — `EERIE_FACE` array (3 vertices each, always triangles)
- `grouplist` — vertex groups / bones
- `materials` — per-material TextureContainer pointers
- `m_skeleton` — animation skeleton

### Face struct
```cpp
struct EERIE_FACE {
    PolyType    facetype;
    MaterialId  material;
    VertexId    vid[3];
    float       u[3], v[3];
    float       transval;
    Vec3f       norm;        // face normal
};
```

### Animation
- Skeleton-based CPU vertex skinning
- Max 4 animation layers (`MAX_ANIM_LAYERS`)
- Max 200 animations per entity (`SAVED_MAX_ANIMS`)
- Extra rotation: up to 4 vertex groups (`SAVED_MAX_EXTRA_ROTATE`)

### File format (FTL)
`ARX_FTL_PRIMARY_HEADER` → `ARX_FTL_SECONDARY_HEADER` → offsets to 3D data, cylinder, clothes, collision spheres, physics box.

## 10. Scene Management

### Portal-based room system (primary spatial subdivision)
- Rooms connected by portals (4-sided quads with bounding sphere)
- Recursive portal traversal: camera room → for each portal, if visible in frustum → compute sub-frustum through portal → recurse to adjacent room
- Room frustums stored as `small_vector<EERIE_FRUSTRUM, 8>`

### 2D tile grid (secondary spatial index)
- Grid: 160 × 160 tiles, each 100 × 100 world units (16,000 × 16,000 total)
- `TileData` contains active tile bitset + polygon data + per-tile light lists
- Two-level frustum culling: screen + per-room portal frustums

### Polygon struct (`EERIEPOLY`)
```cpp
struct EERIEPOLY {
    PolyType type;
    Vec3f  min, max;          // AABB
    Vec3f  norm, norm2;       // face normals
    TexturedVertex v[4];      // vertices (up to 4)
    ColorRGBA color[4];       // per-vertex lighting
    Vec3f  nrml[4];           // per-vertex normals
    TextureContainer * tex;
    Vec3f  center;
    float  transval, area;
    RoomHandle room;
    unsigned short uslInd[4];
};
```

## 11. Water Rendering

- Collects `POLY_WATER` polygons during room culling
- 3 texture stages all bound to environment texture
- UV displacement per vertex via `CalculateWaterDisplacement()`:
  ```
  u = (p.x + addVar1) / divVar1 + sign.x * sin((p.x + addVar2) / divVar2 + offset) / divVar4
  v = (p.z + addVar1) / divVar1 + sign.y * cos((p.z + addVar2) / divVar2 + offset) / divVar4
  ```
- Blend: `BlendDstColor * BlendOne` (multiplicative)
- `POLY_FALL` polygons scroll UVs downward (waterfall)
- Vertex color: `Color::gray(0.314f)`

## 12. Lava Rendering

- Two passes:
  1. Multiplicative blend + `OpModulate2X` on stage 0
  2. Subtractive blend (`BlendZero, BlendInvSrcColor`)
- Pulsing glow via `ApplyLavaGlowToVertex()`: modulates vertex color by `sin(time + x + z)`
- Vertex color: `Color::gray(0.4f)`

## 13. Shadows

**Only blob shadows.** No shadow volumes, no shadow mapping, no stencil shadows.

- Circular dark gradient texture projected as a ground quad per entity
- Camera-facing billboard on ground plane
- Drawn after opaque geometry, before transparent

## 14. Particles

### Particle struct (16-byte aligned, trivially copyable)
```cpp
Vec3f p3Pos;           // position
float fSizeStart;      // start size
Vec3f p3Velocity;      // velocity
float fSizeEnd;        // end size
Color4f fColorStart;   // start color
Color4f fColorEnd;     // end color
GameDuration age, timeToLive;
int iRot;
float fRotStart;
```

### ParticleParams
`pos`, `direction`, `gravity`, `looping`, `nbMax`, `freq`, `rotationRandomDirection/Start`, `life`/`lifeRandom`, `angle`, `speed`/`speedRandom`, `rotation`, `texture` (name/count/frameTime), `blendMode`, `startSegment`/`endSegment` (size/color with random), `spawnFlags` (CIRCULAR, BORDER).

### Magic flares
2D screen-space particles for magic effects. Functions: `AddFlare(pos, size, type, entity)`, `FlareLine(pos0, pos1)`.

## 15. Effects

| Effect | Technique |
|--------|-----------|
| **Halos** | Procedural blur/expand of base texture (±5px), additive blend |
| **Lens flares** | Raycast visibility check from light to camera |
| **Decals** | Projected quads on background geometry (scorch marks, blood splats) |
| **Trails** | Camera-facing ribbon quads extruded perpendicular to view |
| **Lightning** | Recursive bezier subdivision (`BEZIERPrecision = 32`) |
| **Fog volumes** | Camera-facing billboards with animated texture scrolling, configurable blend modes |
| **Screen fade** | Full-screen alpha-blended quad overlay |
| **Rotating cone** | Cone of cold spell, configurable vertex count |
| **Fissure** | Ground cracking effect |
| **Field** | Magical barrier effect |

## 16. Post-Processing

**None.** No FBOs, no render-to-texture, no shaders. Direct-to-framebuffer only.

## 17. Performance & Limits

| Metric | Value |
|--------|-------|
| Max tiles | 160 × 160 = 25,600 |
| Tile size | 100 × 100 world units |
| Map area | 16,000 × 16,000 units |
| Max dynamic lights | 500 |
| Max lights per vertex | 16 (hardcoded) |
| Anime layers | 4 |
| Max anims per entity | 200 |
| Extra rotate groups | 4 |
| Default Z clip | 6400 units |
| Min Z clip | 1200 units |
| Global light factor | 0.85 |
| Water/Lava vertex color | 0.314 / 0.4 gray |
| Mip LOD bias (default) | −0.3 |

### Buffer upload strategies (configurable)
Shadow (CPU copy), Map, MapRange, Persistent (`ARB_buffer_storage`).

## 18. Original D3D7 Renderer (remnants)

- `SavedTextureVertex` matches D3D `D3DTLVERTEX`: pos, rhw, color, specular (unused), tu, tv
- D3D7 immediate mode (`DrawPrimitive`, `DrawIndexedPrimitive`)
- Required DirectX 8.0 runtime (with D3D7 compatibility layer)
- Xbox version used Xbox D3D8-like API

## 19. OpenGL Translation Details

| D3D7 feature | OpenGL equivalent |
|--------------|-------------------|
| RHW coords | `glTranslatef(-1, 1, 0); glScalef(2/w, -2/h, 1)` |
| D3DTLVERTEX fog | `GL_FOG_COORDINATE_ARRAY` reading `TexturedVertex::w` |
| BGR textures | Converted to RGB if `GL_BGR` unavailable |
| Alpha test threshold | `glAlphaFunc(GL_GREATER, 0.5f)` |
| Texture stage ops | `GL_COMBINE` + `GL_RGB_SCALE` |
| Vertex buffers | VBO with multiple upload strategies |
| Alpha-to-coverage | `GL_SAMPLE_ALPHA_TO_COVERAGE` / `GL_SAMPLE_SHADING_ARB` |

## 20. Known Limitations & Bugs

- **No shader support** — entirely fixed-function, limits visual quality
- **No specular** — only Lambertian diffuse
- **No directional/spot lights** — point lights only
- **CPU vertex lighting** — limits vertex counts
- **No shadow maps/volumes** — flat blob shadows only
- **No post-processing** — no HDR, bloom, SSAO
- **Sprite sorting** — uses `std::map<RenderMaterial, ...>` instead of distance-based sorting, causes alpha artifacts
- **NPOT textures forced Clamp** — cannot tile NPOT textures
- **Waterfall sorting** — water/lava drawn after all transparent geometry, potential ordering issues
- **Fog clipping** — zclip can pop geometry when `cdepth` changes rapidly
- **Fog is linear only** — no radial/exp fog distribution
- **NV_fog_distance** incomplete (TODO: needs view-space coords)
- **Portal clipping incomplete** — TODO comments about using portals from intermediate rooms
- **Max 16 lights per vertex** — hardcoded limit
- **No texture memory budget** — textures ref-counted via linked list, no LRU
- **Color key** — limited to magenta convention from D3D7
- **No anisotropic fallback** — if extension unavailable, no trilinear fallback

## File Index

| Path | Content |
|------|---------|
| `src/graphics/Renderer.h/.cpp` | Abstract renderer, RenderState, GRenderer global |
| `src/graphics/Vertex.h` | All vertex struct definitions |
| `src/graphics/GraphicsTypes.h` | EERIEPOLY, EERIE_3DOBJ, EERIE_FACE, PolyTypeFlag |
| `src/graphics/GraphicsFormat.h` | On-disk format structs (SavedTextureVertex, etc.) |
| `src/graphics/Color.h` | Color3, Color4, ColorRGBA |
| `src/graphics/opengl/OpenGLRenderer.h/.cpp` | Main OpenGL fixed-function backend |
| `src/graphics/opengl/GLTexture.h/.cpp` | OpenGL texture implementation |
| `src/graphics/opengl/GLTextureStage.h/.cpp` | Texture stage management (combiner ops) |
| `src/graphics/opengl/GLVertexBuffer.h` | VBO implementations (shadow/map/maprange/persistent) |
| `src/graphics/texture/Texture.h` | Abstract Texture base |
| `src/graphics/texture/TextureStage.h` | TextureOp, WrapMode, FilterMode |
| `src/graphics/data/TextureContainer.h/.cpp` | Texture loading, caching, batch data |
| `src/graphics/data/Mesh.h/.cpp` | Mesh loading, scene data |
| `src/graphics/data/FTLFormat.h` | Mesh file binary format |
| `src/graphics/data/FastSceneFormat.h` | Pre-cached level format |
| `src/graphics/RenderBatcher.h/.cpp` | Material-based render batching |
| `src/graphics/Draw.h/.cpp` | Draw primitives (EERIEDRAWPRIM, bitmaps, sprites) |
| `src/graphics/GlobalFog.h/.cpp` | Depth fog and global mods |
| `src/scene/Light.h/.cpp` | Lighting functions, EERIE_LIGHT, ShaderLight |
| `src/scene/Scene.h/.cpp` | Render loop, portals, water, lava |
| `src/scene/Rooms.h/.cpp` | Portal-based room system |
| `src/scene/Tiles.h/.cpp` | 2D tile grid |
| `src/scene/LevelFormat.h` | Level file format |
| `src/scene/Interactive.h/.cpp` | Entity rendering |
| `src/graphics/effects/Halo.h/.cpp` | Light halos |
| `src/graphics/effects/BlobShadow.h/.cpp` | Blob shadows |
| `src/graphics/effects/Decal.h/.cpp` | Decals (scorch, blood) |
| `src/graphics/effects/Fog.h/.cpp` | Volumetric fog volumes |
| `src/graphics/effects/LightFlare.h/.cpp` | Lens flares |
| `src/graphics/effects/Lightning.h/.cpp` | Procedural lightning |
| `src/graphics/effects/Trail.h/.cpp` | Spell trails |
| `src/graphics/effects/Fade.h/.cpp` | Screen fade |
| `src/graphics/effects/SpellEffects.h/.cpp` | Spell effect base + 3D lines |
| `src/graphics/particle/Particle.h` | Particle struct |
| `src/graphics/particle/ParticleParams.h` | Particle parameter definitions |
| `src/graphics/particle/ParticleSystem.h/.cpp` | Particle system |
| `src/graphics/particle/ParticleManager.h/.cpp` | Particle manager |
| `src/graphics/particle/MagicFlare.h/.cpp` | 2D magic flare particles |
| `src/graphics/particle/ParticleEffects.h/.cpp` | Effect implementations |
| `src/animation/AnimationRender.h/.cpp` | Animated entity rendering |
| `src/game/Camera.h/.cpp` | Camera system |
