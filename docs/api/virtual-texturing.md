# Virtual texturing

The web engine's virtual-texture implementation lives in
`crates/afterglow-web/www/engine/virtual-texture.ts`. It provides a shared
physical atlas, packed page-table entries, LRU residency, mip fallback, and a
fixed per-frame page upload budget.

> **Audit status (2026-07-15): correctness prototype, not production-ready.**
> Sustained movement exposes progressively increasing frame cost. The full
> disk-to-screen audit identifies superlinear cache maintenance, page-thread
> RPC polling, linear container lookup, missing backpressure, cache churn, and
> shader/readback costs. See
> [`../audits/virtual-texture-vertical-slice-2026-07-15.md`](../audits/virtual-texture-vertical-slice-2026-07-15.md).

## Public surface

- `VirtualTextureStore(loader, pageDataProvider?, format?, device?)`
- `loadTexture(path, options?) -> AssetHandle<THREE.Texture>`
- `loadMaterialSet({ albedo, normal?, roughness?, ao? }, options?)` loads aligned
  PBR channels and expands each albedo feedback page to every linked channel.
- `processFeedback(feedback)` accepts globally identified requests
  `{ path, mip, x, y }` and merges them into a fixed persistent scheduler.
  `poll()` dispatches and commits one bounded scheduling quantum per frame.
- `attachRenderer(renderer)` binds the actual Three.js backend textures so
  atlas-slot and packed-page-table changes use `GPUQueue.writeTexture`.
- `unloadTexture(path)` cancels pending generations and releases owned slots.
- `poll()` advances asynchronous page work.
- `getEntry(path)` exposes the read-only per-texture descriptor needed to bind
  its page table in engine materials. `atlasWidth`, `atlasHeight`,
  `atlasPagesX`, and `atlasPagesY` report the device-sized physical atlas.
- `PAGE_SIZE`, `PAGE_BORDER`, `SLOT_SIZE`, `ATLAS_WIDTH`, and `ATLAS_HEIGHT`
  expose shader/layout constants without duplicating them in clients.
- `VirtualTextureFeedbackPass` renders a supplied feedback-material scene into
  an `RG32Uint` target at reduced resolution and performs non-blocking readback.
  `consume()` returns `null` when no newer readback has completed, distinguishing
  that state from a completed readback containing zero pages. Readback decoding
  alternates two retained Maps, uses request objects pooled at `resize()`, and
  reuses fixed mip scratch; a second submit is deferred until the previous map
  is consumed so pooled records cannot be mutated under a consumer.
- `getDebugSnapshot()`, `setDebugPaused()`, `setDebugPageBudget()`, and
  `VirtualTextureDebugController` support reusable atlas and slow-residency UIs.
  Stats include ready uploads, rejected admissions, queue capacity, stage time
  budgets, and budget-exhaustion counts.
- `VirtualTextureRes` is the ECS resource definition.
- `detectBestTextureFormat(adapter?)` selects BC7, ASTC, or RGBA fallback.
- `VT_SAMPLE_WGSL` and `VT_FEEDBACK_WGSL` contain the sampling and feedback
  shader functions. `VT_RESOLVE_MATERIAL_MIP4_WGSL` resolves one level that is
resident in all four PBR page tables; `VT_SAMPLE_LEVEL_WGSL` samples each
channel exactly at that shared level. This provides atomic material visibility
even when channel completions arrive separately. Clock eviction of one material
page also removes the same logical page from every resident channel in constant
group-size work, so mixed residency cannot persist.

`AssetStore.setVirtualTextureStore(store)` enables universal VT;
`AssetStore.loadTexture()` then delegates to that store.

## Layout and encoding

Pages contain a 128x128 payload and a four-texel border on every side. A page
therefore occupies a 136x136 atlas slot. Every virtual mip table is packed
vertically into mip zero of an `r32uint` GPU texture. A resident entry is:

```text
1 | (physical_x << 1) | (physical_y << 9)
```

Bit zero is the resident flag. Sampling starts at the derivative-selected mip
and walks toward coarser mips until it finds a resident page. Mips smaller than
128x128 (64 through 1 texel) are packed with individual four-texel borders into
one additional pinned 136x136 mip-tail slot; its entry occupies the otherwise
unused x=1 texel of the terminal page-table row. Atlas sampling
uses explicit translated gradients and supports clamp, repeat, and mirrored
repeat addressing.

Feedback derivatives are evaluated directly in the reduced render target and
accept an explicit additive LOD bias. The dungeon uses `-1.5`, intentionally
requesting one to two levels finer than the strict screen footprint without the
old accidental fixed three-level bias. The dungeon submits feedback every four
frames: at 144 Hz this is still a 36 Hz residency update while avoiding a GPU
readback and second scene pass on every presented frame. Completed worker jobs
are also committed through a four-page-per-frame upload queue rather than
issuing an unbounded burst of atlas and page-table writes between frames.

Feedback uses `RG32Uint`: word zero stores valid, six mip bits, and eleven bits
for each page coordinate; word one stores the full virtual-texture ID. This
supports the 2048x2048 page grid of a 256K texture without aliasing identities.

## Residency and frame budgets

The physical cache uses an O(1) key-to-slot lookup and a fixed second-chance
clock. Touches set one reference bit; they never scan slots, splice an LRU array,
or rebuild an index. CPU residency reads/writes index the packed page-table
`Uint32Array` directly; the former string-keyed duplicate `Map` was removed. The free-slot stack and ready-upload queue use preallocated
typed/ring storage. Eviction performs at most two fixed-capacity clock passes and
never selects pinned or reserved slots.

Feedback expansion, material-channel deduplication, and capacity-bias fitting
write into preallocated request records plus a fixed numeric scratch map; they
do not construct per-feedback Maps, channel objects, or string keys. A fixed-
capacity persistent scheduler retains feedback that does not fit in one frame's
dispatch budget. Requests unseen for sixteen newer feedback snapshots are
removed; visible requests refresh their age. At the dungeon's 36 Hz feedback
rate this covers the measured ~430 ms worst-case page latency. Cancellation is propagated through
an `AbortSignal`: queued jobs stop before read/transcode stage boundaries, while
an already-running one-in-flight RPC is allowed to finish and its generation is
discarded. A fixed 64-entry serial transcode ring matches that RPC constraint
without an unbounded Promise chain and copies reusable RPC scratch before the
next dispatch. Physical slots are acquired only
after page bytes are ready, so a slow range read or transcode cannot evict useful
resident data while it is pending.

Runtime work is bounded to 64 pending pages and 8 MiB of expected output. The
pending table, ready-upload ring, scheduler, cache slots, and feedback scratch
are preallocated; hot identity uses numeric `textureId`/packed page keys. Path
Maps remain only for load/unload and game-facing lookup. The
scheduler capacity equals the physical atlas capacity. Scheduling checks a
0.25 ms budget in small batches; upload commits are limited to four pages and a
0.35 ms budget per `poll()`. Rejected admissions, stale cancellations, cache
hits/misses/evictions, queue bytes, read latency, upload CPU time, and budget
exhaustion are exposed by `getStats()`. `getStats()` updates and returns one
stable preallocated object and is safe for per-frame telemetry; the allocating
`getDebugSnapshot()` is intended only for explicit diagnostics.

## Asset containers

Container version 5 stores each virtual texture in a compact
`VirtualTextureDirectory`. Every mip is one contiguous row-major block with one
absolute block offset, page-grid dimensions, and a vector of encoded page
sizes. The optional tail stores one offset and size. It does **not** serialize a
full `ChunkInfo` or repeated mip/x/y/encoding metadata per page.

`parseBigHeader()` admits this directory once. `createPageDataProvider()`
expands each compact size vector into fixed `Float64Array` offsets and
`Uint32Array` sizes, after which page lookup is direct `y * pagesX + x`
indexing. The production nine-channel dungeon header fell from 764,192 bytes in
v4 to 123,768 bytes in v5, safely below the 1 MiB RPC output limit. v4 is
rejected; bundled assets were rebuilt rather than retaining compatibility.

`afterglow-pipeline process` decodes PNG/JPEG sources, accepts arbitrary
positive width and height (including rectangular, non-power-of-two, and
sub-page dimensions), generates filtered mips, and writes independently
seekable partial-edge 136x136 bordered pages plus a packed mip tail. The
offline-only `afterglow-basis-encoder` crate encodes every slot independently as
UASTC Basis. `stream_virtual_texture()` walks one mip at a time and retains at
most 64 bordered pages. The writer moves payloads without cloning, compresses
each chunk once into a temporary disk spool, encodes each bounded batch in
parallel, and assembles stream order with a fixed 64 KiB copy buffer instead of
retaining the complete raw page set or encoded container payload in RAM.
Runtime `afterglow-texture` transcodes Basis pages to BC7, ASTC, or
RGBA. Raw RGBA pages remain explicitly tagged and bypass the transcoder.

## Demonstrations

The dungeon's scanned 8K PBR sources are cached as a generic `.big` container
under `/tmp`, then loaded through `AssetLoader` range reads and transcoded from
Basis to the active GPU format by `TextureWorker`. `MeshStandardNodeMaterial`
keeps Three.js's PBR implementation; VT nodes supply albedo, tangent-space
normal, and a packed linear mask page (`R=roughness`, `G=ambient occlusion`).
`afterglow-pipeline pack-masks <roughness> <ao> <output>` performs this offline,
reducing each material from four streamed channels to three and sharing one
shader sample for both masks. The shared atlas remains linear for data channels, while the albedo node applies Three.js `sRGBTransferEOTF` explicitly
before feeding the PBR color input. Basis transcoding runs in a real Web Worker
through the shared-memory ring transport, never in the page's frame loop. The
demo uses optimized `wasm-release` workers.

Both demonstrations use the engine `VirtualTextureStore`, `VT_SAMPLE_WGSL`,
packed page tables, request scheduler, incremental writes, and one physical
atlas sized to the largest whole 136×136-slot grid allowed by the GPU's reported
`maxTextureDimension2D`. On the current adapter this is 8,160×8,160 (60×60 =
3,600 slots, approximately 254 MiB RGBA8). Neither demo has a private cache or
page-table implementation.

`afterglow-cef --example vt-demo` displays one procedural 262,144×262,144
terrain texture (256 GiB logical RGBA). It supports WASD pan, overview and
one-texel zoom plus deterministic programmatic camera control.

`afterglow-cef --example vt-dungeon` is a first-person corridor dungeon using
three downloaded 8K PBR sets across twelve wall instances. Two sets are
8192×8192 and one is natively rectangular at 8192×4096. Albedo, OpenGL normal,
roughness, and AO pages stream together through linked material feedback while
all walls share the engine atlas. Interactive
controls are WASD, Shift sprint, pointer-lock mouse look, reset, and three test
poses. `window.__afterglowVtDungeon` exposes `setProgrammatic`, `setPose`,
`getPose`, `move`, `look`, `step`, `waitForIdle`, allocation-free `telemetry`,
`errorCount`, `snapshot`, and `runScenario`.

```sh
nix-shell shell.nix --run "cargo build --example vt-demo -p afterglow-cef"
DISPLAY=:0 ./scripts/run-vt-dungeon.sh
DISPLAY=:0 ./scripts/test-vt-dungeon-gpu.sh
# With the dungeon already running; writes raw per-second CDP samples:
./scripts/soak-vt-dungeon.sh 600 traverse vt-traverse-10m.log
./scripts/soak-vt-dungeon.sh 600 thrash vt-thrash-10m.log
# Deterministic occupancy states (run cold → half → full → churn in one process):
./scripts/baseline-vt-atlas.sh half vt-atlas-half.log
```

## Atlas-state baseline

The 2026-07-16 real-GPU baseline filled all 3,600 physical slots and then
performed 1,014 cumulative group-aware evictions. Cold, half, and full states
had 6.955 ms maximum rAF intervals at 144 Hz. Churn averaged 6.970 ms with a
20.850 ms maximum and one interval above 17 ms; load failures, queue overflows,
long tasks, and GPU errors remained zero. WebGPU timestamp queries measured the
latest full-state main/feedback contexts at 0.149/0.018 ms. See
`docs/benchmarks/vt-atlas-baseline-2026-07-16.md` and its raw logs.

Corrected 10/30/60-minute soaks produced 86,325 / 258,978 / 517,961 frames.
Mean timing remained 6.950 ms in stable, traversal, and per-frame teleport
modes; failures, queue overflow, long tasks, GPU errors, pending work, and
post-seal pipeline creation were zero. GC-floor heap stayed near 77–79 MiB.
See `docs/benchmarks/vt-soak-2026-07-16.md`. GPU timestamp tracking is disabled
during long soaks because Three r185 retains diagnostic timestamp keys by
frame; resolved keys are explicitly cleared during short GPU captures.

## Real-GPU regression

`scripts/test-vt-gpu.sh` launches the CEF/WebGPU demo and executes its CDP
self-test. It performs three independent CEF launches. Within every launch it
renders and precisely reads back 1,024 `RG32Uint` feedback pixels from eastward,
westward, and rotated raster directions; follows eastbound, westbound, and
diagonal trajectories across overview, paged, and one-texel LODs; byte-verifies
three RGBA writes at top-left, bottom-right, and interior locations; and submits
three supported compressed writes at corresponding atlas locations. Uncaptured
WebGPU validation errors and device loss fail the test.

```sh
DISPLAY=:0 ./scripts/test-vt-gpu.sh
```

On fox-laptop's Radeon 680M/RADV adapter, BC7 was selected. All three independent
launches passed; each launch validated all three directional feedback scenarios,
all three residency trajectories, three exact RGBA readbacks, and three BC7
uploads.

600-frame rAF measurements at 1440×900 held 59.97 FPS with zero drops during
stable rendering, bidirectional panning, and continuous streaming. Pathological
12-way per-frame teleports and full-cache 20-way thrashing each dropped one of
600 frames (p99 16.68 ms, max approximately 33.35 ms). Canonical methodology and
full results are in `docs/research/performance-benchmarks.md`.
