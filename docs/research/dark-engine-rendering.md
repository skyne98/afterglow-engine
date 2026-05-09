# Dark Engine Rendering — Complete Technical Analysis

Sources: Leaked Dark Engine source code (Looking Glass Studios, 1996–2000), covering Thief: The Dark Project, Thief II: The Metal Age, and System Shock 2.

## 1. Graphics API — Triple-Path Architecture

The engine supports three rendering paths switchable at runtime:

| Path | Library | Description |
|------|---------|-------------|
| **Software rasterizer** | G2 | Primary/default path. Hand-tuned x86 assembly (Watcom C inline asm). 8/16-bit output, flat/Gouraud shading, perspective-correct or affine texture mapping. |
| **Direct3D hardware** | LGD3D | DirectDraw + Direct3D 1/2 wrapper (`#ifdef USE_D3D2_API`). Uses `D3DVT_TLVERTEX`, `DrawPrimitive`/`DrawIndexedPrimitive`, texture modulation, z-buffer, alpha blending, fog. |
| **Null driver** | — | Disables all rasterization for headless/benchmark modes. |

Default init: `r3_use_g2()` in `R3D/INIT.C`.

### API Detection
```c
lgd3d_set_RGB();      // software RGB mode
lgd3d_set_hardware();  // D3D hardware accelerated
lgd3d_set_software();  // software emulation
```

## 2. Render Architecture — Layered Design

1. **R3D** — 3D rendering core: transform/context/clip pipeline
2. **G2** — Software rasterizer: function-table dispatch with 3 rasterizer types (perspective-correct, affine, flat), sub-pixel accurate triangle rasterization
3. **LGD3D** — Direct3D wrapper: converts `r3s_point` → `D3DTLVERTEX`, batched rendering (max 50 verts)
4. **Portal Renderer** (Thief-specific) — cell-based visibility traversal, frustum clipping, texture/surface caching

### Pipeline Flow
```
portal_render_scene()
  → portal traverses cells via portals
    → for each visible cell:
      → for each surface in cell:
        → software path: g2 rasterizer
        → hardware path: lgd3d via porthw
      → render objects in cell
```

### Camera Spaces (`R3D/CTXTS.H`)
```c
R3_CLIPPING_SPACE=0   // optimized for clipping
R3_PROJECT_SPACE=1    // optimized for fast projection
R3_UNSCALED_SPACE=2   // slow at projection and clipping
R3_LINEAR_SPACE=3     // super fast but only for far objects
```

## 3. Shading / Lighting Model

### World Lighting — Lightmaps
- Pre-computed lightmaps baked into world surfaces (`#define LIGHT_MAP`)
- Dynamic lighting added per-frame modifies lightmap aggregates
- Animated lighting with time-varying intensity via callbacks

### Light Types (`PORT.H`)
| Flag | Description |
|------|-------------|
| `LIGHT_DYNAMIC` | Moving lights |
| `LIGHT_ANIMATED` | Separate lightmap, added with weighting |
| `LIGHT_QUICK` | Not raycast lit |
| `LIGHT_QUAD` | Oversampled raycast |
| `LIGHT_OBJCAST` | Raycast with objects (shadow casting) |

### Multi-Light System for Objects (`MLIGHT.H`)
```c
typedef struct {
   mxs_vector loc, dir;
   rgb_vector bright;     // or float bright (mono)
   float inner, outer, radius;
} mls_multi_light;        // 48 bytes (RGB) or 32 bytes
```
- Up to **32 lights per object**, ~**128 total**
- Accumulated per-vertex: ambient + diffuse + specular

### Per-Vertex Lighting (`MD/LIGHT.C`)
```c
float mdd_lt_amb = .5;    // ambient
float mdd_lt_diff = .5;   // diffuse
float mdd_lt_spec = 0;    // specular
mxs_vector mdd_sun_vec;   // sun direction
```
- Compacted normals (packed into ulong)
- `norm.x = X_NORM(lts[i].norm)` via bitfield extraction

### Palette-Based Lighting (CLUT)
- `pt_light_table[N][256]` — N lookup tables for per-vertex brightness
- `pt_clut[256]` — per-polygon color transformation
- Medium CLUTs for underwater: `pt_medium_entry_clut`, `pt_medium_exit_clut`, `pt_medium_haze_clut`

## 4. Vertex / Geometry Processing

### 3D Point (`R3D/R3DS.H`)
```c
typedef struct {
   mxs_vector p;     // float x,y,z
   ulong ccodes;     // clip codes
   grs_point grp;    // 2D projected + shading
} r3s_point;
```

### 2D Projected Point (`grspoint.h`)
```c
typedef struct {
   fix sx, sy;         // screen coords (16.16 fixed)
   mxs_real w;         // homogeneous w (1/z)
   ulong flags;
   mxs_real i, u, v;   // intensity, texture coords
} grs_point;
```

### G2 Point (with RGB) (`G2/g2pt.h`)
```c
typedef struct {
   fix sx, sy;
   mxs_real w;
   ulong flags;
   mxs_real i, h, d;   // RGB lighting for RGB_GOURAUD mode
   mxs_real u, v;
} g2s_point;
```

### Transform System
```c
typedef float mxs_real;
typedef struct { mxs_real x,y,z; } mxs_vector;
typedef struct { mxs_real m[9]; } mxs_matrix;
typedef struct { mxs_matrix mat; mxs_vector vec; } mxs_trans;
```

Object transforms use a stack model: `r3_start_object*` / `r3_end_object`. Context holds camera space, zoom, aspect, view angles. Zoom = `1.0 / tan(fov * PI / 360)`.

## 5. Texturing

### Bitmap Types
| Type | Description |
|------|-------------|
| `BMT_FLAT8` | 8-bit palettized (primary format) |
| `BMT_FLAT16` | 16-bit RGB |
| `BMT_FLAT24` | 24-bit RGB |
| `BMT_FLAT32` | 32-bit RGBA |

### Texture Family System (`FAMILY.H`)
- Textures organized into "families" sharing a single palette
- Family functions: `family_add()`, `family_remove()`, `family_load()`
- Water texture system: `family_add_water()`, `family_find_water()`

### Texture Pipeline (`LGD3D/TEXTURE.C`)
- Palettized 8-bit + 16-bit RGB support
- Hardware palette via DirectDraw `PALETTE` objects
- Handle-based: `texture_manager` struct with `init/load/unload/set_texture` callbacks
- MIP mapping: textures scaled to power-of-two for GPU

### Chroma Key
```c
void lgd3d_set_chromakey(int r, int g, int b) {
   chroma_key = b>>3;
   if (555) chroma_key += ((g>>3)<<5) + ((r>>3)<<10);
   else     chroma_key += ((g>>2)<<5) + ((r>>3)<<11);  // 565
}
```

## 6. Shadows

- **No stencil/shadow volumes** — 2D projected textures / pre-baked lightmaps
- Dynamic lights create shadows by selectively illuminating surfaces
- `LIGHT_OBJCAST` flag enables raycasting from lights for object shadowing
- `ObjShadowInit()` in `RENDER/RENDER.CPP`
- Object shadows rendered as projected decals on world surfaces via `RENDER.CPP`
- AI vision uses portal system for line-of-sight checks (`CELL_BLOCKS_VISION` flag)
- Surface shadow caching: `clear_surface_cache()`, `clear_surfaces_for_texture()`

## 7. Fog / Atmosphere

### LGD3D Fog
```c
void lgd3d_set_fog_level(float level);    // 0.0–1.0
void lgd3d_set_fog_color(int r, int g, int b);
void lgd3d_set_fog_enable(BOOL);
void lgd3d_set_fog_density(float density);
```
- D3D table fog (exponential): `D3DRENDERSTATE_FOGTABLEMODE = D3DFOG_EXP`
- Vertex fog also supported

### Portal Fog (`PORT.H`)
```c
ulong portal_fog_color[3];
float portal_fog_dist;
BOOL portal_fog_on;
```
- Cell-level fog flags: `CELL_FOGGED_OUT` (32), `CELL_FOG` (64)
- Applied as per-vertex specular alpha

### Underwater Haze
- CLUT-based color transformation via `pt_medium_haze_clut[256]`
- Motion-based overrides: `pt_motion_haze_clut[256]`

## 8. Camera System

```c
typedef enum {
   FIRST_PERSON, THIRD_PERSON, OBJ_ATTACH,
   REMOTE_CAM, VIEW_CAM, DETACHED, TRACKING, USER_DEFINED
} eCameraMode;

typedef struct {
   eCameraMode mode;
   mxs_real    zoom;          // 1.0 = 90° FOV
   mxs_vector  pos;
   mxs_angvec  ang;
   mxs_angvec  ang_off;      // head offset
   ObjID       objid;
} Camera;
```
- Zoom ↔ FOV: `zoom = 1.0 / tan(fov_deg * PI / 360)`
- Per-mode position offsets: `CameraPosOffsets[MAX_CAMERA_MODE]`

## 9. Level Geometry — Portal Rendering System

### Cell Structure (`WRDB.H`)
```c
typedef struct {
   uchar num_vertices, num_polys, num_render_polys, num_portal_polys;
   uchar num_planes, medium, flags, num_full_bright;
   Vertex *vpool;
   PortalPolygonCore *poly_list, *portal_poly_list;
   PortalPolygonRenderInfo *render_list;
   uchar *vertex_list;
   PortalPlane *plane_list;
   PortalCellRenderData *render_data;
   PortalLightMap *light_list;
   Vertex sphere_center;
   mxs_real sphere_radius;
} PortalCell;   // 84 bytes
```

### Polygon Core (8 bytes)
```c
typedef struct {
   uchar flags, num_vertices, planeid, clut_id;
   ushort destination;    // portal destination cell
   uchar motion_index;
} PortalPolygonCore;
```

### Polygon Flags
| Flag | Description |
|------|-------------|
| `PORTAL_BLOCKS_VISION` | Blocks AI sight |
| `PORTAL_BLOCKS_PHYSICS` | Blocks movement |
| `RENDER_DOESNT_LIGHT` | Self-lit surface |
| `RENDER_NO_PHYSICS` | Decal (no collision) |
| `RENDER_CALLBACK` | Custom render callback |

### Cell Flags
| Flag | Value | Description |
|------|-------|-------------|
| `CELL_RENDER_WIREFRAME` | 1 | Debug wireframe |
| `CELL_BLOCKS_VISION` | 8 | Blocks AI line-of-sight |
| `CELL_FOGGED_OUT` | 32 | Fully fogged |
| `CELL_FOG` | 64 | Fog active |

### Portal Traversal Algorithm
1. BSP tree locates player cell
2. Recursively follow portals from each visible cell
3. Frustum clipping at each portal via `PortalClipFromPolygon()`
4. Screen-space rectangle or polygon clipping (`ClipData`)

### Raycasting
```c
void PortalRaycastVector(Location*, mxs_vector*, Location*, int epsilon);
bool PortalRaycast(Location*, Location*, Location*, int epsilon);
#define RAYCAST_MAX_REFS 128
```

## 10. AI Visibility / Light Gem

### Light Gem (`VISMETER.CPP`)
- 16 model variants: `"uicry%02d"` (Thief) or `"watch%02d"` (Deep Cover/SS2)
- Cutoffs from AI visibility control:
```cpp
sAIVisibilityControl *pVisCtrl = AIGetVisCtrl(PlayerObject());
int low = pVisCtrl->lowVisibility;
int med = pVisCtrl->midVisibility;
int hi  = pVisCtrl->highVisibility;
```
- Refresh: `vismeter_refresh` (default 50ms)
- Uses portal system to trace AI→player line-of-sight
- Light level at player location + `CELL_BLOCKS_VISION` determines visibility

## 11. Animation — Skeletal Mesh System

### Mesh Model (MM) — `LIBSRC/MM/`
- Model ID: `"LGMM"`, versioned
- Segments (bones), materials, polygons with vertex weights
- Compacted normals: `mms_uvn { float u,v; ulong norm; }`
- Three render pipelines: `MM_RPIPE_POLYSORT` (software B2F), `MM_RPIPE_ZBUFFER`, `MM_RPIPE_HARDWARE`
- Stretchy joints for cloth/flexible meshes via callback

### Old Model Library (MD) — `LIBSRC/MD/`
- Model ID: `"LGMD"`, version 3
- Rigid-body subobjects with BSP-tree polygon ordering
- Hierarchical subobject transforms: `MD_SUB_ROT`, `MD_SUB_SLIDE`
- BSP node types: `RAW`, `SPLIT`, `CALL`, `VCALL`, `SUBOBJ`

## 12. Special Effects

| Effect | Files |
|--------|-------|
| **Particles** | `PARTICLE.CPP`, `PARTTYPE.H` — sprite-based groups with physics/timing |
| **Weather (rain/snow)** | `WEATHER.CPP`, `WEATHERG.H` — integrated with portal renderer |
| **Water** | `H2OCOLOR.CPP`, `PORTWATR.C/H` — moving surface textures via `PortalCellMotion` (max 256) |
| **Coronas (lens flares)** | `CORONA.CPP` — light source glare |
| **Fire/smoke** | `SPRKPROP.CPP` — spark/fire particle properties |
| **Screen flashes** | `RNFLASH.CPP` — full-screen flash (flashbombs), `flash_clamp_time` |
| **Distant art (sky bg)** | `DISTOBJ.CPP` |

## 13. Sound Engine

### Architecture
- **ISndMixer** COM interface (master control)
- **ISndSample** COM interface (per-sound)
- Backends: DirectSound (`SndCreateDSMixer`), QuickSound (`SndCreateQSMixer`), A3D (`SndCreateA3DMixer`)
- 3D methods: `kSnd3DMethodPanVol`, `kSnd3DMethodSoftware`, `kSnd3DMethodHardware`

### Propagation
- Schema-based: `SCHEMA.CPP` — event-driven playback
- Zone-based ambient: `AMBIENT.CPP`
- Sound propagates through portals — naturally attenuates through doorways
- Limits: `SNDSRC_MAX_GATES = 8`, `SNDSRC_MAX_LABELS = 8`

### Environment
```c
typedef struct {
   float dopplerFactor;
   float distanceFactor;
   float rolloffFactor;
} sSndEnvironment;
```
- Sound groups: `kSndNumGroups = 16`

## 14. Sky Rendering

### Two Sky Systems

**Portal Sky** (`PORTSKY.H`) — software path:
```c
PTSKY_NORMAL    // treat sky like terrain
PTSKY_SPAN      // render into span buffer
PTSKY_ZBUFFER   // render far away
PTSKY_NONE      // skip
```

**Enhanced Sky** (`SKYREND.CPP`) — hardware only:
```c
static ISkyObject *RenderedComponents[] = {
   &SkyRenderer,             // base sky sphere
   &StarRenderer,            // starfield (Flight Unlimited III tech)
   &CelestialObjectRenderer1-4,  // sun/moon
   &CloudDeckRenderer,       // cloud layers
   &DistantArtRenderer,      // distant background art
   0
};
```

### Sky Modes: `kSkyModeTextures`, `kSkyModeStars`

### Sky Object Config
```c
struct sMissionSkyObj {
   BOOL bUseNewSky, bEnableFog;
   float fRadius, fCenterOffset, fHorizonDipAng;
   mxs_vector ControlPointColors[5], GlowColor;
   float fGlowLat, fGlowLong, fGlowAng, fGlowScale;
   eGlowMethod GlowMethod;  // kMethod_Sum or kMethod_Interpolate
};
```

### Star Field (`LIBSRC/STAR/`)
- Vector-based rendering with anti-aliasing
- Supports color ranges and clipping to polygon regions
- From Flight Unlimited III codebase

## 15. Screen Effects

### Gamma
```c
float gamma_level = 1.0;
void set_hardware_gamma_level(float level) {
   IDisplayDevice *pDispDev = AppGetObj(IDisplayDevice);
   pDispDev->SetGamma(gamma_level);
}
```

### Full-Screen Effects
- Flash: `FlashOnlyPlayer(1.0)` — white flash
- Fade: palette-based via `palette_install_fade()`
- Dithering: `lgd3d_set_dithering(BOOL)`
- Anti-aliasing: `lgd3d_set_antialiasing(BOOL)`

### Color Tables (`RENDER.CPP`)
```c
#define BuildIntensity(r,g,b) (((r)*0.3)+((g)*0.55)+((b)*0.15))
```
- Inverse palette (IPAL) accelerated color matching
- `RGBDistanceFunction` pointer for color difference

## 16. Engine Limits

| Limit | Value | Source |
|-------|-------|--------|
| Max terrain polygons | ~1024 per cell | WRDB.H |
| Max textures | `LGD3D_MAX_TEXTURES` (256–1024) | TEXTURE.C |
| Max palettes | 256 | TEXTURE.C |
| Max cells/regions | 28,672 | WRLIMIT.H |
| Max active regions | 768 | WRLIMIT.H |
| Max visible objects | 4,096 | PORTAL.C |
| Max cell motions | 256 | PORT.H |
| Max lights per object | 32 | OBJLIGHT.C |
| Max total lights | ~128 | OBJLIGHT.C |
| Max verts per poly | 50 | LGD3D/RENDER.C |
| Max points per batch | 50 | LGD3D/RENDER.C |
| Max raycast refs | 128 | PORT.H |
| Max texel size | 256×256 | TEXTURE.C |
| Transform stack depth | 8 | INIT.C |
| Z-near / Z-far | 1.0 / 256.0 world units | RENDER.CPP |
| Portal detail level | 1.90 (default) | PORTDRAW.C |
| Sunlight cast length | 1000.0 | PORT.H |
| Model table size | 128 | md.h |
| Sound groups | 16 | lgsound.h |

## 17. Key Source File Index

| System | Path |
|--------|------|
| **R3D Core** | `DarkEngine/LIBSRC/R3D/INIT.C`, `VIEW.C`, `SPACE.C`, `OBJECT.C`, `CLIP.C`, `PRIM.C`, `CTXTS.H` |
| **G2 Software Raster** | `DarkEngine/LIBSRC/G2/G2D.C`, `G2PT.H`, `G2TM.C`, `TMAPD.H`, `GRTM.C`, `PT_MAIN.C`, `PTMAPPER.ASM`, `PTPERSP.ASM`, `PTLINEAR.ASM` |
| **LGD3D Direct3D** | `DarkEngine/LIBSRC/LGD3D/SETUP.C`, `RENDER.C`, `TEXTURE.C`, `TMGR.C` |
| **Portal Renderer** | `thief2/src/PORTAL/PORTAL.C`, `PORTMAIN.C`, `PORTDRAW.C`, `PORTLIT.C`, `PORTHW.C`, `PORTSKY.C`, `PORTWATR.C`, `PORTCLIP.C`, `SURFACES.C` |
| **World Rep** | `thief2/src/PORTAL/WRDB.H`, `WRFUNC.H`, `WRTYPE.H`, `WRLIMIT.H`, `WRBSP.CPP` |
| **Object Rendering** | `thief2/src/RENDER/RENDOBJ.CPP`, `OBJMODEL.C`, `OBJLIGHT.C`, `OBJSHAPE.CPP` |
| **MD Models** | `DarkEngine/LIBSRC/MD/RENDER.C`, `LIGHT.C`, `MIPMAP.C`, `FANCY.C` |
| **MM Skeletal** | `DarkEngine/LIBSRC/MM/RENDER.C`, `SORTPOLY.C`, `XFORMSEG.C` |
| **Sky/Stars** | `DarkEngine/LIBSRC/STAR/STAR.C`; `thief2/src/RENDER/SKYREND.CPP`, `SKYOBJ.CPP`, `CLOUDS.CPP` |
| **Lighting** | `thief2/src/RENDER/MLIGHT.C`, `OBJLIGHT.C`, `LITPROP.CPP`, `ANIMLGT.C` |
| **Camera** | `thief2/src/RENDER/CAMERA.C` |
| **Textures** | `thief2/src/RENDER/FAMILY.C`, `TEXMEM.C`, `MESHTEX.C` |
| **Sound** | `DarkEngine/LIBSRC/SOUND/DMIXER.CPP`, `DSAMPLE.CPP`; `thief2/src/SOUND/SCHEMA.CPP`, `AMBIENT.CPP` |
| **Light Gem** | `thief2/src/DARK/VISMETER.CPP` |
| **Particles** | `thief2/src/RENDER/PARTICLE.CPP`, `WEATHER.CPP`, `CORONA.CPP`, `RNFLASH.CPP` |
| **Palette** | `DarkEngine/LIBSRC/DEV2D/PAL.C`; `thief2/src/RENDER/PALETTE.C`, `PALMGR.C` |
| **Thief Rendering** | `thief2/src/DARK/DRKREND.C` |

## 18. Notable Comments & TODOs
```c
// @TODO: replace this with real animation system            (PORTDRAW.C, RENDER.CPP)
// @TODO: If we want to use this for real we need to know ambient (PORTLIT.C)
// @OPTIMIZE: THIS MUST GO, extra if per texture lookup       (RENDER.CPP)
// @TBD: Set display device kind and set screen mode          (SETUP.C)
// @TBD: make this a UI element, or maybe just config-based   (SKYREND.CPP)
// "zclear not yet implemented"                               (LGD3D/RENDER.C)
// "disable fogtable on @#$! powerVR!"                        (LGD3D/RENDER.C)
// "Cells which are part of doorways should be in small,
//  contiguous clusters, and wear funny little hats,
//  like fezzes with earflaps."                               (WRDB.H)
// "THIS IS HORRIFYING!!"                                     (DRKREND.C)
// "we suck!!!"                                                (DRKREND.C)
// "WHAT IS THIS!!!!!"                                        (DRKREND.C)
```
