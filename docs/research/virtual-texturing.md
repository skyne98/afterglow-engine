# Virtual Texturing Research

Research date: 2026-07-12

## Summary

Virtual texturing (VT) allows rendering textures too large to fit in VRAM by
splitting them into small pages (typically 128×128), loading only the pages
the GPU actually samples, and mapping virtual addresses to physical pages
through a page table — exactly like OS virtual memory, but for textures.

### Key insight from all sources

VT is NOT just "progressive mip streaming" (which we already have). It's a
fundamentally different rendering technique where:

1. The GPU samples a **page table** texture to translate virtual UV → physical atlas UV
2. A **feedback pass** tells the CPU which pages the GPU needs
3. The CPU loads/transcodes pages on demand and updates the page table

The follow-up
[`virtual-texture-perceptual-priority-score.md`](virtual-texture-perceptual-priority-score.md)
audits the known-good priority mechanisms selected for Afterglow's predicted
streaming experiment: Zhang et al.'s additive fragment coverage/distance/
displayed-quality weight, Cesium foveation, and RAGE resident-mip deficit.

---

## 1. Existing Three.js VT Efforts

### elfrank/virtual-texturing (three.js r57, WebGL)

The most complete Three.js VT implementation. MIT licensed.

**Architecture:**

| Component | Implementation |
|-----------|---------------|
| **Page table** | `THREE.DataTexture` (FloatType, RGBA, NearestFilter). Each texel = (pageX, pageY, mipLevel, 255). Quad-tree structure, mipmapped. |
| **Physical atlas** | `THREE.DataTexture` (RGBA8, UnsignedByteType). Fixed size (e.g. 2048×2048). Divided into tile slots with padding. Pages uploaded via `gl.texSubImage2D`. |
| **Feedback** | Separate render pass to low-res `WebGLRenderTarget` (1/8 screen). Shader writes (pageX, pageY, mipLevel, 255). CPU reads back with `gl.readPixels()`. |
| **Page cache** | LRU eviction. `freeSlot()` finds lowest-mip-level page to evict. `restorePage()` for pages pending delete. |
| **Tile queue** | Priority queue sorted by hit count. Loads tiles as images via HTTP. |
| **Progressive loading** | When a page is requested, also request all parent (lower-res) pages — blurry preview before high-res arrives. |

**Key shader — page table lookup:**
```glsl
vec2 computeUvCoords(vec2 vUv) {
    vec3 pageData = texture2D(tCacheIndirection, vUv).xyz;
    float mipExp = exp2(pageData.z);
    vec2 inPageOffset = fract(vUv * mipExp) * vCachePageSize;
    return pageData.xy + inPageOffset;
}
```

**Key shader — feedback (mip level from screen-space derivatives):**
```glsl
float MipLevel(vec2 uv, vec2 size) {
    vec2 coordPixels = uv * size;
    vec2 dx = dFdx(coordPixels);
    vec2 dy = dFdy(coordPixels);
    float d = max(dot(dx, dx), dot(dy, dy));
    return max(0.5 * log2(d) - 1.0, 0.0);
}
```

**Key shader — feedback pass writes page ID:**
```glsl
void main() {
    float mipLevel = floor(MipLevel(vUv, fVirtualTextureSize));
    mipLevel = clamp(mipLevel, 0.0, fMaximumMipMapLevel);
    vec4 result;
    result.rg = floor(vUv.xy * fTileCount / exp2(mipLevel)); // pageId
    result.b = mipLevel;
    result.a = 255.0;
    gl_FragColor = result / 255.0;
}
```

**WebGL only** — uses `gl.readPixels()` for feedback, `gl.texSubImage2D` for
page uploads. No WebGPU support.

### Other Three.js VT

- Three.js issue #2587 — elfrank's original VT proposal (closed, not merged)
- No VT in Three.js core or official examples
- No known Three.js WebGPU VT implementation exists

---

## 2. Three.js WebGPU VT

**No existing implementation found.** Three.js WebGPU is still maturing.

### What's available for custom VT in Three.js WebGPU:

**TSL (Three.js Shading Language):**
```ts
import { texture, uv } from 'three/tsl';
const detail = texture(detailMap, uv().mul(10));
const material = new THREE.MeshStandardNodeMaterial();
material.colorNode = texture(colorMap).mul(detail);
```

TSL supports:
- `texture(sampler, uv)` — standard texture sampling
- Custom nodes for `colorNode`, `positionNode`, `vertexNode`
- `NodeMaterial` / `MeshStandardNodeMaterial` for custom shader injection
- Storage buffers (via `storageBuffer` node)
- `textureGrad()` equivalent for explicit derivatives

**What we'd need for VT in TSL:**
- A custom `textureVT(virtualUV)` node that:
  1. Samples the page table: `texture(pageTable, virtualUV)`
  2. Computes physical atlas UV from page table result
  3. Samples the atlas: `textureGrad(atlas, physicalUV, dx, dy)`
- A feedback render pass (separate scene + render target)
- GPU→CPU readback via `copyBufferToBuffer` + `mapAsync`

**WebGPU feedback approach:**
- Render feedback to a `GPUTexture` (unsigned integer format)
- `copyTextureToBuffer` → `mapAsync` to read back (1-2 frame latency)
- OR: fragment shader writes to a `GPUStorageBuffer` (if supported in Three.js WebGPU)

---

## 3. Generic WebGPU VT

### shlomnissan/virtual-textures (C++/OpenGL, 2025)

A minimal, clean VT prototype. Not WebGPU; it is the best concise reference for
software page-table/atlas sampling, but not the best end-to-end streaming or
production-policy reference. See the later
[id Tech audit](id-tech-virtual-texturing-audit.md) and
[Unreal public-interface audit](unreal-engine-virtual-texturing-public-audit.md).

**Architecture:**

| Component | Implementation |
|-----------|---------------|
| **Page table** | `usampler2D` (unsigned integer, mipmapped). Each entry: bit 0 = resident, bits 1-8 = physical page X, bits 9-16 = physical page Y. Page table is itself mipmapped — mip 0 has finest pages, higher mips have coarser. |
| **Physical atlas** | `sampler2D` (SRGBA8), 4096×4096. Pages 512×512 + 4px padding. Pages uploaded via `UpdateSubregion()`. |
| **Feedback** | Render to `uvec4` framebuffer (unsigned integer). Each pixel: bit 31 = valid, bits 0-4 = mip, bits 5-12 = pageX, bits 13-20 = pageY. CPU reads back. |
| **Page cache** | LRU with `std::list` + `std::unordered_map`. Pinned LODs (coarsest mips always resident). `Touch()` updates LRU, `Acquire()` evicts LRU page. |
| **Page manager** | `IngestFeedback()` → `RequestPage()` → `FlushProcessingRequests()` (async image loading). |

**Key shader — material (page table lookup with fallback):**
```glsl
void main() {
    float mip_float = clamp(ComputeMipLevel(u_VirtualSize, v_TexCoord), ...);
    int mip_level = int(mip_float);

    // Walk from desired mip up to max, looking for resident page
    for (; mip_level <= max_level; ++mip_level) {
        vec2 page_coords = floor(v_TexCoord * curr_page_grid);
        entry = texelFetch(u_PageTable, ivec2(page_coords), mip_level).r;
        if ((entry & 1u) != 0u) { is_resident = true; break; }
    }

    // Compute physical atlas UV
    ivec2 physical_page = ivec2((entry >> 1) & 0xFF, (entry >> 9) & 0xFF);
    vec2 local_uv = fract(v_TexCoord * curr_page_grid);
    vec2 page_origin = vec2(physical_page) * (u_PageSize + u_PagePadding);
    vec2 atlas_uv = (page_origin + half_padding + local_uv * u_PageSize) / u_AtlasSize;

    // Sample with scaled derivatives for correct filtering
    vec2 dx = dFdx(v_TexCoord) * curr_page_grid * (u_PageSize / u_AtlasSize);
    vec2 dy = dFdy(v_TexCoord) * curr_page_grid * (u_PageSize / u_AtlasSize);
    v_FragColor = textureGrad(u_TextureAtlas, atlas_uv, dx, dy);
}
```

**Key pattern — fallback to coarser mip:** If the desired mip's page isn't
resident, the shader walks up to coarser mips until it finds one. This gives
automatic "blurry preview" without explicit progressive loading.

**Key shader — feedback:**
```glsl
void main() {
    uint mip_level = uint(clamp(ComputeMipLevel(...), ...));
    float mip_scale = exp2(-float(mip_level));
    vec2 curr_page_grid = max(u_PageGrid * mip_scale, vec2(1.0));
    vec2 page_coords = floor(v_TexCoord * curr_page_grid);
    v_FragColor = uvec4(PackPageData(mip_level, page_coords.x, page_coords.y), 0, 0, 0);
}
```

### WebGPU limitations

- **No sparse textures** — WebGPU has no `GPUSparseTexture` (proposed in gpuweb#455, not implemented)
- **No hardware page tables** — must use page table texture (software indirection)
- **GPU→CPU readback** — `copyBufferToBuffer` / `copyTextureToBuffer` + `mapAsync` (1-2 frame latency)
- **Storage buffers** — can be used for feedback (fragment shader writes page IDs), but readback still async

### Other references

- **LibVT** (core-code/LibVT) — C++ VT library, MIT, OpenGL. Supports up to 256k² virtual textures, multithreaded tile streaming, DXT1/5 compression. References Sean Barrett and Carmack.
- **studiopixl.com sparse virtual textures** — blog post with GLSL shader code for feedback buffer
- **gpuweb#455** — WebGPU sparse resources proposal (not implemented)

---

## 4. CryEngine + Spartan Engine

### CryEngine

CryEngine has **texture streaming** (`r_TexturesStreaming`), not full virtual
texturing. It streams mip levels of individual textures, not pages from a
virtual texture atlas.

- CryEngine 2 (Crysis) used "Texture Virtualization" for terrain (Mittring 2008)
- The streaming system copies texture data in background, with priority queues
- No page table / indirection texture — it's traditional texture streaming

CryEngine's approach is closer to what we already have (progressive mip
streaming per texture) than true virtual texturing.

### Spartan Engine (PanosK92/SpartanEngine)

- GPU-driven, bindless renderer with path-traced GI
- **No virtual texturing** — searched source code, no VT/page table/atlas/tile files found
- Uses bindless textures (different approach to the "many textures" problem)

### Other engines

- **Unreal Engine** — production SVT since UE4.19: cooked tiles, page tables,
  fixed physical pools, feedback, disk-capable producers, and controlled
  finalization. Full implementation source is EULA-gated and was not available
  to the current audit identity; official public contracts are analyzed in
  [Unreal Engine virtual texturing](unreal-engine-virtual-texturing-public-audit.md).
- **Unity** — Texture2DArray + mipmap streaming, not full VT
- **O3DE** — no VT found

---

## 5. id Tech / Doom

The detailed follow-up is [id Software virtual texturing](id-tech-virtual-texturing-audit.md).
It distinguishes the GPL id Tech 4 camera clipmap from the unreleased but
extensively documented production RAGE asset-VT system.

### Sean Barrett — "Sparse Virtual Textures" (GDC 2008)

The seminal VT talk. Source code at silverspaceship.com/src/svt/ (public domain).

**Key concepts introduced:**
1. **Page table as mipmapped texture** — one texel per virtual page, hardware does nearest-texel lookup
2. **Feedback buffer** — separate render pass, low resolution (10x smaller than screen)
3. **LRU eviction** — finest mip pages evicted first
4. **Page borders** — 4-texel border for bilinear/anisotropic filtering
5. **Fallback to coarser mip** — if page not resident, page table points to coarser mip

Barrett noted: "the page table textures can be big bloated floats just fine,
since they're being repeatedly sampled so much, but packing the physical
textures is crucial for performance" (DXT/BC compression).

### id Software — "Software Virtual Textures" (van Waveren, 2012)

The definitive paper on VT implementation. Used in RAGE (id Tech 5).

**Key architecture:**

| Parameter | Value |
|-----------|-------|
| Page size | 128×128 texels + 4-texel border (120×120 payload) |
| Physical atlas | 4096×4096 (32×32 pages) |
| Virtual texture | 120K×120K (1024×1024 pages) |
| Page table | 2.66 MB (2 bytes/page) |
| Feedback buffer | 80×60 pixels (10x smaller than render) |
| Page format | YCoCg-DXT5 (diffuse) + DXT5 (normal) + DXT1 (specular) |

**Address translation approaches (6 variants, from simple to complex):**

1. **FP32×4 page table** — one texel per virtual page, stores (scale, bias). 21.33 MB. Simplest, fastest, but large memory.
2. **8:8 page table + FP32×4 mapping** — page table stores (physX, physY), mapping texture stores scale/bias. 2.66 MB. Dependent read.
3. **8:8 + 3×FP32×1** — three single-component mapping textures. 2.66 MB + 12 KB.
4. **8:8 + UINT16×1/2** — fixed-point mapping. 2.66 MB + 6 KB.
5. **8:8:8:8 RGBA page table** — stores (physX, physY, scale_lo, scale_hi). 5.33 MB. No dependent read.
6. **5:6:5 RGB page table** — 5-bit physX, 6-bit log2(width), 5-bit physY. 2.66 MB. Most compact.

**Recommended for our engine:** Approach 5 (RGBA8 page table) — no dependent
read, reasonable memory, simple shader. Or approach 1 (FP32×4) for simplicity
if memory isn't a concern.

**Feedback analysis:**
- Feedback rendered at 10x smaller resolution (80×60 for 1280×720)
- CPU analysis takes ~0.5ms using hash table for quad-tree parent lookup
- Can be 1 frame old (latency tolerant)
- Pages sorted by: (1) distance from desired to resident mip, (2) hit count

**Oversubscription handling:**
- Track resident pages seen in feedback
- If > high water mark: increment feedback LOD bias (back off detail)
- If < low water mark: decrement LOD bias (add detail back)
- Prevents thrashing without performance loss

**LOD snapping / texture popping:**
- When finer mip arrives, create it by upsampling coarser mip first
- Blend from upsampled → actual finer mip gradually
- Avoids abrupt "pop" from bilinear magnification differences

**Page update pipeline:**
1. Render feedback → small screen buffer
2. Feedback analysis → sorted list of needed pages
3. For each page: fetch compressed data from cache (or schedule disk load)
4. Allocate physical page, unmap old page (GPU falls back to coarser mip)
5. Transcode compressed → DXT format
6. Map new page (GPU starts using it)

**Performance (RAGE):**
- 8-16 pages transcoded per frame at 60 FPS = 15 MT/s
- 120K×120K virtual texture, ~300 MB compressed on disk per environment
- Runs at 60 FPS on PS3, Xbox 360, PC

### id Tech 5 / RAGE

- 128K×128K virtual textures supported
- Single virtual texture for all static geometry + another for dynamic objects
- 10 channels per texel (diffuse YCoCg, normal XY, specular RGB, alpha)
- JPEG-like or HD-Photo compression on disk, DXT in VRAM
- Multi-threaded transcoding (Cell SPEs on PS3)

---

## Architecture for afterglow-engine

Based on all research, here's the recommended VT architecture:

### Constants
```
Page size:       128×128 texels + 4px border = 136×136 physical
Atlas size:      2048×2048 (15×15 = 225 pages, use 14×14 = 196 for alignment)
Virtual size:    configurable (e.g. 16K×16K = 128×128 pages)
Page table:      RGBA8, mipmapped, 1 texel per virtual page
Feedback:        RGBA8 render target, 1/8 screen resolution
```

### Components needed

1. **Page table texture** — `THREE.DataTexture` (RGBA8, NearestFilter, mipmapped)
   - Each texel: (physPageX, physPageY, mipLevel, residentFlag)
   - Updated via `texSubImage2D` when pages are loaded/evicted

2. **Physical atlas texture** — `THREE.DataTexture` (RGBA8, 2048×2048)
   - Pages uploaded via `texSubImage2D` at slot positions
   - LRU cache manages which pages are resident

3. **Feedback render target** — low-res render target (1/8 screen)
   - Separate scene with VT feedback shader
   - Writes (pageX, pageY, mipLevel) per pixel
   - CPU reads back via `readPixels` (WebGL) or `copyBufferToBuffer` + `mapAsync` (WebGPU)

4. **Page cache** — LRU with pinned coarsest mips
   - `Touch()` on feedback hit
   - `Acquire()` evicts LRU page (finest mip first)
   - `Commit()` marks page resident, updates page table

5. **Page loader** — async page loading from `.big` container
   - We already have seekable `read(offset, len)` in AssetStore
   - Each page is a compressed chunk in the `.big` format
   - Transcode via Basis worker (if pages stored as Basis)

6. **Custom TSL shader** — `textureVT(virtualUV)` node
   - Samples page table → gets physical page coords
   - Computes atlas UV → samples atlas with `textureGrad`
   - Fallback: walks to coarser mip if page not resident

### Data flow
```
[.big file] → [Page loader] → [Basis transcode] → [Physical atlas] → GPU
                    ↑                                    ↓
              [Page cache] ← [Feedback analysis] ← [Feedback RT] ← GPU
                    ↓
              [Page table] → GPU shader
```

### What we already have (reusable)
- ✅ Basis → RGBA transcode (texture worker)
- ✅ `.big` seekable container format (page = chunk)
- ✅ AssetStore streaming infrastructure
- ✅ WebGPU renderer (Three.js WebGPU)
- ✅ Mip chain generation

### What's new (must build)
- ❌ Page table texture management
- ❌ Physical atlas + LRU cache
- ❌ Feedback render pass + readback
- ❌ Custom TSL `textureVT()` shader node
- ❌ Page loader (read specific page from `.big` by offset)
- ❌ Feedback analysis (parse feedback buffer → page requests)
- ❌ Page layout in `.big` (offline tool to tile textures into pages)
