# id Software virtual-texturing audit

Research date: 2026-07-25

## Evidence boundary

This note separates three materially different evidence sets:

1. **id Tech 4 source:** the official GPL Doom 3 repository at
   [`a9c49da5afb18201d31e3f0a429a037e56ce2b9a`](https://github.com/id-Software/DOOM-3/tree/a9c49da5afb18201d31e3f0a429a037e56ce2b9a).
   Its `idMegaTexture` implementation is fully source-auditable.
2. **id Tech 5 / RAGE:** no engine source was released. The primary evidence is
   J. M. P. van Waveren's id-authored
   [*Software Virtual Textures*](https://mrelusive.com/publications/papers/Software-Virtual-Textures.pdf)
   (2012),
   [*id Tech 5 Challenges*](https://mrl.cs.vsb.cz/people/gaura/agu/05-JP_id_Tech_5_Challenges.pdf)
   (SIGGRAPH 2009), and
   [*High Quality Software and Hardware Virtual Textures*](https://mrelusive.com/publications/presentations/2013_siggraph/hq_sw_hw_vts_12.pdf)
   (SIGGRAPH 2013). These describe shipped RAGE behavior but are not source.
3. **id Tech 6/7:** no relevant engine source was released and this audit found
   no id-authored implementation disclosure comparable to the RAGE paper. This
   note does not infer their internals from third-party summaries.

The official Doom 3 BFG source at
[`1caba1979589971b5ed44e315d9ead30b278d8b4`](https://github.com/id-Software/DOOM-3-BFG/tree/1caba1979589971b5ed44e315d9ead30b278d8b4)
contains only an incidental offline-megatexture comment in `BinaryImage.h`; it
has no `idMegaTexture`, feedback, page-table, or streaming implementation.

## Conclusion

**The RAGE design is the closest published architectural reference for
Afterglow.** It covers the complete asset-VT problem: offline sparse population,
independent compressed pages, feedback, bounded-quality overload behavior,
multilevel caches, asynchronous reads, transcode into GPU BC formats, physical
page replacement, and content-before-mapping publication.

The GPL Doom 3 code is not that system. It is an earlier, simple **disk-backed
camera clipmap**: every mip owns a 4×4 toroidal cache around one camera focus,
loads raw RGBA pages synchronously, and samples several clip levels. It has no
GPU feedback, software page table, LRU, transcode, asynchronous I/O, or general
arbitrary-mesh residency.

## Part I — id Tech 4 `idMegaTexture`

### Source inventory

| Path | Role |
|---|---|
| [`neo/renderer/MegaTexture.h`](https://github.com/id-Software/DOOM-3/blob/a9c49da5afb18201d31e3f0a429a037e56ce2b9a/neo/renderer/MegaTexture.h) | constants, file header, per-level and owner state |
| [`neo/renderer/MegaTexture.cpp`](https://github.com/id-Software/DOOM-3/blob/a9c49da5afb18201d31e3f0a429a037e56ce2b9a/neo/renderer/MegaTexture.cpp) | cooker, mip generation, synchronous page reads/uploads, clipmap update |
| [`neo/renderer/Material.cpp`](https://github.com/id-Software/DOOM-3/blob/a9c49da5afb18201d31e3f0a429a037e56ce2b9a/neo/renderer/Material.cpp) | `megaTexture` material keyword and object construction |
| [`neo/renderer/draw_common.cpp`](https://github.com/id-Software/DOOM-3/blob/a9c49da5afb18201d31e3f0a429a037e56ce2b9a/neo/renderer/draw_common.cpp) | per-surface mapping, view-centered update, binding and draw integration |
| [`neo/renderer/RenderSystem_init.cpp`](https://github.com/id-Software/DOOM-3/blob/a9c49da5afb18201d31e3f0a429a037e56ce2b9a/neo/renderer/RenderSystem_init.cpp) | `MakeMegaTexture` console command |

`Material.cpp` requests `megaTexture.vfp`, but that ARB vertex/fragment program
is absent from the GPL repository. The GPL source therefore exposes all CPU,
file, cache, and upload behavior but not the final sampling/masking program.

### Constants and physical organization

```text
TILE_SIZE       = 128
TILE_PER_LEVEL  = 4
level image     = 512 × 512 RGBA8
MAX_LEVELS      = 12
normal channels = nominally 3, but this implementation stores one RGBA image
```

Each logical mip level gets an independent 512² OpenGL texture. Its sixteen
128² slots form a toroidal 4×4 window. A slot's local coordinate must equal the
low two bits of its global page coordinate. As the view crosses a page boundary,
only slots whose global identity changes are reread and overwritten.

A 512² RGBA8 base image consumes 1 MiB; with the complete hardware mip chain it
is about 1.33 MiB. A normal maximum seven-level source therefore uses roughly
9.33 MiB for clip-level images, excluding object/driver overhead.

### `.mega` file format and cooker

The file starts with three native-endian integers:

```text
int tileSize
int tilesWide
int tilesHigh
```

The cooker deliberately seeks to byte `128 × 128 × 4 = 65,536` before writing
the first payload. Runtime uses `tileOffset=1` and seeks to
`tileNumber × 65,536`; consequently the first whole page-sized region is
reserved for the small header.

Payloads are raw, fixed-size, uncompressed 128² RGBA pages. Pages are level-
major, then row-major within each level. Every mip keeps the same 128² physical
page dimensions; logical page-grid width and height halve each level.

`MakeMegaTexture_f`:

1. accepts an uncompressed/RLE RGB(A) or grayscale TGA;
2. rounds each source dimension **down** to a power of two;
3. discards source area beyond the resulting whole 128-page domain;
4. streams one 128-row band of the TGA at a time;
5. writes its pages without holding the whole image in RAM;
6. generates successively coarser page levels into the same file;
7. emits a conventional preview TGA from the first mip fitting in 2048².

This is a real, direct-indexed tile container, but it has no per-page size,
compression mode, checksum, sparse-page marker, channel directory, or version.

### Runtime setup

`InitFromMegaFile` reads the header and builds levels until both dimensions are
at most four pages. Level offsets are cumulative products of prior level grid
dimensions. It allocates one generated 512² image per level, initializes every
slot identity to an invalid sentinel, and forces the first view update to fill
the cache.

The normal TGA path cannot exceed 32,768 source texels after power-of-two
rounding, or 256 pages on the longest axis. This produces at most seven clip
levels and fits the eight-entry diagnostic-color table and seven bound level
textures. A hand-authored `.mega` file is only minimally validated and could
violate those implicit limits.

### Surface-to-texture mapping

`SetMappingForSurface` searches mesh vertices for an origin and two axis points,
then derives two affine dot products mapping a local-space camera position into
texture coordinates. The source explicitly calls this mapping “not very robust”
and says it works for rectangular grids.

At draw time `draw_common.cpp`:

1. asks the mega texture to derive mapping for the current triangle surface;
2. transforms the global viewer position into object-local space;
3. calls `BindForViewOrigin`;
4. binds the ARB programs and draws;
5. unbinds the level images.

This makes demand a function of **camera position**, not actual sampled pixels.
It is appropriate for contiguous terrain-like surfaces and not arbitrary UV
islands distributed across a scene.

### Page selection and I/O

`SetViewOrigin` maps the camera to a normalized texture center and calls every
level's `UpdateForCenter`. A level computes a four-page window around that
center, rotates it through local slots with bit masks, and calls `UpdateTile`
for all sixteen positions. `UpdateTile` exits when the slot already holds the
requested global page.

A cache miss performs synchronously on the rendering thread:

```text
Seek(global page's fixed offset)
Read(65,536 bytes into a stack buffer)
TexSubImage2D(base level)
CPU box-filter in place
TexSubImage2D(each hardware mip until 1×1)
```

There is a FIXME stating that the disk load should happen in the background.
There is no queue, operation/byte budget, cancellation, stale-generation check,
prefetch, read coalescing, telemetry, or explicit error handling for a short
page read.

### Sampling model recoverable from source

The missing `megaTexture.vfp` prevents a complete shader audit. The C++ contract
shows that texture unit 0 receives a border-clamp/mask image and units 1–7
receive clip levels from coarsest to finest. Each level receives a four-float
mapping/mask parameter. Unused levels bind white with parameters that disable
contribution. The likely model is clipmap masking/blending, but its exact blend,
derivative, seam, and fallback behavior cannot be verified from this release.

There are no per-page borders in the file. The toroidal placement keeps most
virtual neighbors adjacent in the level cache, with texture repeat handling the
ring edge; that is fundamentally different from a general atlas whose adjacent
virtual pages may occupy unrelated slots.

### Source-level defects and limitations

- **Blocking hot path:** file seek/read, CPU mip generation, and all uploads can
  occur while preparing a draw.
- **No bounded page work:** a large center jump can replace every slot at every
  level in one update.
- **Header/runtime mismatch:** runtime validates only `tileSize >= 64` but all
  offsets and stack buffers use compile-time `TILE_SIZE=128`; a different
  header tile size is accepted and misread.
- **Rectangular mip edge defect:** `GenerateMegaMipMaps` tests `tx > width`
  instead of `tx >= width` (same for y). Once one axis reaches one page before
  the other, the second child coordinate can read outside that level.
- **Wrong off-edge buffer:** the off-edge branch clears `newBlock`, then still
  downsamples stale `oldBlock`; it should clear the input block.
- **Ownership leak:** `idMegaTexture` has no destructor, its long-lived
  `fileHandle` is never closed, and `idMaterial::FreeData` frees the copied
  `newShaderStage_t` without deleting its `megaTexture` pointer.
- **No file version or bounds validation:** malformed dimensions can exceed
  fixed arrays and diagnostic color indexing.
- **Single raw channel group:** comments anticipate normal/diffuse/specular,
  but the implementation stores only one RGBA payload stream.
- **No tests found:** repository searches found no focused MegaTexture test.

These are consistent with an experimental engine path, not the production RAGE
system described later.

## Part II — id Tech 5 / RAGE software virtual textures

### Product shape

RAGE generally used one 120,000² virtual texture for static geometry and a
second for dynamic objects in an environment; very large outdoor areas could
use multiple static VTs. A 120k texture is 1024 pages wide when each physical
page is 128² with a 120² payload and four-texel inset border.

Unlike the id Tech 4 clipmap, geometry can occupy arbitrary unique regions of a
large atlas. Demand is pixel-derived. Multiple VTs can have separate page tables
while sharing or separating physical page pools.

### Address translation

The paper evaluates six representations:

1. complete FP32×4 scale/bias page table;
2. 8:8 physical-page table plus FP32×4 mapping texture;
3. 8:8 plus separate FP32 scalar mapping textures;
4. 8:8 plus fixed-point UINT16 mapping textures;
5. RGBA8 physical xy plus virtual-level width;
6. RGB565 physical xy plus logarithmic virtual-level width.

For a 1024²-page VT, table memory ranges from 21.33 MiB (FP32×4) to 2.66 MiB
(8:8 or RGB565), while RGBA8 consumes 5.33 MiB. RGBA8 measured among the
fastest because it avoids a dependent mapping read; RGB565 halves its memory at
extra ALU cost. The publication does not unambiguously identify one format as
the sole shipping RAGE format, so it should not be inferred.

Every nonresident fine-page entry maps to the nearest resident coarse page.
Mapping a coarse page can require rewriting large square regions of all finer
page-table mips. This is why direct table lookup is cheap for sampling but page
publication can be expensive.

### Filtering and borders

A physical page is 128² with a four-texel border and 120² useful payload. The
border costs about 12% extra texels and is stored on disk to keep pages fully
independent. It aligns with 4×4 BC blocks and supports useful anisotropy.

RAGE used physical textures without hardware mip chains and applied a fixed page
-table LOD bias of `-log2(maxAniso)`. The higher-quality alternative computes
anisotropic LOD from virtual-coordinate derivatives, translates two adjacent
virtual mips, samples both physical pages, and interpolates manually. The 2013
follow-up identifies this as a quality improvement rather than proof that RAGE
shipped the two-translation path.

### Feedback and priority

Feedback stores virtual x/y, desired mip, and VT ID. It may be a separate
low-resolution depth-tested pass or an additional render target. The paper uses
an 80×60 example for 1280×720—more than ten times smaller per axis was reported
without noticeable anomalies—and permits one-frame-old feedback.

CPU feedback analysis:

1. scans the feedback image;
2. deduplicates requests into a quadtree with a hash table for fast parent
   lookup;
3. prioritizes pages first by the gap between desired and resident mip;
4. breaks/improves priority by sample count;
5. touches resident visible pages and requests the misses offering the largest
   visual improvement.

The reported 80×60 analysis cost was about 0.5 ms on multicore PC/Cell. Alpha,
transparency, and multiple VT sources require checkerboard/interleaved feedback
or per-pixel lists; alternating whole sources per frame can destabilize demand.

### Oversubscription policy

RAGE tracks the number of resident pages observed in feedback. Above a high
water mark it increments the global feedback LOD bias; below a low water mark it
decrements toward zero. This sacrifices detail instead of allowing an
unrepresentable working set to thrash.

This is one of the strongest direct lessons for Afterglow: capacity pressure is
converted into an explicit, reversible quality policy before it becomes
unbounded I/O and replacement churn.

### Physical pools and replacement

The normal pool is 4096², or 32×32 = 1,024 physical pages. Three linked physical
textures contain ten material channels:

- BC3/DXT5 YCoCg diffuse;
- BC3/DXT5 normal XY plus power;
- BC1/DXT1 RGB/monochrome specular plus one-bit cover mask.

Together they use about 40 MiB; one linked page is 40 KiB GPU-compressed or
192 KiB uncompressed. A coarsest page is pinned so every address has fallback.

Replacement prefers finer pages first, then least recently used among that
mip. This preserves a resident quadtree by removing leaves. The 2009 talk
summarizes separate linked lists for free, LRU, and locked pages.

### Storage, locality, and caching

Disk pages use variable-rate JPEG-like DCT or HD Photo/JPEG XR, normally 1–6
KiB for all ten channels, versus 40 KiB GPU BC output. The paper recommends
keeping this representation compressed through optical disk, hard-disk cache,
and system-memory cache; only the physical GPU pool uses fixed-rate BC.
Separate I/O threads service each storage device.

Offline processing also:

- computes visibility to omit never-visible finest pages;
- blurs invisible portions to improve compression;
- rescales UV charts based on observed minimum detail, not just world area;
- spatially orders charts before 2D placement for view locality;
- considers page locality on slow optical media, where seeks exceed 100 ms.

The reported RAGE environment data ranged from roughly 112k to 244k populated
pages. HD Photo examples were approximately 170–416 MiB, versus 4.4–9.5 GiB of
GPU-ready BC for the same pages.

### Read, transcode, and publication pipeline

```text
low-resolution feedback
  -> deduplicate/build requested quadtree
  -> sort by visual improvement
  -> compressed cache hit, or schedule background storage miss
  -> allocate replacement physical page
  -> unmap old virtual identity (sampling falls to coarse parent)
  -> transcode variable-rate source into three GPU BC page payloads
  -> map new identity only after every output byte exists
```

The transcode system emits two jobs per page, normally 8–16 pages or 16–32 jobs
per frame. At 16 pages/frame and 60 Hz the target is about 15 MTexel/s. Platforms
with direct texture-memory access can write BC blocks directly into final
memory; CPU and GPU implementations were both supported.

The ordering is explicit and important: old mapping is removed before overwrite,
and the new mapping is published only after transcode completes. The renderer
therefore sees a valid old/coarse page or a valid new page, never partially
replaced bytes under a new identity.

The 2009 job architecture gave latency-tolerant jobs one frame to complete. Its
virtual-texture budget was reported as roughly 8 ms of aggregate CPU work in an
illustrative 16 ms frame, distributed across available heterogeneous workers;
that is not a single-thread or fixed per-stage timing.

### LOD transition quality

If fine data arrives several mips late, simply mapping it causes a visible pop.
The proposed solution allocates the desired fine page immediately with data
upsampled from the coarse parent, then blends its texels toward the actual fine
page when available. This keeps sample positions stable during the transition.
It is more expensive than the simple coarse fallback used by Afterglow today.

### Hardware sparse follow-up

The 2013 talk examines AMD partially resident textures, but it is primarily a
design/quality analysis, not evidence that RAGE replaced its software atlas:

- hardware pages avoid software borders and enable correct cross-page filtering;
- a min-LOD texture is still needed to avoid page faults and control fallback;
- then-current hardware VTs were limited to 16k², requiring arrays for RAGE-
  sized domains;
- page texel dimensions vary by format because the hardware page is 64 KiB;
- software pages with stored borders do not directly match borderless sparse
  pages;
- then-current APIs coupled allocation and upload too late for expensive
  streaming/transcode preparation.

This reinforces that sparse hardware is not automatically a better architecture
for a portable software VT.

## What transfers to Afterglow

### Strong matches

- RAGE's source-page → transcode → physical-page pipeline matches Afterglow's
  BIG/UASTC → worker transcode → atlas upload path much more closely than
  Wicked or Dagor.
- Four-texel self-contained borders, linked material channels, pinned coarse
  coverage, depth-tested reduced feedback, desired-vs-resident priority, and
  content-before-mapping publication remain directly relevant.
- Global high/low-water feedback bias is a concrete deterministic response to
  impossible demand.
- Compressed source should remain compressed until the final transcode domain;
  an extra persistent derived cache is not required by this design.
- Offline visibility/locality analysis can reduce both source bytes and request
  fragmentation before runtime policy becomes involved.

### Do not copy

- Doom 3's camera clipmap, synchronous reads, implicit array limits, or raw
  unversioned `.mega` format.
- RAGE's frame allocations, linked lists, whole-pool sorts, or unspecified
  queue capacities into Afterglow's sealed hot path.
- A global quality bias without per-channel policy and telemetry.
- The assumption that a 2009 optical-disk cache hierarchy or platform-direct
  texture memory maps directly to browser/WebGPU constraints.
- LOD blending until a measured prototype proves its upload/transcode cost.

## Open questions requiring measurement, not assumption

1. Whether Afterglow's current clock policy plus stale cancellation already
   bounds hostile oversubscription well enough, or needs a RAGE-style global
   feedback bias.
2. Whether desired-vs-resident mip gap should precede current coverage/urgency
   in the scheduler.
3. Whether linked channels should share request admission while retaining
   independent mip and eviction policy.
4. Whether offline source-order clustering can lower no-cache bulk-request
   amplification without increasing visible batch latency.
5. Whether progressive coarse-to-fine texel blending is worth its additional
   atlas writes on the 680M.

These are prototype questions, not selected policy.
