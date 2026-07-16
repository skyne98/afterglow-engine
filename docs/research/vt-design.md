# Universal Virtual Texturing — Design Document

Date: 2026-07-12

> **2026-07-16 correction:** The universal-VT decision remains current, but
> claims below that pages are offline-pretranscoded and that `texture.wasm` is
> absent are obsolete. `.big` v5 stores portable per-page UASTC Basis; runtime
> selects BC7/ASTC/RGBA and transcodes through a fixed worker pool. The next
> step is a persistent per-device derived cache so warm launches reuse the
> selected GPU blocks. See `docs/api/virtual-texturing.md` and
> `docs/research/device-transcoded-texture-cache.md`.

## Decision

**ALL sampled textures are virtual textures.** No exceptions. Every texture
loaded from disk goes through the VT pipeline: tiled into 128×128 pages,
stored in `.big` as seekable chunks, loaded into a shared physical atlas on
demand via page table indirection.

This is the id Software approach (RAGE/id Tech 5):
"virtual textures applied universally, geometry is typically only broken up
into multiple batches for culling granularity."

## What Stays Non-Virtual

- Render targets (shadow maps, post-processing buffers) — GPU-written, not disk-loaded
- That's it.

## Systems Eliminated (~630 lines of runtime texture code)

| System | Lines | Why eliminated |
|--------|-------|---------------|
| `loadStreamingBasisTexture()` | 72 | Page table fallback loop replaces progressive streaming |
| `processStreaming()` | 66 | Strategy handles page loading priority |
| `parseSerializedMips()` | 76 | Pages are individual `.big` chunks, not serialized mip arrays |
| `detectBestTextureFormat()` | 15 | Offline pipeline pre-transcodes — no runtime detection |
| `nearestUpscale()` | 17 | Shader falls back to coarser mip automatically |
| `StreamingTexture` interface | 9 | Not needed |
| `queueMipUpload()` | 8 | Not needed |
| `loadBasisTexture()` | 7 | Not needed |
| `loadChunked()` | 16 | Pages are 64KB — no chunking |
| Texture fallback (`fallback.ts`) | ~50 | Page table fallback IS the fallback |
| `texture.client.js` | ~300 | No runtime transcoding |
| `texture.wasm` | 10MB | Not loaded at runtime |

## Systems Simplified

### AssetStore
`loadTexture()` → look up VT entry in `.big` header → create page table
binding → return `AssetHandle<THREE.Texture>` pointing to shared atlas.
No format detection, no transcoding, no streaming, no chunking.

### GPU Memory
Before: each texture is a separate GPU allocation (64KB–64MB each).
After: one 2048×2048 atlas (16MB) + one page table (4MB) = 20MB total for
ALL textures. Bounded and predictable.

### Material System
Every material uses `vtSample()`. No `texture()` calls. No per-material
texture bindings — all sample the same atlas. No compressed vs uncompressed
branching.

### LOD Coupling
Before: manual per-LOD texture mip selection.
After: automatic. GPU screen-space derivatives determine mip, page table
provides the right page. Mesh LOD and texture LOD decoupled but both automatic.

### Frame Loop
VT adds 2 steps: feedback readback (1-frame latency) + page loading.
Both use existing patterns (`getArrayBufferAsync()` + `AssetLoader.read(offset, len)`).

### .big Format
Already designed for this. `ChunkInfo { offset, compressed_size }` is exactly
page-level seekable storage. Add `ChunkMeta::VirtualTexturePage { mip, page_x, page_y }`.

### Compression
Before: can't use BC7 in DataTexture (Three.js limitation).
After: atlas is a single CompressedTexture, pages are pre-compressed BC7
blocks. 16× VRAM reduction.

## How VT Maps Onto Existing Engine

| Existing System | VT Use |
|----------------|--------|
| `AssetLoader.read(path, offset, len)` | Page loading — already seekable |
| `.big` format `ChunkInfo { offset, compressed_size }` | Page storage — already seekable per-chunk |
| `AssetHandle.generation` | Page table updates increment generation |
| `AssetStore.poll()` | VT page loading added to poll |
| `prepareAfterglowFrame()` step 1 | Insert feedback readback + page loading |
| `defineResource<T>()` | `VirtualTextureRes` |
| `MeshStandardNodeMaterial.colorNode` | Override with `vtSample()` wgslFn |
| `wgslFn()` / TSL | Write page table lookup + feedback shaders |

## VT Data Flow (integrated)

```
Frame N:
  1. prepareAfterglowFrame()
     a. poll workers (meshopt, asset loader)
     b. VT: read back feedback buffer from frame N-1
     c. VT: strategy.processFeedback() → priority-sorted page requests
     d. VT: load pages (loader.read(offset, len) → copy to atlas)
     e. VT: update page table texture
     f. VT: evict old pages (LRU)
     g. structural commands, transforms, GPU upload
  2. renderer.render(scene, camera)
     a. Render feedback pass (1/8 res, writes page IDs to RT)
     b. Render main scene — material shader uses vtSample():
        - Sample page table → get physical atlas coords
        - Sample atlas with textureGrad()
        - Fallback to coarser mip if page not resident
  3. VT: copy feedback RT → buffer for next frame readback
```

## Prototype Validation

All algorithms validated in `prototype/vt/` with 282 tests, 153,201 assertions.
See `prototype/vt/ALGORITHMS.md` for algorithm documentation with source references.

### Constants

| Parameter | Value |
|-----------|-------|
| Page size | 128×128 texels (payload) |
| Border | 4 texels per side |
| Slot size | 136×136 |
| Atlas | 2048×2048 (15×15 = 225 slots) |
| Page table | RGBA8, mipmapped, NearestFilter |
| Feedback | 1/8 screen resolution |
| Pinned mips | Coarsest 2 levels (always resident) |

### Smart LOD Strategy

Multi-factor priority score with adaptive quality:
```
priority = (visualImpact + temporalStability + prediction + hitScore) × hysteresis
```

- Visual impact: mip distance × screen coverage × center bias
- Temporal stability: consecutive frames × hysteresis (anti-thrashing)
- Prediction: camera velocity × N frames → pre-load future pages
- Adaptive quality: frame-time + oversubscription LOD bias (via max())
- Budget: max pages/frame, adjusts with performance
- Eviction: grace-frame based

All configurable via `VTConfig` (20+ parameters).
