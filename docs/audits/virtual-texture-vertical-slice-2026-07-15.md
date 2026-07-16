# Virtual-texture vertical-slice audit — 2026-07-15

## Executive summary

The current virtual-texture (VT) vertical slice is **functionally correct enough
to demonstrate** offline tiling, `.big` range reads, Basis transcoding, GPU atlas
uploads, page-table lookup, linked PBR channels, rectangular textures, and GPU
feedback. It is **not production-ready or predictably playable under sustained
movement**.

The reported symptom—performance becoming progressively worse as the player
runs along walls—is consistent with the implementation. It is not explained by
one bad constant. Several costs increase with cache population or outstanding
work:

1. `PageCache.touch()` performs linear slot searches and repeatedly rebuilds the
   complete LRU index. Feedback processing therefore becomes more expensive as
   residency grows.
2. The asset loader runs its wasm executor on the page thread and creates one
   independent `setTimeout(0)` polling loop per RPC. Sustained page traffic can
   create an event/polling storm.
3. Disk reads, transcoding, and completed uploads have no end-to-end bounded
   pipeline. The admission budget is per feedback frame, not a cap on in-flight
   or queued work.
4. Every page lookup scans the asset's `.big` chunk array linearly. An 8K square
   map has 5,461 regular page records; the dungeon has 54,612 regular page
   records across its twelve channels.
5. The feedback pass reads and scans approximately 20,000 RG32Uint pixels per
   completion, creates many temporary objects/strings/maps, expands each albedo
   request into four PBR requests, and submits a second scene pass plus GPU
   readback.
6. During a miss, each of the four PBR channels independently walks multiple
   page-table levels per fragment. Movement through nonresident regions is the
   shader's most expensive state.
7. The cache policy considers only one feedback image, has no persistent
   priority queue or stale-request cancellation, and can churn once the atlas
   fills.

The highest-priority correction is not another mip/upload tuning pass. It is to
replace the residency hot path with O(1) cache operations and an explicitly
bounded, observable pipeline:

```text
feedback candidates
  -> persistent prioritized request queue
  -> bounded range-read slots
  -> bounded transcode slots
  -> bounded ready-upload queue
  -> frame-budgeted atomic material-page commits
```

Until that is done, benchmark numbers from short, cold-cache camera poses should
not be described as evidence of stable VT performance.

The engine-wide remediation requirements and phased implementation are recorded
in [`../implementation/no-runtime-allocation-constant-time-budget-plan.md`](../implementation/no-runtime-allocation-constant-time-budget-plan.md).

### Remediation progress

The first KISS pass has landed:

- VT resident touch/lookup is O(1), and eviction uses a fixed bounded
  second-chance clock instead of array LRU rebuilds (`VT-A01`).
- `.big` VT pages are indexed once into O(1) numeric lookup maps (`VT-A05`).
- `AsyncWorker` has one shared page-thread pump rather than one timer loop per
  call (`VT-A02`).
- VT admissions are capped at 64 pending pages and 8 MiB expected output.
- A fixed persistent scheduler retains deferred feedback, replaces requests
  absent from sixteen newer feedback snapshots, and acquires physical slots only
  after bytes are ready (`VT-A03`, partial).
- Scheduling and upload commits have both operation and wall-clock caps; hot
  telemetry uses a stable allocation-free stats view.
- Feedback expansion/deduplication/capacity fitting now reuses fixed numeric
  scratch records instead of allocating Maps and channel objects per readback.
- CPU page-table residency directly indexes packed `Uint32Array` storage; the
  duplicate string-keyed Map was deleted.
- PBR materials use packed roughness/AO masks, one shared fallback resolve, and
  group-wide logical-page eviction.
- `AsyncWorker` pending calls and browser fetches use fixed 256-slot tables.
- Authored TypeScript imports authored `.ts` modules directly; macro generation
  and build checks reject generated `.js` artifact specifiers.
- `FrameBudgetRes` now supplies frame-rate-scaled cumulative stage deadlines,
  typed deferral, and fixed operation/exhaustion/overrun counters.
- EngineMemory owns a fixed structural-command ring. The engine contract CI
  enforces artifact drift, 21 marked hot regions, fixed browser storage tests,
  and Rust tracked-allocation regressions. A machine-checked effect manifest
  classifies all marked regions and budgeted/bootstrap boundaries.
- `soak-vt-dungeon.sh` captures stable/traversal/thrash frame buckets and VT
  queue/cache counters to raw CDP logs without per-frame debug snapshots.
- Hierarchy updates maintain fixed linked siblings and rebuild into a second
  typed order buffer at 512 operations / 0.2 ms per frame; commit forces child
  matrix/appearance synchronization.
- VT pending/ready/cache records are fixed and page identity is numeric after
  admission. Renderer preparation uses bounded structural/dirty/hierarchy/
  unique slices, prewarmed proxy pools, and post-seal pipeline detection.
- Full-atlas validation reached 3,600 slots and 1,014 evictions. Corrected
  10/30/60-minute soaks covered 863,264 frames with zero failed loads, queue
  overflow, GPU errors, long tasks, pending work, or post-seal pipelines.
- Feedback decoding reuses two maps, resize-time request pools, and fixed mip
  scratch. AssetStore now uses fixed numeric state tables and a preallocated
  completion ring; Promise callbacks enqueue only and publication is capped at
  32 per poll.
- `.big` v5 replaces per-page `ChunkInfo` records with compact contiguous-mip
  directories; the dungeon header dropped from 764,192 B to 123,768 B. Runtime
  expands offsets into typed arrays once. Bundled assets are rebuilt and v4 is
  intentionally rejected. Native range reads reuse a fixed 16-entry cache of
  open `FsSource` handles. Generated web workers reserve 256 completion entries;
  `AsyncWorker` uses 256 task slots and drains at most 32 completions per poll.
- The offline path walks one mip at a time, caps raw/encoded batches at 64
  pages, spools encoded payloads to disk, and assembles stream order through a
  fixed 64 KiB buffer. It no longer retains a complete tiled asset/container.
- The writer moves VT payloads without cloning and replaces repeated quadratic
  chunk scans/recompression with direct compact indexing.
- AssetStore attaches one continuation per pending load instead of one per
  frame.

Cancellation now stops stale jobs at read/transcode stage boundaries and a
fixed serial transcode ring replaces the unbounded page-thread Promise chain.
Cancellation inside an already-running worker RPC, GPU feedback compaction,
atomic material groups, and shader fallback work remain open. The
prototype is therefore still not production-ready.

Post-remediation rAF regressions on fox-laptop (rAF measures page-thread frame
production, not presentation) completed 1,800 close-wall traversal frames and
5,399 all-segment close-wall frames with p99 6.95 ms, zero intervals above
17 ms, zero reported engine/GPU errors, and 2,237 resident slots in the latter
run. The all-segment run did not fill the 3,600-slot atlas, so full-cache churn
remains a required acceptance test.

After the persistent scheduler/ready-time acquisition pass, a further 1,799-
interval close-wall traversal measured 6.955 ms p99, zero intervals above 17 ms,
816 resident slots, no queue overflow, and no engine/GPU errors. End-to-end page
read+transcode latency averaged 303.5 ms and peaked at 441.5 ms during this run;
595 obsolete requests were discarded with the then-four-snapshot visibility
window. The production default is now sixteen snapshots (~444 ms at 36 Hz),
chosen to cover the measured worst-case page latency. A later cancellation/serial-ring run reported zero failed pages, zero
queue overflow, 6.955 ms p99, 307.9 ms average and 431.9 ms maximum read+
transcode latency, and 488 discarded stale requests. Lower worker latency and
in-worker cancellation remain valuable.

The offline pipeline now packs roughness and AO into mask R/G. The rebuilt demo
uses nine instead of twelve VT channels, reducing logical RGBA content from
2.5 GiB to 1.875 GiB and the v4 manifest from roughly 1.02 MiB to 764,192 bytes.
A 899-interval close-wall run measured 6.955 ms p99/max, zero intervals above
17 ms, zero failed loads/queue overflow/GPU errors, and 612 resident pages.

A follow-up preallocated-feedback change briefly exposed an async ownership bug:
`queuePageLoad()` passed a reusable scheduler request record into the page
provider, then recycled that record while the read was pending. Coordinates
could therefore change before chunk lookup/completion, presenting unrelated
pages as apparently rotated or scrambled patches. The queue now copies into its
owned `CachedPage` generation before starting asynchronous work and uses that
owned record through provider lookup and upload. A regression test reuses the
scheduler slot while two reads remain pending and verifies both coordinate sets
stay unchanged. A post-fix screenshot and runtime scenario showed coherent wall
pages, zero failed loads, zero scheduler overflow, and zero GPU errors.

---

## Scope and audited files

This audit follows data from offline source files to final Three.js PBR shading.

### Offline pipeline

- `crates/afterglow-pipeline/src/main.rs`
- `crates/afterglow-pipeline/src/texture.rs`
- `crates/afterglow-pipeline/src/format.rs`
- `crates/afterglow-basis-encoder/`

### Container serving and range I/O

- `crates/afterglow-assets/src/source.rs`
- `crates/afterglow-assets/src/range.rs`
- `crates/afterglow-cef/src/resources.rs`
- `crates/afterglow-assets-worker/src/lib.rs`
- `crates/afterglow-assets-worker/src/fetch.rs`
- `crates/afterglow-web/www/async-worker.js`

### Runtime indexing, RPC, and transcoding

- `crates/afterglow-web/www/engine/big-parser.ts`
- `crates/afterglow-web/www/rpc.js`
- `crates/afterglow-web/www/worker.js`
- `crates/afterglow-texture/src/lib.rs`
- `crates/afterglow-texture/src/safe.rs`

### Residency, feedback, upload, and shaders

- `crates/afterglow-web/www/engine/virtual-texture-layout.ts`
- `crates/afterglow-web/www/engine/virtual-texture-feedback.ts`
- `crates/afterglow-web/www/engine/virtual-texture-feedback-pass.ts`
- `crates/afterglow-web/www/engine/virtual-texture.ts`
- `crates/afterglow-web/www/vt-dungeon.html`

### Concrete audited asset

`/tmp/afterglow-vt-dungeon-materials-v1/vt-dungeon.big`:

| Property | Value |
|---|---:|
| Container size | 1,016,811,712 bytes |
| Header/data offset | 1,023,808 bytes |
| Header payload | 1,023,792 bytes |
| Assets/channels | 12 |
| 8192×8192 channels | 8 |
| 8192×4096 channels | 4 |
| Regular page records | 54,612 |
| Worker scratch/output limit | 1,048,576 bytes |
| Header headroom below worker limit | 24,768 bytes |

The manifest is already within roughly 24 KiB of a hard 1 MiB RPC scratch
limit. Adding another modest VT asset can make the demo fail during header load.

---

## End-to-end data flow

```text
8K PNG channels
  |
  | image decode -> full RGBA8 image
  v
full mip chain in RAM
  |
  | split into 128x128 payload + 4px border (136x136 slots)
  | pack <=64 texel levels into one mip-tail slot
  v
thousands of raw page Vec<u8>
  |
  | Rayon UASTC Basis encoding
  v
BigWriter (all encoded chunks retained in RAM)
  |
  | postcard manifest + globally mip-sorted chunk payloads
  v
.big v4 on disk
  |
  | CEF afterglow:// Range request -> FsSource::read_at/pread
  v
Chromium fetch ArrayBuffer
  |
  | AssetLoader wasm import -> wasm Vec -> postcard response -> JS Uint8Array
  v
main-page JS pageData
  |
  | SAB ring copy
  v
Texture Web Worker / separate wasm memory
  |
  | pure-Rust Basis -> BC7/ASTC/RGBA
  | response copied worker wasm -> SAB ring -> page JS
  v
ready upload Uint8Array
  |
  | CPU atlas shadow copy
  | queue.writeTexture(atlas slot)
  | queue.writeTexture(one r32uint page-table texel)
  v
physical atlas + per-texture packed page tables
  |
  | 4 independent VT sampling functions in MeshStandardNodeMaterial
  v
Three.js PBR output
```

This route is complete, but it has excessive indexing work, copies, queueing,
and synchronization points.

---

## Findings by severity

### P0 — sustained runtime degradation

#### VT-A01: Cache touches are O(slots + resident pages) and worsen as the atlas fills

**Code:** `PageCache.touch`, `PageCache.acquire`, `rebuildLruMap`, `usedSlots`,
`pinnedSlots` in `virtual-texture.ts`.

`touch()` first scans every physical slot with `slots.find(...)`, despite already
having `lruMap`. On a hit it then:

1. splices the LRU array;
2. unshifts the page;
3. clears and rebuilds the complete `lruMap`.

`processFeedback()` calls `touch()` for every expanded material request. A
feedback set of 180 albedo pages becomes up to 720 channel requests. At 3,600
slots this can perform millions of comparisons per feedback completion, followed
by repeated O(resident) LRU rebuilds. This directly matches “the longer it runs,
the stronger the lag.”

Related linear operations:

- eviction uses `slots.findIndex()`;
- commit calls `freeSlots.indexOf()` even after the slot was popped;
- release/remove use `includes()` on the free list;
- statistics repeatedly filter all slots;
- the HUD requests the full debug snapshot every frame.

**Required fix:** use direct key→slot and slot→record maps plus an intrusive LRU,
clock, or segmented-clock policy. Cache hit/touch, commit, release, and eviction
must be O(1). Maintain used/pinned/free counters incrementally.

#### VT-A02: One page read creates one page-thread auto-poll timer

**Code:** `AsyncWorker.call()` in `async-worker.js`.

Every RPC call starts its own recursive `setTimeout(poll, 0)` loop. All loops
call the same wasm executor and drain the same completion queue. The dungeon
also calls `assetLoader.poll()` every frame. With N outstanding range reads,
there can be N redundant timer loops plus the frame poll. Under sustained
streaming this creates event-loop pressure and redundant executor ticks.

The asset “worker” wasm is instantiated on the page thread. Fetch itself is
asynchronous, but wasm request decoding, executor polling, response encoding,
shared-memory copies, and completion draining occur on the page thread.

**Required fix:** exactly one owner must pump an async transport. Prefer a real
AssetLoader Web Worker using the ring protocol. At minimum remove per-call
auto-polling and use one scheduled pump that starts when pending transitions
0→1 and stops at 1→0.

#### VT-A03: No bounded end-to-end streaming pipeline or backpressure

**Code:** `processFeedback`, `queuePageLoad`, `createPageDataProvider`.

`pageBudget = 8` limits admissions per processed feedback map, not total work.
Feedback runs repeatedly while old work remains. There is no explicit bound on:

- outstanding range fetches;
- fetched Basis pages waiting to transcode;
- bytes retained in the promise-based transcode chain;
- completed pages waiting to upload;
- reserved but not committed atlas slots.

`createPageDataProvider()` serializes transcoding by chaining promises, but
range reads happen before entering that chain. If disk/fetch is faster than the
single transcoder, complete Basis `Uint8Array`s accumulate in closures. The
promise chain is FIFO and cannot cancel stale pages.

**Required fix:** one scheduler owns stage capacities and bytes:

```text
max range reads:             8 (example, measured)
max fetched Basis backlog:  32 pages / byte cap
max transcodes:              worker count
max ready uploads:          32 pages / byte cap
max uploads per frame:       GPU-time budget, not only page count
```

Reject or replace stale low-priority requests before reserving atlas slots.
Expose every queue depth and oldest-job age.

#### VT-A04: Residency policy can thrash and has no persistent request state

**Code:** `fitFeedbackToAtlas()` and `processFeedback()`.

Capacity fitting compares only the current expanded feedback set to
`totalSlots - pinnedSlots`. It does not include the current resident working
set, pending reservations, ready uploads, or recently visible pages. The system
has no persistent priority queue, visibility age, grace period, or admission
control. Once full, a new request immediately evicts the current LRU page before
its replacement has even been read or transcoded.

Because feedback is sparse and delayed, a page can be evicted shortly before it
is needed again. Material channels are separate cache records, so churn is
multiplied by four.

**Required fix:** track a bounded persistent working set. Use feedback frequency
(pixel coverage), mip importance, frame age, and material-group identity.
Reserve eviction until data is ready to commit, then atomically swap. Add a
short protected/hot segment or clock reference bit instead of ad-hoc grace
heuristics spread through the engine.

### P1 — major scaling and frame-time risks

#### VT-A05: `.big` page lookup is linear in chunk count

**Code:** `findVTPageChunk()` and `findVTMipTailChunk()` in `big-parser.ts`.

Every requested page performs:

1. linear `header.assets.find()`;
2. linear `asset.chunks.find()` over up to 5,462 records.

This runs on the page thread before every range request. It allocates no index
once and repeats the scan for every channel page.

**Required fix:** build immutable maps once after parsing:

```text
assetName -> asset descriptor
(assetId, mip, x, y, tail) -> { offset, compressedSize, encoding }
```

For still lower overhead, replace object-heavy postcard chunk records with a
compact fixed-stride page directory and calculate/ binary-search entries.

#### VT-A06: The manifest is effectively at the 1 MiB transport ceiling

The current header is 1,023,792 bytes; the wasm scratch is 1,048,576 bytes.
Postcard response framing and any additional records leave little room. The
vertical slice cannot scale to a normal game asset set even though page payloads
are individually seekable.

**Required fix:** keep a small root manifest and per-asset page directories.
Read directories separately/range-wise, or use fixed-size tables that can be
memory-mapped/streamed. No single metadata RPC should approach the ring scratch
limit.

#### VT-A07: Feedback readback is CPU/object-allocation heavy

**Code:** `VirtualTextureFeedbackPass.submit()`.

At 1440×900 and scale 1/8, a feedback result contains about 20,340 pixels or
40,680 u32 words. Every completion scans all pixels and can create:

- decoded objects;
- request objects;
- string keys;
- a `Map`;
- a `Set` of mips;
- a second map during four-channel expansion.

The pass also renders a second scene and performs an asynchronous GPU readback.
Throttling to every four frames reduces frequency but not per-completion cost.

**Required fix:** first remove per-pixel JS allocations (integer packed keys,
reused typed arrays, direct texture-ID arrays). The scalable design performs GPU
compaction/deduplication into a bounded request buffer or hierarchical bitset
and reads back only unique requests plus coverage counts.

#### VT-A08: Four independent shader fallback walks per PBR fragment

**Code:** `VT_SAMPLE_WGSL` plus `sampleEntry()` in `vt-dungeon.html`.

Albedo, normal, roughness, and AO each recompute derivatives and independently
walk from desired mip toward a resident ancestor. In a fully resident view this
is four page-table loads plus four atlas samples. In a streaming/miss-heavy view
it can become dozens of page-table loads per fragment. That makes movement
through missing pages the most expensive GPU state.

**Required fix:** resolve material residency once. Linked channels have aligned
coordinates and should share a resolved mip. Return or cache the resolved page
coordinate/mip, then directly load each channel's page-table entry at that mip.
Longer term, propagate fallback mappings into page-table entries so lookup is
constant-time rather than a shader loop.

#### VT-A09: GPU updates are many tiny queue submissions

Each committed page causes at least:

1. one atlas `queue.writeTexture`;
2. one 1×1 r32uint page-table `queue.writeTexture`.

Evictions add another page-table write. Four linked channels multiply this.
`uploadsPerPoll` controls count but not measured CPU/GPU time, and updates are
not coalesced by destination row/texture.

**Required fix:** batch page-table changes in CPU staging arrays and upload
contiguous dirty rectangles once per table/frame. Batch atlas copies through a
staging buffer and one command encoder where measurements show benefit. Use an
actual microsecond/byte budget and carry unused budget forward conservatively.

#### VT-A10: Material channels are over-stored and over-transcoded

Roughness and AO are grayscale PNGs but each is encoded/transcoded/stored as a
full BC7 RGBA page. This doubles page requests and consumes two full atlas slots
where one packed mask page could hold roughness, AO, metallic, and another mask.

**Required fix:** add generic offline channel packing described by asset
metadata—not a dungeon special case. A practical set is:

- BC7 sRGB albedo;
- BC5/appropriate two-channel normal if supported, otherwise BC7 normal;
- one packed linear mask texture (R=AO, G=roughness, B=metallic, A=free).

This reduces a four-channel material to three physical page streams, or two
where normal/masks can use a suitable packed representation.

#### VT-A11: CEF performs one custom-scheme request/source open per page

Each page calls `fetch` with a Range header. CEF resolves the path, opens an
`FsSource`, parses headers, returns `206`, and services the read through
`read_at`. The actual disk access is bounded and uses pread, which is good, but
per-page browser/custom-scheme/request setup is high compared with a persistent
container reader.

**Required fix:** retain a container handle in a dedicated asset service and
read page ranges through the ring transport. Web deployment still needs HTTP
Range, but should use bounded concurrency and connection reuse. CEF should not
route tens of thousands of tiny local reads through browser fetch if a native
worker transport is available.

#### VT-A12: Runtime copies each page many times

A compressed page travels through CEF buffers, a fetch `ArrayBuffer`, asset wasm
memory, postcard output, JS-owned bytes, SAB request ring, texture worker wasm,
SAB response ring, JS output, CPU atlas shadow, and WebGPU upload staging.

Some copies are inherent because the web worker has separate wasm memory, but
several are architectural choices. The CPU atlas shadow is retained at full
physical-atlas size even after the GPU texture is attached.

**Required fix:** measure bytes copied per resident page. Avoid routing web
fetch bytes through an on-page wasm encode/decode layer. Drop or make optional
the full CPU atlas shadow after initialization for compressed runtime atlases.
Use fixed output buffers/pools to reduce allocation and GC.

### P2 — correctness, maintainability, and offline issues

#### VT-A13: `BigWriter::finish()` has quadratic chunk lookup behavior

For every ordered chunk, each of three offset-solving passes and the final write
uses `self.chunks.iter().find(...)`. With about 54,000 chunks this is
quadratic. `compress_chunk(None)` also clones data during each sizing pass.
`add_virtual_texture_with_tail()` clones every page payload into the writer,
while the original page vector still exists.

This is offline rather than frame-time critical, but it makes large VT builds
needlessly expensive in CPU time and peak RAM.

**Required fix:** store chunks in direct `[asset][chunk]` order or a direct
index, move page `Vec<u8>` rather than clone it, and calculate `Compression::None`
sizes without copying. Stream payloads to temporary files/output rather than
retaining a gigabyte in `BigWriter`.

#### VT-A14: `.big` format metadata is object-heavy and duplicates derivable data

Every page stores enum tags, mip, x, y, encoding, offsets, sizes, compression,
and generic chunk fields. For fixed-size UASTC VT pages much of this is uniform
or derivable. The one-megabyte manifest for only twelve channels is evidence.

**Required fix:** define a VT-specific compact directory in the next container
version: dimensions, page size/border, encoding, mip row offsets, and a packed
array of `(offset,size)` entries in deterministic page order.

#### VT-A15: Feedback uses `fract(uv)` before derivative evaluation

The dungeon passes `uv().fract()` into the feedback function. Derivatives at
repeat boundaries can be discontinuous and produce spuriously coarse mip
requests along seams. The sampling shader correctly computes derivatives from
continuous UV before applying addressing, but the feedback path does not follow
the same rule.

**Required fix:** pass continuous UV and address mode to feedback, compute
continuous derivatives first, then apply repeat/mirror/clamp only for page
coordinates.

#### VT-A16: No trilinear transition and incomplete anisotropic page coverage

Mip selection truncates to an integer page level. There is no blend between
adjacent virtual mips. `textureSampleGrad` protects atlas derivatives, but a
single selected page does not guarantee all pages touched by an anisotropic
footprint are resident. Grazing walls can shimmer or sample fallback/borders.

**Required fix:** define the quality target explicitly. At minimum use stable
mip selection and optional cross-fade. For anisotropy, feedback must request the
footprint's page set or conservatively dilate requests in the major-axis
direction.

#### VT-A17: PBR color-space handling is not explicit

All channel pages share one atlas texture with one Three.js color-space state.
Albedo is normally sRGB; normal, roughness, and AO are linear. The node code
samples all channels from the same atlas without an explicit albedo sRGB→linear
conversion. Visual output can therefore be physically incorrect even when
residency is correct.

**Required fix:** make channel color space metadata explicit and decode albedo
in the node graph, or separate atlas classes where hardware/Three.js applies the
correct format conversion.

#### VT-A18: Material-group residency is not atomic

Feedback expansion requests four channel pages, but each read/transcode/upload
completes independently. A frame can combine a fine albedo page with fallback
normal/roughness/AO pages. The independent shader fallback walks hide missing
channels at significant GPU cost and can create visible material changes.

**Required fix:** schedule and commit a material page group as one logical unit,
or carry a shared group residency table/mip that guarantees every sampled
channel is available.

#### VT-A19: Atlas coordinate packing has an implicit 8-bit slot limit

Physical X/Y use eight bits each in the page-table entry. Current 60×60 atlases
fit, but this constraint is not validated against future GPU limits or alternate
page sizes.

**Required fix:** validate `atlasPagesX/Y <= 255` or revise entry packing.

#### VT-A20: Debug metrics conceal the actual bottlenecks

Current diagnostics expose resident and pending counts, but not:

- feedback pixels and unique requests;
- cache hits/misses and evictions per second;
- request age and stale drops;
- range queue depth/bytes/latency;
- transcode queue depth/latency;
- ready-upload depth and upload time;
- LRU touch CPU time;
- chunk-index lookup time;
- page-table/atlas writes per frame;
- JS heap/GC;
- GPU duration of main and feedback passes.

`requestAnimationFrame` intervals measure page-thread frame production, not GPU
presentation. A 144 FPS rAF result does not disprove GPU saturation or visible
presentation stutter. The project documentation already acknowledges this
limitation; VT validation must not overinterpret rAF.

**Required fix:** add stage counters and histograms to `getDebugSnapshot`, use
Chrome tracing/Performance panel for JS and GC, and use WebGPU timestamp queries
where available for main/feedback GPU passes. Maintain a long-run benchmark,
not only a short stationary test.

---

## What is sound and should be retained

The audit does not recommend discarding the entire implementation. These parts
are good foundations:

- Page identity includes texture path; cross-texture aliasing is avoided.
- `.big` payloads are independently seekable.
- CEF serving performs bounded range reads instead of loading the 1 GB file.
- The corrected `206`/`Content-Range`/CEF skip behavior is necessary and works.
- Basis transcoding now runs in a real Web Worker through SAB rings.
- The texture worker uses optimized `wasm-release` artifacts.
- Rectangular and non-power-of-two page grids are represented correctly.
- Partial edge pages clamp at actual image boundaries.
- Packed mip tails avoid wasting full slots on tiny levels.
- Atlas and page-table writes are incremental rather than complete reuploads.
- Material channels use Three.js `MeshStandardNodeMaterial`; the PBR shader is
  not forked.
- Feedback texture identity is a full u32 and coordinates support large grids.
- Payload `postMessage` is not used for the texture worker.

These are necessary pieces. The missing production layer is efficient,
bounded residency orchestration and scalable lookup.

---

## Recommended replacement architecture

### 1. Compact immutable container index

On startup, load a small root manifest. Load one compact VT directory when a
material becomes relevant. Build or directly use O(1) page records.

```ts
interface VtDirectory {
  width: number;
  height: number;
  firstTailMip: number;
  mipOffsets: Uint32Array;
  offsets: BigUint64Array;
  sizes: Uint32Array;
  encoding: TextureEncoding;
}
```

Page index is calculated from mip row offsets and `(x,y)`, not found by scanning
objects.

### 2. One bounded scheduler

Use explicit page states, but keep them minimal and centralized:

```text
Absent -> Requested -> Reading -> Fetched -> Transcoding -> Ready -> Resident
                         |                       |
                         +---- stale/cancel -----+
```

A state record should contain key, material group, priority, last feedback
frame, queue stage, byte cost, and reserved/committed slot. No stage may grow
without a configured item and byte cap.

Priority inputs:

1. coarser safety pages before finer pages;
2. visible pixel coverage from feedback;
3. requested mip;
4. current-frame visibility;
5. age, only as a tie-breaker;
6. material group completeness.

Do not reserve/evict a physical slot until a replacement is ready to commit.

### 3. O(1) cache

Use:

- `Map<PageKey, SlotIndex>`;
- fixed slot records;
- free-slot stack without scans;
- clock/second-chance or intrusive LRU;
- incremental counters.

A clock policy is simpler than a JS intrusive list and avoids moving hundreds
of entries for every feedback image. Feedback sets a reference bit; eviction
advances a hand until it finds an unpinned, unreferenced slot.

### 4. Workerized asset reads and transcoding

The page thread should submit requests and receive completions through one
transport pump. No per-RPC timer loops. For CEF, a native asset worker should
hold the `.big` file open and `pread` ranges. For web, a real worker can own
bounded HTTP Range fetches. Texture transcoding remains in its dedicated worker.

If website↔native bridging is not yet available, the interim CEF fetch path must
still have one JS scheduler and bounded request count.

### 5. Atomic material-page groups

Pack AO/roughness masks offline and schedule all remaining material channels as
one logical page. The scheduler commits a group only after each channel is
ready. The shader resolves one shared fallback mip and directly samples channel
entries.

### 6. GPU-reduced feedback

Keep reduced-resolution feedback but compact on GPU or at least decode into
reused integer tables. Return unique page keys with coverage counts. Preserve
continuous derivatives across repeat boundaries. Add conservative footprint
coverage for grazing angles.

### 7. Batched updates

At the frame boundary:

- commit a measured number of ready material groups;
- coalesce page-table texels into dirty rows/rectangles;
- upload atlas data through a staging strategy selected by benchmark;
- publish page-table entries only after atlas writes are ordered.

---

## Remediation plan

### Phase 0 — instrumentation before more tuning

1. Add counters/timers for every stage listed in VT-A20.
2. Add current and high-water queue depths.
3. Add cache hit/miss/eviction and stale-request counts.
4. Add a 10-minute scripted wall-run benchmark that fills and churns the cache.
5. Record JS CPU profiles at cold, half-full, full, and churn states.
6. Record GPU main-pass and feedback-pass timestamps when supported.

**Exit criterion:** the progressive lag has a measured CPU/GPU attribution, and
no claim depends only on rAF averages.

### Phase 1 — remove known superlinear page-thread work

1. Replace `PageCache` arrays/scans with O(1) maps + clock eviction.
2. Pre-index `.big` pages.
3. Remove one-auto-poll-loop-per-asset-call.
4. Replace string page keys in hot loops with packed integer/two-word keys.
5. Make debug statistics O(1) and avoid constructing a full texture snapshot
   every HUD frame.

**Exit criterion:** feedback CPU time does not trend upward with resident slot
count.

### Phase 2 — bounded pipeline

1. Introduce the persistent scheduler and stage caps.
2. Stop reserving/evicting slots before data is ready.
3. Add stale cancellation and queue replacement.
4. Enforce byte caps, not only item caps.
5. Commit material groups atomically.

**Exit criterion:** all queue depths remain bounded during impossible workloads;
memory and event-loop task count plateau.

### Phase 3 — reduce rendering and upload cost

1. Resolve one fallback mip per material set.
2. Pack roughness/AO masks offline.
3. Batch page-table updates.
4. Measure and optimize atlas upload strategy.
5. Implement continuous-UV feedback and anisotropic footprint coverage.
6. Add explicit albedo color-space conversion.

**Exit criterion:** missing-page and fully-resident shader costs are both within
the GPU frame budget, with no large movement-only penalty.

### Phase 4 — scalable container and native/web I/O

1. Introduce compact per-VT directories in `.big` v5.
2. Stream writer output without retaining/cloning all pages.
3. Replace quadratic `BigWriter::finish` lookup.
4. Add persistent CEF container reads through the native worker transport.
5. Keep bounded HTTP Range reads for browser deployment.

**Exit criterion:** startup metadata and runtime memory scale to game-sized asset
sets rather than a twelve-channel demo.

---

## Required regression tests and acceptance criteria

### Correctness

- Arbitrary rectangular and non-power-of-two sources.
- Every partial edge-page border and every tail rectangle.
- Repeat/mirror/clamp derivatives at seams.
- Material-group atomic residency.
- Eviction while reads/transcodes are pending.
- Stale completion after unload/cancel.
- Container index lookup for every page.
- Cache full with pinned pages and pending replacements.

### Performance

Run on the documented fox-laptop target and preserve raw results:

1. **Stationary close wall, cold cache:** settle target mip within 1 second.
2. **Continuous close-wall run, 10 minutes:** p99 presentation/frame target
   remains stable from minute 1 through minute 10.
3. **Atlas fill:** frame cost does not rise merely because resident count rises.
4. **Full-cache traversal:** bounded queues and no monotonic JS heap growth.
5. **Teleport/thrash:** no unbounded work; stale requests are dropped.
6. **Feedback:** CPU decode and GPU feedback durations reported separately.
7. **I/O:** range, transcode, and upload latency distributions reported.

Minimum invariants:

- O(1) cache touch/lookup/eviction amortized.
- O(1) page-directory lookup.
- One transport pump per worker/client, not one timer per call.
- Explicit maxima for every queue and byte backlog.
- No whole-container or near-1MiB monolithic manifest RPC.
- No increasing frame cost as a function of elapsed play time alone.
- Zero GPU validation errors and zero worker traps.

---

## Verdict

The vertical slice proves the file format, transport pieces, transcode path,
page-table representation, and PBR node integration can work. It does **not**
yet prove that the residency system is suitable for a game.

The progressive lag is primarily an architecture/data-structure problem:
superlinear cache maintenance, page-thread RPC polling, linear manifest lookup,
and unbounded staged work. Mip bias, feedback frequency, MSAA, and upload count
can change the symptom but cannot make this implementation stable over long
sessions.

Treat the current code as a correctness prototype. Complete Phases 0–2 before
performing further visual-quality tuning or publishing new VT performance
claims.
