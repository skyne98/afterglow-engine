# Software Virtual Texturing

## Scope

This note records a practical path for adding generic virtual textures to `afterglow-engine`.

The intended design is software virtual texturing:

- no dependency on hardware sparse/tiled resources
- one generic page-table indirection path
- one physical tile cache per compatible texture class
- automated feedback, loading, eviction, and fallback mips
- material channels layered on top of the same page identity system

The goal is not a special terrain-only system. The goal is a generic streaming texture layer that can be used by terrain, world props, interiors, decals, and large authored surfaces.

## Core Idea

Virtual texturing is GPU-side pagination for textures.

A normal material samples a texture directly:

```text
uv -> texture sample
```

A virtual-textured material samples through indirection:

```text
uv -> virtual page id -> page table -> physical cache tile -> texture sample
```

The source texture is split offline into fixed-size pages at every mip level. Runtime memory holds only a subset of those pages in a physical cache texture. The shader uses a page table to translate virtual texture coordinates into cache coordinates.

The same system can cover many textures if every virtual texture has:

- an ID
- base dimensions
- mip count
- page size
- page table metadata
- disk offsets for pages
- residency state

## Why This Is Generic

The generic part is the address translation. The shader does not need to know whether a page came from terrain, a wall, a prop, or a painting. It only needs:

- virtual texture ID
- UV coordinates
- requested mip
- page table
- physical cache

That makes the system automatable:

1. Import source texture.
2. Tile and pad all mips offline.
3. Store page metadata in an asset.
4. Assign a virtual texture ID.
5. At runtime, feedback requests pages.
6. Streaming jobs upload pages.
7. Page table points virtual pages to physical cache slots.

The material authoring model can remain simple: artists still assign textures. The asset pipeline decides whether a texture is resident or virtual.

## Required Runtime Pieces

### 1. Tiled Texture Asset

Offline processing emits:

```rust
pub struct VirtualTextureAsset {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub page_size: u32,
    pub border_size: u32,
    pub mip_count: u32,
    pub format: VirtualTextureFormat,
    pub pages: Vec<VirtualTexturePageInfo>,
}

pub struct VirtualTexturePageInfo {
    pub mip: u16,
    pub page_x: u16,
    pub page_y: u16,
    pub byte_offset: u64,
    pub byte_size: u32,
}
```

Pages should include border texels so bilinear filtering does not bleed from unrelated neighboring cache tiles.

### 2. Physical Tile Cache

The physical cache is a fixed-size GPU texture atlas:

```text
VirtualTextureCacheAlbedo: 2D texture atlas
VirtualTextureCacheNormal: 2D texture atlas
VirtualTextureCacheOrm: 2D texture atlas
```

Each cache slot holds one page. The cache has a strict memory budget. Eviction can start with LRU and later include priority:

- visible this frame
- visible recently
- requested mip
- camera distance
- chunk priority
- gameplay importance

Root/lowest-resolution pages should stay resident so every sample has a fallback.

### 3. Page Table

The page table maps virtual pages to cache slots:

```text
virtual texture id + mip + page x + page y -> cache x + cache y + resident flag + fallback level
```

Two implementation choices:

1. One mipmapped page-table texture per virtual texture.
2. A packed/global page-table texture or storage buffer keyed by virtual texture ID.

Start with one page table per virtual texture set because it is easier to debug. Move to packed tables later if binding pressure becomes real.

Each page-table entry should encode:

- physical cache tile X
- physical cache tile Y
- mip or level
- validity

If a requested page is not resident, the shader must fall back to a coarser resident page.

### 4. Feedback Pass

A feedback pass determines which pages are needed.

The simple portable version renders a low-resolution offscreen pass. Instead of final material color, it writes:

```text
virtual texture id
page x
page y
mip/level
```

The feedback buffer can be much smaller than the main target because pages cover many pixels. References describe 4x smaller width/height feedback, and even tiny buffers with jitter for experiments.

Important details:

- compute requested mip from derivatives
- compensate for feedback buffer scale
- depth test should be enabled
- blending should be disabled
- use double/triple-buffered readback
- process feedback asynchronously on CPU
- avoid blocking the frame waiting for readback

Later, a compute or sampler-feedback-like path can replace this, but the offscreen feedback pass is the portable first version.

### 5. Streaming And Upload

The CPU pipeline:

1. read feedback from an older frame
2. deduplicate page requests
3. prioritize requests
4. cancel obsolete requests
5. load compressed page data from disk
6. upload a bounded number of pages per frame
7. update page-table entries after upload completes
8. evict low-priority pages when cache is full

Every frame needs explicit budgets:

```rust
pub struct VirtualTextureBudgets {
    pub max_page_uploads_per_frame: u32,
    pub max_upload_bytes_per_frame: u64,
    pub max_feedback_pixels: u32,
}
```

Chunk streaming should prefetch page ranges before feedback sees them. Feedback corrects the prediction.

## Shader Sampling

The shader path is:

1. compute requested mip from UV derivatives
2. find virtual page coordinates at that mip
3. query page-table entry
4. if invalid, walk to coarser mips until valid
5. compute within-page UV
6. apply border/padding correction
7. sample physical cache texture

Fallback is mandatory. Missing high-res pages should appear blurry, not black.

Pseudo-WGSL:

```wgsl
fn sample_virtual_texture(vt_id: u32, uv: vec2<f32>, ddx_uv: vec2<f32>, ddy_uv: vec2<f32>) -> vec4<f32> {
    var mip = requested_mip(uv, ddx_uv, ddy_uv);

    loop {
        let page = virtual_page(vt_id, uv, mip);
        let entry = page_table_lookup(page);

        if entry.valid {
            let cache_uv = physical_cache_uv(entry, uv, mip);
            return textureSampleLevel(physical_cache, physical_sampler, cache_uv, 0.0);
        }

        mip = mip + 1u;
    }
}
```

The loop must always terminate because root pages are permanently resident.

## Multi-Channel PBR

For retro PBR, virtual texturing should treat material channels as a page set:

- albedo
- normal
- ORM: occlusion, roughness, metallic
- emissive, optional
- height/detail, optional

Practical first version:

- one shared page identity for all channels
- separate physical caches per format class
- separate page tables only if channels can have different residency

The simplest robust policy is to load all required core channels together for a page. Later, distant mips can drop normals or optional channels.

## Bevy Integration

Bevy does not provide built-in virtual textures, so this should live as an `afterglow-engine` render subsystem.

Likely module layout:

```text
src/virtual_texture/
  mod.rs
  asset.rs
  cache.rs
  feedback.rs
  page_table.rs
  streaming.rs
  shader.wgsl
  debug.rs
```

Render integration:

- extract visible virtual texture users
- run feedback pass before main material pass
- read back feedback from previous frames
- upload pages in `PrepareResources`
- update page tables before material draw
- sample virtual textures in Afterglow materials

This pairs naturally with chunk streaming:

- chunks declare likely page ranges
- visible chunks raise priority
- feedback requests exact pages
- unloaded chunks release page priority

## Why Hardware Sparse Textures Are Optional

Hardware sparse/tiled resources can simplify filtering and avoid manual atlas indirection, but they are not portable through Bevy/wgpu today. The references also note that software implementations remain valid and sometimes desirable for GPU-driven pipelines.

For Afterglow, use software VT first:

- portable across Bevy/wgpu backends
- debuggable
- compatible with custom material shaders
- does not wait on sparse resource APIs

Hardware sparse support can be an optional backend later.

## Minimum Viable Implementation

Build this in phases:

1. Offline tile one albedo texture into pages with mips and borders.
2. Create one physical cache texture and one page table.
3. Render a low-resolution feedback pass for one test mesh.
4. Read back feedback with frame latency.
5. Load/upload requested pages with a small per-frame budget.
6. Sample through page-table indirection in one material shader.
7. Add fallback mip walking.
8. Add LRU eviction.
9. Add normal and ORM as synchronized page channels.
10. Add chunk prefetch and debug views.

Debug views required:

- feedback buffer
- physical cache atlas
- page table
- page residency overlay
- missing-page overlay
- upload bandwidth
- cache hit/miss counters
- evicted pages per frame

## Recommendation

Yes, there is a single simple conceptual design:

```text
feedback -> page requests -> streaming -> physical cache -> page table -> shader indirection
```

That design is generic and automatable. The first implementation should lean into that simplicity instead of overfitting to terrain or relying on hardware sparse resources.

The production work is in edge cases:

- mip fallback
- tile borders/filtering
- readback latency
- upload budgets
- eviction policy
- multi-channel consistency
- tooling

For `afterglow-engine`, virtual texturing should be planned as a first-class open-world feature, but implemented after the renderer has chunk extraction, material specialization, and GPU debug views in place.

## Sources

- PLAYERUNKNOWN Productions, "Virtual Texturing", 2024  
  https://playerunknownproductions.net/news/virtual-texturing

- Sander van Rossen, "Infinite virtual textures", 2011  
  https://sandervanrossen.blogspot.com/2011/02/infinite-virtual-textures.html

- Nathan Gauër, "Sparse virtual textures", 2022  
  https://studiopixl.com/2022-04-27/sparse-virtual-textures

- Toni Sagristà Sellés, "Sparse Virtual Textures", 2023, updated 2024  
  https://tonisagrista.com/blog/2023/sparse-virtual-textures/
