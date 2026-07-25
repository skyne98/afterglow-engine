# Wicked Engine and Dagor Engine virtual texturing — comparative source audit

Date: 2026-07-25

Source revisions:

- Wicked Engine [`3a800b7134aafe58461093c8abb2e274d4e64033`](https://github.com/turanszkij/WickedEngine/tree/3a800b7134aafe58461093c8abb2e274d4e64033)
- Dagor Engine [`f8935cb690f8397d6eaa0d24c37c44f8d714702f`](https://github.com/GaijinEntertainment/DagorEngine/tree/f8935cb690f8397d6eaa0d24c37c44f8d714702f)

This is a static source audit, not a benchmark. The repositories were cloned
with `--depth 1 --filter=blob:none` into `/tmp`, verified at the hashes above,
searched broadly for virtual/sparse/tiled/clipmap/feedback terminology, then the
complete core C++/shader paths and backend entry points were read. An independent
second source review checked the numerical derivations and corrected misleading
initial readings (notably Wicked's sparse aliasing and Dagor's three distinct
"virtual texture" names). Detailed evidence and complete file inventories are
in:

- [Wicked Engine terrain VT](wicked-engine-virtual-texturing-source-audit.md)
- [Dagor Engine clipmap VT](dagor-engine-virtual-texturing-source-audit.md)

## Executive conclusion

Neither engine implements the same product as Afterglow's asset virtual
texturing. Both systems are **runtime terrain render caches**:

- Wicked gives every near terrain chunk a finite 65,536² logical texture. GPU
  feedback selects pages; a CPU allocator assigns a shared atlas slot; compute
  shaders blend source materials and directly emit BC blocks.
- Dagor maintains a camera-relative, multi-mip 128×128 clipmap address window.
  screen feedback or CPU frustum coverage selects pages; CPU policy assigns
  ordinary atlas slots; an orthographic landmesh/decal pass renders each page,
  followed by optional GPU compression.

Neither reads pre-cooked page payloads, addresses a page container, performs
page-level file/network I/O, or runtime-transcodes pre-authored VT tiles. Both
also expose low-level hardware sparse/tiled-resource APIs, but neither terrain
VT uses hardware sparse mapping as its demand-residency/page-table mechanism.

Dagor has the more complete demand-control policy: compacted screen feedback,
fixed request arrays, an explicit 8/4-page game update budget, camera/coarse
synthetic demand, bounded cascade scoring, three levels of visual fallback,
and strong debug views. Wicked is much simpler but has no page-update budget or
screen-impact priority and scans/republishes large per-chunk page domains.

## Side-by-side architecture

| Property | Wicked Engine | Dagor Engine |
|---|---|---|
| Product scope | Procedural terrain chunks only | Terrain/landmesh, decals, prefab/rendinst land-class extensions |
| Logical domain | Per-near-chunk finite 65,536² image | Camera-relative infinite-world clipmap window |
| Virtual page | 256² logical texels | Normally 248² useful texels inside a 256² page at anisotropy 4 |
| Address dimensions | 256×256 pages at LOD0, through 1×1, plus tail | 128×128 addresses × up to 11 virtual mips × terrain/RI offset |
| Physical coordinates | 62×62 = 3,844 shared slots; Apple fallback 62×31 | hard cap 1,024; game default 512 desktop compressed / 128 mobile compressed |
| Current channels | four: BC1 base, BC5 normal, BC3 surface, BC1 emissive | three: BC3/BC5/BC3 or ETC2/uncompressed equivalents |
| Nominal final cache memory | about 768 MiB non-Apple sparse-alias backing | about 96 MiB current compressed desktop final caches |
| Page producer | compute blendmap/material sampler + BC encoder | orthographic landmesh/decal render callback + compressor |
| Disk tile streaming | none | none |
| Demand input | per-chunk R32Uint minimum-LOD image | pixel-UAV compact feedback, legacy feedback RT, or CPU frustum walk |
| GPU compaction | fixed hierarchy reduction then scans every logical page | screen buffer → histogram → compact bounded tile list |
| CPU membership | owning-pointer comparison in physical slot | fixed full-domain bitset plus tag maps |
| Replacement | frame-age sort; oldest unused slot first | retain demanded entries; recycle superfluous coordinates |
| Update budget | none beyond available physical slots | game default 8 desktop / 4 mobile; config setter is unchecked |
| Coarse guarantee | last non-packed page + six-mip tail touched every CPU update | synthetic camera/RI demand; coarser indirection; fallback pages; last clip |
| Page table | per-chunk RGBA8Uint mip-chain residency maps | one RGBA8 indirection render target |
| Publication | page upload → residency compute → tile writes in one ordered compute submission | render/compress page first; then rebuild dirty indirection |
| Cache mip strategy | virtual hierarchy + six packed sub-256 tail images | virtual hierarchy; current final cache usually has one hardware mip; no packed tail |
| Readback depth | two allocation/page ring entries | three capture/fence/readback entries |
| Main policy complexity | whole-domain page publication and physical-slot sort | bounded arrays plus sort/maps; cascade candidate adjustment bounded to H≤32 |
| Runtime allocation | vectors, sort, dynamic GPU uploads | frame allocator/hash maps, sorting, global feedback job; many fixed caps |
| Automated VT tests found | none | none |

Memory figures are derived from current dimensions/formats and exclude API
alignment, metadata, staging, compression buffers, residency maps, and source
textures. They are architecture comparisons, not measured resident-memory data.

## End-to-end timelines

### Wicked

```text
prior opaque sampling
  -> per-chunk R32Uint feedback InterlockedMin
  -> compute reduces feedback into per-page minimum request LOD
  -> compute scans all pages and emits x/y/lod allocation records
  -> copy queue writes two-slot readback
  -> later CPU job reads a safe slot
       touches resident pages
       rebuilds/sorts reusable physical slots
       allocates misses
       writes complete page map
  -> copy queue uploads page map
  -> async compute rebuilds residency-map mip chains
  -> same async-compute command list bakes all queued four-channel tiles
  -> later terrain sampling resolves residency and manually blends two LODs
```

The coarse pages mask feedback latency. Reused mappings are visible only after
queue ordering completes both the residency and tile work, but unlike Dagor the
source writes the new residency map before writing tile content in that command
list. Correctness therefore relies on submission/queue synchronization, not an
explicit table-after-content publication operation.

### Dagor

```text
normal ground/static rendering
  -> pixel UAV writes one packed request per feedback pixel
  -> compute clears histogram, downsamples/accumulates, compacts tile list
  -> submit one of three readback/fence slots
  -> later prepareFeedback selects a completed capture (newest ready may win)
       copies and validates readback
       worker or current thread merges synthetic demand
       sorts and applies bounded admission/replacement policy
  -> following prepareRender renders selected orthographic land pages
  -> batches up to four pages, generates requested cache mips, compresses/copies
  -> marks page cached
  -> rebuilds dirty indirection render target coarse-to-fine
  -> terrain shaders sample atlas via explicit gradients
       and blend to fallback page / last clip when needed
```

The three-slot ring deliberately allows stale captures to be skipped. It also
has an explicit stall policy: after repeated fence misses it warns and locks the
oldest entry. This is a clear failure policy, though not a no-stall one.

## Feedback and prioritisation

### Wicked's hierarchy is cheap to understand, expensive to scale

Each source pixel writes an integer LOD minimum into a per-chunk feedback map.
The first compute pass directly writes LOD0/LOD1 requests and atomically reduces
LOD2+. A second pass dispatches over every page in the virtual hierarchy. It
emits requested resident and missing pages alike; CPU pointer ownership filters
resident ones.

Advantages:

- compact 32-bit feedback value;
- no CPU scan of a full screen feedback image;
- coarse-parent requests are generated on GPU;
- one feedback stream drives all four linked terrain channels.

Costs:

- feedback, request, allocation, page, residency, and ring resources exist per
  near chunk;
- a 65,536² chunk has 87,381 normal pages plus one tail entry;
- the CPU republishes every page mapping and the residency shader traverses the
  hierarchy for every base map texel;
- no operation/page/time budget prevents a burst from generating thousands of
  BC tile updates in one frame.

### Dagor compacts aggressively, then applies explicit policy

The UAV path first stores a packed request per reduced screen pixel. Compute
then builds a 180,224-entry terrain histogram (2,883,584 entries with RI domain)
and compacts only non-zero entries into a 3,176-entry capture list. CPU merges
up to 920 reserved synthetic requests, sorts by weighted feedback, and retains
at most the physical cache domain.

Admission order is deliberate:

1. visible synthetic/fake demand;
2. a reserved fraction for stale cached pages needing rerender and invisible
   synthetic pages;
3. normal misses;
4. if misses exceed the remaining small budget, cascade scoring chooses pages
   that maximize improvement considering available coarser pages.

The cascade candidate set is at most 64 and chosen pages at most 32. The
candidate-adjustment portion is bounded O(H²), H≤32; it is not an occupancy-
sized quadratic scheduler.

## Residency and fallback

Wicked stores exact atlas xy, the first usable coarser LOD, and packed-tail xy
in each residency entry. Sampling computes LOD from derivatives, reads the
appropriate residency-map mip, remaps into the padded physical tile, samples
two virtual LODs at atlas LOD0, and interpolates manually. The pinned 256² page
and tail normally guarantee a chain. The source lacks a defined shader branch
for complete bootstrap allocation failure, and `SampleBias` currently applies
its bias twice.

Dagor paints coarse mappings over broad fine-grid table regions and then lets
finer resident pages overwrite them. Sampling computes a minimum safe table
mip for the moving address window, fetches cache xy/source mip, rescales
explicit gradients, and samples three linked channels. Near the end of its
virtual hierarchy it blends first to separately rendered fallback pages and
then to a conventional last-clip texture. This gives more explicit degradation
than Wicked at the cost of more policy and shader state.

## Sparse-resource findings

### Wicked

Wicked has complete generic D3D12, Vulkan, and Metal sparse APIs. Terrain makes
both a compressed atlas image and a quarter-dimension raw BC-block UAV sparse,
then maps **the entire two resources** to the same tile-pool range exactly once.
Compute writes raw block words and the compressed view samples the same bytes.
Logical VT page allocation never calls `SparseUpdate`.

Apple disables this route because sparse block-compression aliasing is reported
not to work there. It halves atlas height, allocates separate raw/final images,
and copies updated regions. This is a format-alias portability fallback, not a
second residency design.

### Dagor

Dagor's `TileMapping`/`TextureTilingInfo` API and DX12/Vulkan backends implement
manual sparse page and packed-tail mapping. The clipmap cache does not set
`TEXCF_TILED_RESOURCE`, query tiling, or call the mapper. It uses ordinary
render-target/update-destination textures and its own RGBA indirection table.
The daFrameGraph `VirtualTextureRequest` name is also unrelated: it is a graph
virtual-resource handle, not virtual texturing.

## Robustness and source-level concerns

### Wicked

- no per-frame tile-update, operation, or byte budget;
- full dynamic vectors for allocation/update work;
- whole-page-map CPU publication and full reusable-slot sort every update;
- no explicit no-resident shader fallback if mandatory coarse allocation fails;
- `SampleBias` applies bias twice in the VT branch;
- fixed ten-entry LOD arrays/nine residency UAVs exactly match the current
  65,536² configuration;
- no dedicated VT tests found.

### Dagor

- the public update-budget setter accepts negative, zero, and >1,024 values;
- feedback collection has an intentional eventual blocking-lock fallback;
- one file-static feedback job serializes all `ClipmapImpl` instances;
- sorting/hash-map/frame-allocation work scales with captured demand;
- point/box invalidation is terrain-only, with RI invalidation left TODO;
- consistency validation is compiled as an immediate return;
- apparent CPU/shader off-by-one: C++ accepts RI offsets 0..15, while the
  histogram shader's `MAX_RI_OFFSET=MAX_RI_VTEX_CNT=15` loops/tests admit 0..14;
- no dedicated clipmap VT tests found.

The RI discrepancy is a source-level concern, not a demonstrated game bug. A
full-slot GPU-feedback test is required before concluding slot 15 is reachable
and broken.

## Transferable lessons for Afterglow

### Adopt

1. **Dagor's explicit multi-tier fallback.** Exact page → coarser page → small
   local fallback → conventional coarse texture is resilient to feedback and
   streaming latency.
2. **Dagor's GPU compaction before readback.** Do not transfer a screen-sized
   feedback image when fixed-size compact requests suffice.
3. **Dagor's reserved synthetic demand.** Camera-near and coarsest coverage are
   explicit policy inputs, not accidental results of feedback.
4. **Bounded improvement scoring.** The cascade score is a useful model for
   choosing between one fine page and a coarser page benefiting multiple
   samples, provided the candidate domain remains tiny and fixed.
5. **Wicked's one-demand-stream linked-channel model.** Shared material demand
   avoids four duplicate feedback writes; channel quality/residency policy can
   still differ in the consumer.
6. **Wicked's compact packed-tail layout.** Packing sub-page mips into one
   guaranteed physical tile is efficient for finite asset VTs.
7. **Both engines' render-before-use intent.** A physical slot must not become
   sampleable with replacement identity until its new bytes are complete.

### Do not copy

1. Runtime allocation, full scans/sorts, unchecked capacities, or monolithic
   ownership into Afterglow's sealed hot path.
2. Wicked's unbounded page-update burst or per-chunk 87k-entry publication.
3. Dagor's global feedback job/static mode state.
4. Implicit CPU/shader constants without generated/shared contract tests.
5. Hardware sparse APIs merely because the feature is named virtual texturing;
   neither engine demonstrates that sparse residency improves this cache.
6. Any assumption that these systems validate file-backed asset VT. They do
   not contain the relevant I/O, cancellation, stale-generation, transcode, or
   container behavior.

### Afterglow-specific interpretation

Afterglow's current file-backed page path already solves a different problem:
source-indexed BIG reads, worker transcode, bounded admission, atlas upload,
and exact page-table publication. The useful research outcome is therefore not
to replace it with either terrain cache. The strongest candidates for measured
prototypes are:

- coverage-weighted GPU request compaction inspired by Dagor;
- a tiny fixed cascade-benefit selector for overload frames;
- an explicit conventional coarse/fallback texture beyond per-page parent walk;
- generated CPU/TypeScript/WGSL packing contracts and all-sentinel tests;
- separate telemetry for requested, compacted, admitted, stale-canceled,
  rendered/transcoded, uploaded, and first-sample-eligible stages.

These are technical prototype candidates, not selected engine policy. Any
change to current page priority, fallback quality, channel coupling, or GPU
feedback ABI needs its own measured acceptance gate.
