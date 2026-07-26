# Virtual texturing

The web engine's virtual-texture implementation lives in
`crates/afterglow-web/web/src/engine/virtual-texturing/virtual-texture.ts`. It provides a shared
physical atlas, packed page-table entries, LRU residency, mip fallback, and a
fixed per-frame page upload budget.

> **Audit status (updated 2026-07-25): bounded no-cache correctness
> prototype, not yet a fully validated two-target release.** Persistent derived
> caching was removed; admission, queueing, deadlines, and feedback cadence are
> explicit. The live BIG provider still does not wire the source-sorting helper,
> and native-shell Dungeon validation remains open. See
> [`../audits/virtual-texture-vertical-slice-2026-07-15.md`](../audits/virtual-texture-vertical-slice-2026-07-15.md).

## Public surface

- `VirtualTextureTuning(config?)` owns the central bounded atlas/page-table
  commit policy. `VirtualTextureTuningRes` is the ECS resource definition.
  Configuration is partial: omitted fields retain the defaults, including when
  only `atlasMaxDimension` is supplied. It starts at two pages / 0.20 ms,
  tightens after repeated overloaded rAF samples with backlog, then cautiously
  probes toward configured device caps after stable windows; a bad promoted cap
  immediately returns to the independently validated baseline.
  `atlasMaxDimension` caps the physical atlas to the largest whole 136-texel
  slot grid at or below that dimension; zero uses the device limit.
- `VirtualTextureStore(loader, capacities, pageDataProvider?, format?, device?,
  tuning?, telemetry?)` requires `VirtualTextureRuntimeCapacities {
  maxPendingPages, maxPendingBytes }` during bootstrap and accepts the shared
  tuning instance. `recordFrameTime(milliseconds)` feeds the tuner without
  allocation; `getStats()` exposes active limits and downshift/recovery
  counters.
- `loadTexture(path, options?) -> AssetHandle<THREE.Texture>`
- `loadMaterialSet({ albedo, normal?, masks?, roughness?, ao?, emissive? }, options?)`
  loads aligned PBR channels and registers one albedo-driven stream policy.
  `options.mipBiases` is material-configurable; omitted roles use albedo `0`,
  normal/emissive `+1`, and masks/roughness/AO `+2`.
- `linkMaterialSet(set, mipBiases?)` applies that stream policy to already-loaded
  aligned entries. One albedo feedback request expands into independently
  resident channel requests. If an albedo is shared by several materials, the
  finest requested bias per channel wins.
- `processFeedback(feedback)` accepts globally identified requests
  `{ path, mip, x, y, screenPriority?, coverage? }` and merges them into a
  fixed persistent priority scheduler. `screenPriority` ranges from 0 at the
  screen center to 255 at the edge/corners; `coverage` is the feedback-pixel
  count. `VirtualTextureFeedbackPass` supplies both automatically. `poll()`
  dispatches and commits one bounded scheduling quantum per frame.
- `attachRenderer(renderer)` binds the actual Three.js backend textures so
  atlas-slot and packed-page-table changes use `GPUQueue.writeTexture`. Packed
  page-table writes reuse one preallocated `Uint32Array` instead of creating
  per-update subarray views. Atlas page uploads require bytes backed by an owned
  `ArrayBuffer`; a `SharedArrayBuffer` view is rejected rather than passed to an
  incompatible WebGPU queue boundary.
- `unloadTexture(path)` cancels pending generations and releases owned slots.
- `dispose()` idempotently unloads every texture, disposes page-table/atlas
  textures, and clears native GPU references; `BigAssetSession.close()` owns it.
- `poll()` advances asynchronous page work.
- `getEntry(path)` exposes the read-only per-texture descriptor needed to bind
  its page table in engine materials. `atlasWidth`, `atlasHeight`,
  `atlasPagesX`, and `atlasPagesY` report the device-sized physical atlas.
- `PAGE_SIZE`, `PAGE_BORDER`, `SLOT_SIZE`, `ATLAS_WIDTH`, and `ATLAS_HEIGHT`
  expose shader/layout constants without duplicating them in clients. Consumers
  express atlas caps as an integer slot count multiplied by `SLOT_SIZE`; the
  dungeon profile uses `53 * SLOT_SIZE` (2,809 physical slots).
- `VirtualTextureFeedbackCoordinator(renderer, store, capacities)` is the
  application-facing feedback owner. Capacities explicitly bound registered
  renderables and total channel passes; `cadenceMs` is the monotonic submission
  interval and `scale` selects target size. A
  `FeedbackRenderable` supplies its scene/camera, fixed pass count, active state,
  and begin/end material hooks. The coordinator preallocates all low-level
  passes, owns resize/warm/seal/disposal, disables and restores shadows, and
  restores render targets/material state with `try/finally`. It implements both
  `EngineRenderPass` and `RenderWorkerInput`: rendering submits one logical
  snapshot and worker polling publishes it only after every asynchronous channel
  readback completes. Partial snapshots are discarded, never used to cancel
  another channel's pages. Registration returns explicit invalid-count,
  capacity, or sealed statuses. Stable telemetry reports submitted, completed,
  discarded, deferred, and active-pass counts.
- `VirtualTextureStore.processFeedbackBatch(maps, count)` merges several maps as
  one visibility epoch using preallocated expansion/deduplication scratch. It
  does not build a transient merged `Map`.
- `VirtualTextureFeedbackPass(scale?)` is the low-level target/readback primitive
  used by the coordinator. It renders a supplied feedback-material scene into an
  `RG32Uint` target and performs non-blocking readback.
  `resize(displayWidth, displayHeight)` updates the target and stable
  `pixelScale: Vector2` (`feedback pixels / physical display pixels`) used by
  feedback shaders for exact derivative correction. `consume()` returns `null`
  when no newer readback has completed, distinguishing that state from a
  completed readback containing zero pages. Readback decoding alternates two
  retained Maps, uses request objects pooled at `resize()`, and reuses fixed mip
  scratch. Duplicate pixels retain the closest-to-center sample and accumulate
  bounded screen coverage. `canSubmit` supports atomic preflight, and render
  target restoration is exception-safe.
- `getDebugSnapshot()`, `setDebugPaused()`, `setDebugPageBudget()`, and
  `VirtualTextureDebugController` support reusable atlas and slow-residency UIs.
  Stats include ready uploads, rejected admissions, queue capacity, stage time
  budgets, and budget-exhaustion counts.
- `VirtualTextureRes` is the ECS resource definition.
- `detectBestTextureFormat(adapter?)` selects BC7, ASTC, or RGBA fallback.
- `VT_SAMPLE_WGSL` and `VT_FEEDBACK_WGSL` contain the sampling and feedback
  shader functions. `VT_DESIRED_MIP_WGSL` calculates one channel's requested
  mip from stable derivatives plus its explicit bias.
  `VT_SAMPLE_FROM_LEVEL_WGSL` starts a displaced sample at that channel's level,
  walks only its page table to coarser resident pages, then uses its pinned tail
  while retaining gradients from a separate continuous base UV. Albedo, normal,
  emissive, and scalar data may therefore resolve different mip levels. Clock
  eviction removes only the selected physical page.

`AssetStore.setVirtualTextureStore(store)` routes subsequent `loadTexture()`
assets into VT storage, but a shared atlas texture is not directly sampleable as
a conventional `material.map`. Materials must use VT page-table nodes.
`createVirtualGltfMaterialPair(THREE, store, set, feedbackPixelScale, options)`
builds matched visible/feedback `MeshStandardNodeMaterial` variants for glTF
base color plus optional normal and packed metallic/roughness channels. Aligned
channels share one albedo feedback stream but sample and evict independently;
differently sized channels emit separate feedback and also sample independently. It works with Mesh,
InstancedMesh, SkinnedMesh, and morph targets because Three retains its normal
geometry vertex path. Animated/deformed objects must render feedback with the
same object (temporarily swapping the prewarmed material), not a bind-pose proxy.

The tree-shakeable public barrel is `web/src/engine/virtual-texturing/index.ts`.
`parseGLTFAsset()` records `GLTFParser.associations` as a stable
`materialIndices` map. `VirtualGltfBinding.create(asset, store, options)` uses
those indices—not material names—to join primitives to cooked texture layouts.
It has an explicit primitive capacity, creates one pair per source material,
and preserves standard scalar/color/alpha/depth/side factors plus factor-only
KHR material transmission through a physical node material. Source texture UV
channels, Three's glTF/KHR texture matrix, and repeat/clamp/mirror address modes
are applied identically in visible sampling and feedback derivatives. Linear
sampling uses explicit gradients; all-nearest glTF samplers use integer atlas
loads. Mixed min/mag modes and asymmetric S/T wrapping cannot be represented by
the shared atlas and fail during bootstrap instead of rendering approximately.
The binding disposes replaced imported textures and materials
and atomically restores primitive visibility/materials around every feedback
pass. Materials without virtual base color remain visible
normally and are hidden only during feedback. `VirtualMaterialBinding` provides the same prewarmed visible/feedback ownership
for one procedural mesh without inventing a glTF asset. The public procedural
store factory likewise keeps direct `VirtualTextureStore` construction out of
visual entrypoints.

Missing indices, duplicate
layouts, unsupported/non-PBR material replacement, unavailable images, and
capacity overflow fail during bootstrap with rollback. Imported textures shared
with an unreplaced material remain owned by that live material; only exclusive
replaced images are released. Disposing a binding hides its replaced meshes
before disposing their VT materials, so no visible object retains a disposed
material. A metadata-free fallback scene remains renderable and contributes no
feedback.

### Material-channel stream policy

Material feedback identity remains the albedo texture ID for aligned sets. The
store expands that request through a bootstrap-only fixed channel descriptor:
texture ID, mip bias, and channel class. One hundred thirty-two fixed scheduler
lanes first separate urgent parent restoration from exact-quality promotion,
then preserve albedo → normal/emissive → scalar-mask and coarse/center ordering
inside each tier. Waiting-request aging is clamped to its tier/channel floor and
cannot make a 16 ms quality promotion outrank pending urgent restoration.

Residency and eviction are not grouped. Each channel walks its own page table,
falls back to its own pinned tail, and consumes or releases one physical slot.
The default `0/+1/+2` biases reduce normal page demand by roughly four and scalar
page demand by roughly sixteen for equal-size textures. Shared albedo stream
registrations merge channels and retain the finest bias required by any user.

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
uses explicit translated gradients and selects clamp, repeat, or mirrored
repeat addressing in the shader. **Known cook limitation:** stored border
texels always clamp at the global image edge. Repeat/mirrored-repeat filtering
therefore does not yet have correct opposite-edge border data at the global
texture seam; those modes are not currently seam-correct.

`VirtualTextureFeedbackPass.pixelScale` records the exact feedback-target pixels
per physical display pixel after every resize, including ceil-rounding and
non-square dimensions. `VT_FEEDBACK_WGSL` multiplies its reduced-target X/Y
derivatives by that vector before calculating `floor(log2(texelFootprint))`.
With quality bias zero, the selected discrete mip retains approximately one to
two source texels per physical screen pixel regardless of source resolution,
DPR, feedback scale, or UV tiling. Quality bias is a separate explicit control;
feedback-scale correction is never hand-tuned into it.

Feedback receives `sampleUV` separately from continuous `gradientUV`. Repeat
addressing and POM displacement affect page coordinates, while mip derivatives
remain free of wrap/control-flow discontinuities. Dungeon prewarms matching
base and POM-aware feedback materials; when POM is active, the reduced pass runs
the same bounded height march and requests the page actually sampled by the
visible displaced surface. The pass uses a 55 ms monotonic cadence rather than
a refresh-dependent frame count. Completed worker jobs are committed through
the centrally tuned bounded upload queue rather than
issuing an unbounded burst of atlas and page-table writes between frames.

Feedback uses `RG32Uint`: word zero stores valid, six mip bits, and eleven bits
for each page coordinate; word one stores the full virtual-texture ID. This
supports the 2048x2048 page grid of a 256K texture without aliasing identities.

## Residency and frame budgets

The physical cache uses an O(1) key-to-slot lookup and a fixed second-chance
clock. Atlas capacity is a bootstrap choice and does not alter upload tuning:
constructing `VirtualTextureTuning({ atlasMaxDimension })` preserves the default
finite upload count and time budget. A regression test loads and commits pages
through a constrained 4×4-slot store to enforce that invariant. Touches set one reference bit; they never scan slots, splice an LRU array,
or rebuild an index. CPU residency reads/writes index the packed page-table
`Uint32Array` directly; the former string-keyed duplicate `Map` was removed. The free-slot stack and ready-upload queue use preallocated
typed/ring storage. Eviction performs at most two fixed-capacity clock passes and
never selects pinned or reserved slots.

Feedback expansion, material-channel deduplication, and capacity-bias fitting
write into preallocated request records plus a fixed numeric scratch map; they
do not construct per-feedback Maps, channel objects, or string keys. A fixed-
capacity persistent scheduler retains feedback that does not fit in one frame's
dispatch budget. For each missing desired page it emits at most two requests:
an urgent mip+2 parent (clamped to the terminal paged mip) and the exact page.
The parent uses a non-resettable 1 ms maximum batch window; exact promotion uses
a non-resettable 16 ms maximum window. Existing resident coarser pages and the
pinned tail remain immediately sampleable. Tier/channel/quality/center lanes
avoid sorting and allocation; visible waiting requests age only within their
tier/channel floor.

Requests absent from two newer feedback snapshots are removed. Dungeon submits
on a 55 ms monotonic cadence, so expiry is approximately 110 ms plus discrete
frame/readback timing at any refresh rate. Newly important center/coarse-
restoration work marks a strictly worse non-pinned load canceled when the
16-entry table is full, but the
slot remains occupied until the asynchronous stage acknowledges cancellation;
this is not immediate replacement. Cancellation propagates through an
`AbortSignal`: queued jobs stop before read/transcode stage boundaries, while an
already-running read or one-in-flight RPC may finish and its stale generation is
discarded. HTTP Fetch and the CEF bridge do not currently receive the signal or
apply a response deadline, so a stalled transport can retain bounded capacity
indefinitely.
`createFetchRangeLoader().readBulk()` automatically selects the bounded CEF
shared-message bridge when its private renderer binding exists; web targets
issue a standard HTTP multi-range request and parse one
`multipart/byteranges` response. The live `BigAssetSession` provider currently
dispatches spans in scheduler/admission order. A separate
`createPageRangeReader()` helper source-sorts and restores caller order, but it
is not wired into the provider. CEF therefore merges only spans already adjacent
in admission order. Caddy supports multipart ranges through its static file
server; CEF and the development server can also
stream that compatibility format through `afterglow-assets::MultipartSource`.
A fixed 256-slot client queue, 4 MiB complete-response cap,
two-response/8 MiB in-flight cap, and non-overlapping range validation make
overflow deterministic. It avoids routing
tiny page ranges through the page-side AssetLoader wasm executor. A fixed
12-entry waiting ring dispatches across two to four independent WASM texture
workers (four on machines exposing eight or more logical CPUs). Each worker
remains one-in-flight/SPSC-safe and owns its response before reuse. This is the
public-web backend; `afterglow-shell` composes generated native clients and real
OS workers instead. Physical slots are acquired only
after page bytes are ready, so a slow range read or transcode cannot evict useful
resident data while it is pending. Upload currently copies every completed page
into the full CPU atlas shadow before `GPUQueue.writeTexture`, even after the
native GPU atlas is attached. Together with generated RPC encoding and two
owned-output slices, this is bounded but remains an intentionally documented
copy/memory optimization target.

Runtime work is bounded to 16 pending pages and 2 MiB of expected output. The
pending table, 16-entry ready-upload ring, scheduler, resident atlas slots, and
feedback scratch are preallocated; hot identity uses numeric `textureId`/packed
page keys. Pinned startup requests that exceed the admission cap enter the same
fixed scheduler at highest priority and are retried until resident; they are
exempt from feedback-staleness cancellation. Path Maps remain only for load/
unload and game-facing lookup. The
scheduler capacity equals the physical atlas capacity. Scheduling checks a
0.25 ms budget in small batches; atlas/page-table commits are limited to **two**
pages and a **0.20 ms** budget per `poll()` by default. The central
`VirtualTextureTuning` observes presentation intervals only while residency
backlog exists. Active streaming samples accumulate across short empty gaps. In
a 15-sample window, two intervals above 1.25× the target reduce the active page
cap (then its time budget at the one-page floor). One stable window with real
backlog probes one step toward configured device caps
(four pages / 0.35 ms by default), allowing bootstrap streaming to calibrate
short gameplay bursts. A clean probe becomes the new known-safe setting; a bad
promoted cap immediately rolls back to the independently validated two-page /
0.20 ms baseline and waits sixty quarter-second windows before probing again. Completed pages stay in the fixed ready
ring for a later rAF rather than turning a full-residency replacement into a
presentation burst. Rejected admissions, stale cancellations, priority
preemptions, resident hits/misses/evictions, queue bytes, range-read latency,
bulk queued/in-flight bytes, urgent/quality batch counts, bulk rejects/cancels,
transcode worker/queue/runtime telemetry, upload CPU time, and budget exhaustion
are exposed by `getStats()`. `getStats()` updates and returns one
stable preallocated object and is safe for per-frame telemetry; the allocating
`getDebugSnapshot()` is intended only for explicit diagnostics.

When a `BigAssetSession` receives `runtime.telemetry`, armed unified captures
also correlate each page through feedback detection, scheduler wait, page load,
bulk timer wait, bulk source dispatch, transcode queue, transcode execution,
ready-upload work, atlas write, and page-table publication. Publication records
the numeric frame whose later render pass can first sample the committed entry.
Web Fetch and native arena-backed range operations feed the same descriptors; native worker-internal `pread` and
arena lease subspans remain a separate adapter gate.

The pre-removal baseline profile (`docs/benchmarks/dungeon-vt-unified-
telemetry-rtx3090-2026-07-25.md`) found 149.0 ms mean / 231.4 ms p99 complete
page latency on a cold-cache RTX 3090 traversal. The dominant components were
the configured bulk batching window (56.9 ms mean) and transcode queue (81.8 ms
mean), versus 3.0 ms bulk I/O, 11.4 ms transcode execution, and 0.028 ms upload/
page-table publication. The run loaded 973 pages with no failures and emitted
24,850 records without drops or unmatched spans.

The accepted no-cache RTX 3090 implementation profile is
`docs/benchmarks/dungeon-vt-no-cache-rtx3090-2026-07-25.md`. Hostile teleport
improved admitted page latency to 42.02 ms mean / 58.40 ms p99, bulk wait to
9.11/16.45 ms, and transcode queueing to 18.89/34.55 ms. Its 582 frames had
6.955 ms p99, 13.895 ms maximum, no slower-than-60 Hz interval, no failures,
and a 26,861-record trace with zero drops/unmatched spans. Source bytes fell
13%, while bulk request count rose from 53 to 156; that 2.94× count is the sole
unaccepted RTX plan gate. A measured 24 ms deadline reduced requests only to
124 while violating latency/frame targets. A deterministic replay of the
committed trace reproduced all 156 requests: source sorting reduced modeled
adjacent source runs by 30.9% but did not change request count, and mip-deficit/
channel-affinity sensitivity also remained at 156. The approved 16 ms policy
therefore remains selected pending an explicit request-count-policy decision;
meeting 2× requires a different buffering, prefetch, or cooked-superpage policy.

## Asset containers

The compact `VirtualTextureDirectory` was introduced in container v5 and is
unchanged in current writer version 6 (v6 adds resident-texture format metadata).
The bundled Dungeon remains a readable v5 file. Every VT mip is one contiguous
row-major block with one
absolute block offset, page-grid dimensions, and a vector of encoded page
sizes. The optional tail stores one offset and size. It does **not** serialize a
full `ChunkInfo` or repeated mip/x/y/encoding metadata per page.

`parseBigHeader()` admits this directory once. `readBigHeader()` checks magic,
the supported outer version, and the configured header-byte cap, but does not
yet fully validate decoded directory invariants, duplicate asset names, exact
decoder consumption, safe cumulative offsets, or every payload span against
the container's identity size. Malformed-container bootstrap hardening remains
open. `createFetchRangeLoader(baseUrl?)` returns the browser/CEF serving-layer
`load`/`size`/`identity`/`read`/`readBulk`
implementation. `read` requires one exact 206 response; `readBulk` emits up to
256 explicit spans and validates each returned multipart `Content-Range` in
request order. `identity` exposes source size/ETag/Last-Modified to generic
serving-layer consumers.
`createPageDataProvider(loader, header, textureWorkers, format, config,
telemetry?)` accepts a fixed worker list plus explicit transcode queue and
urgent/quality deadline policy, expands each compact size vector into fixed
`Float64Array` offsets and `Uint32Array` sizes, and exposes a stable
`getStats()` view for read/transcode stages. There is no persistent derived-page
cache: every nonresident page follows source read then Basis transcode. Page
lookup is direct `y * pagesX + x` indexing. The production nine-channel dungeon header fell from 764,192 bytes in
v4 to 123,768 bytes in v5, safely below the 1 MiB RPC output limit. v4 is
rejected; bundled assets were rebuilt rather than retaining compatibility.

`afterglow-pipeline height-r16 <height.png> <output.r16>` losslessly decodes a
16-bit displacement source into the engine's versioned, single-channel
little-endian normalized `u16` runtime payload. This intentionally bypasses
browser image decoding and is separate from virtual color/PBR processing.

`afterglow-pipeline process` decodes PNG/JPEG sources and embedded GLB images,
accepts arbitrary positive width and height (including rectangular, non-power-of-two, and
sub-page dimensions), generates filtered mips, and writes independently
seekable partial-edge 136x136 bordered pages plus a packed mip tail. The
offline-only `afterglow-basis-encoder` crate encodes every slot independently as
UASTC Basis. `stream_virtual_texture()` walks one mip at a time and retains at
most 64 bordered pages. The writer moves payloads without cloning, compresses
each chunk once into a temporary disk spool, encodes each bounded batch in
parallel, and assembles stream order with a fixed 64 KiB copy buffer instead of
retaining the complete raw page set or encoded container payload in RAM.
Runtime `afterglow-texture` transcodes Basis pages to BC7, ASTC, or
RGBA. Raw RGBA pages remain explicitly tagged and bypass the transcoder. A
self-contained GLB is also packed as a seekable raw model asset. Embedded images
use deterministic `<model>#image-N` VT names; external image URIs are rejected.

## Demonstrations

The dungeon's scanned 8K PBR sources are cooked into a generic `.big` container,
then loaded through range reads and transcoded from
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
3,600 slots, approximately 254 MiB RGBA8). Neither demo has a private residency
or page-table implementation; no demo uses persistent derived-page storage.

`vt-demo` (canonical `EngineRuntime` consumer) uses
the procedural store factory, generic `VirtualMaterialBinding`, renderer host,
and feedback coordinator. It displays one procedural 262,144×262,144 terrain
texture (256 GiB logical RGBA), with WASD pan, overview, one-texel zoom, and
deterministic programmatic control. Three independent real-GPU launches pass
all raw-feedback, compressed/uncompressed upload, and residency trajectories.

`rigged-vt-demo` (canonical `EngineRuntime` consumer) uses `BigAssetSession`, which loads two image-free GLBs from the same `.big`
container as their extracted virtual material channels; the cook reduced the
current container from roughly 633 MiB to 463,702,085 bytes. Stable parser
indices drive `VirtualGltfBinding`, and one coordinator publishes atomic
multi-pass feedback. **1** selects the
first animated rig; **2** selects Spooky Iluha's downloaded “Sci-Fi Character -
Dragon Warrior (Futuristic)” with 18 skinned meshes, an Idle clip, and 45 virtual
material images from 512² through 4096². The offline cook embeds its external
`.gltf`/`.bin`/texture package, moves texture metadata into the Afterglow
extension, and strips imported image bytes before packing. The session-owned
`AssetStore` parses both and sends grouped triangle indices through runtime
meshopt. Visible and feedback materials render the same objects, so requests
follow current deformation. Both rigs cast animated/self shadows from a bounded
2048² directional PCF shadow map onto the receiving floor; feedback renders
explicitly disable redundant shadow passes. **W/S** zoom and **A/D** orbit with
damped inertia; **B** toggles the active skeleton. The GPU regression validates
both models, grounding, inertia, required page residency, zero errors, and zero
post-seal pipelines. Model 1 is KallMor's “Decraniated (Low Poly
Retro Pixel),” CC BY 4.0; model 2 is CC BY-NC 4.0.

`dungeon` (canonical `EngineRuntime` consumer) uses
`RendererHost`, `BigAssetSession`, `VirtualTextureFeedbackCoordinator`, bounded
input/diagnostics, and engine-owned POM materials. It is a first-person corridor dungeon using
three downloaded 8K PBR sets across twelve wall instances. Two sets are
8192×8192 and one is natively rectangular at 8192×4096. Albedo, OpenGL normal,
roughness, and AO pages share one albedo feedback stream while retaining
independent residency, mip fallback, priority, and eviction in the engine atlas. Official resident 1K, 16-bit displacement
maps from the same materials are expanded losslessly into filterable,
single-channel WebGPU `r32float` and drive the 8–32-layer POM + 8-step light self-shadow tier; all
displaced PBR channels use `VT_SAMPLE_FROM_LEVEL_WGSL` for
coarser-page/tail fallback. Dungeon submits VT feedback every 8 frames; the
measured 4-frame cadence caused additional missed-vsync events.
Interactive controls are WASD, Shift sprint, raw pointer-lock mouse look, POM
(**P**), reset, and three test poses. `window.__afterglowDungeon` exposes
`setProgrammatic`, `setPomEnabled`, `pomStatus`, `setPose`, `getPose`, `move`,
`look`, `step`, `waitForIdle`, allocation-free `telemetry`, `errorCount`,
`snapshot`, and `runScenario`.

> **Launcher status:** the `afterglow-cef` example launchers and their GPU
> soak scripts have been removed with `afterglow-cef`. The demo pages themselves
> (`vt-demo.html`, `rigged-vt-demo.html`, `dungeon.html`) still exist under
> `crates/afterglow-web/www`; they await rehoming as shell-launched pages — see
> `docs/implementation/shell-promotion-plan.md`. The commands below are retained
> as historical evidence of the validated configurations.

```sh
# Native host launch awaits the shell promotion (afterglow-cef launchers removed).
# Historical launcher shape:
#   nix-shell shell.nix --run "cargo build --example vt-demo -p afterglow-cef"
#   nix-shell shell.nix --run "cargo build --example rigged-vt-demo -p afterglow-cef"
#   DISPLAY=:0 ./target/debug/examples/rigged-vt-demo --ozone-platform=x11
# Soak scripts removed: test-rigged-vt-gpu.sh, run-dungeon.sh, test-dungeon-gpu.sh,
# soak-dungeon.sh. Re-establish under the shell host as a promotion work item.
```
# Deterministic occupancy states (run cold → half → full → churn in one process):
./scripts/baseline-vt-atlas.sh half vt-atlas-half.log
```

## GPU timing diagnostics

`VirtualTextureFeedbackCoordinator.resolveGpuTimings(out)` is a diagnostic slow
path. `RendererHost` owns Three's external-loop frame boundary: before visible
rendering it resets `renderer.info` and assigns the engine `frameId`, so the
visible scene, output conversion, and any later VT feedback passes share one
logical timestamp frame.

`VirtualTextureGpuTimings` contains:

- `gpuTimingValid` and `resolvedFrameId` (`false` / `-1` when unavailable);
- `gpuSceneMs`: internal HDR scene work excluding output and VT feedback;
- `gpuOutputMs`: fullscreen tone-mapping/color-space conversion;
- `gpuFeedbackMs`: sum of every VT feedback target in that frame;
- `gpuTotalMs`: sum of all Three render contexts in that frame.

Resolution filters every context by `resolvedFrameId`, clears Three's retained
diagnostic keys after readback, and deterministically returns invalid/zero values
if timestamp support or the r185 private adapter shape is unavailable. There is
no legacy `gpuMainMs`; it accidentally named the output context and was deleted.

## Atlas-state baseline

The 2026-07-16 real-GPU baseline filled all 3,600 physical slots and then
performed 1,014 cumulative evictions under the former grouped-residency policy. Cold, half, and full states
had 6.955 ms maximum rAF intervals at 144 Hz. Churn averaged 6.970 ms with a
20.850 ms maximum and one interval above 17 ms; load failures, queue overflows,
long tasks, and GPU errors remained zero. WebGPU timestamp queries measured the
latest full-state main/feedback contexts at 0.149/0.018 ms. A later constrained-
atlas AMD RGP 2.7 comparison identified a 4.56-million-pixel material draw as
the dominant traced event. POM raised FS register use from 40 to 56 VGPR and
reduced theoretical occupancy from 12/16 to 9/16 without spills. The trace's
4.824/5.749 ms durations are not production timing because SQTT was active and
fine-page residency had not settled. The subsequent timestamp audit invalidated
the historical 10.63 ms result: Three's frame ID stayed at zero under the engine
rAF loop, and `gpuMainMs` named the output transform rather than the HDR scene.
With corrected frame identity, scene-plus-output means were
4.19/4.28/5.84 ms non-POM and 6.56/5.49/8.29 ms POM across the three canonical
poses; corner POM p99 was 10.49 ms. The permanent split-field gate then resolved
80/80 monotonic frames with exact scope sums: POM/feedback-on forward means were
5.211 ms scene, 1.083 ms output, 0.006 ms feedback, and 6.301 ms total. See
`docs/benchmarks/vt-atlas-baseline-2026-07-16.md` and
`docs/research/amd-rgp-radv-capture-methodology.md`.

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
