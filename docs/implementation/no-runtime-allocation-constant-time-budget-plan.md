# Allocation-disciplined, constant-time, frame-budgeted engine plan

**Status:** implemented and release-gated; all sequence items complete (2026-07-16)  
**Date:** 2026-07-15  
**Motivation:** sustained VT movement exposed progressively increasing cache,
queue, event-loop, and GPU costs. The policy in this document applies to the
whole engine, not only virtual texturing.

Related audit:
[`../audits/virtual-texture-vertical-slice-2026-07-15.md`](../audits/virtual-texture-vertical-slice-2026-07-15.md).

---

## 1. Non-negotiable goals

1. **Engine-authored gameplay-frame code performs no general-purpose dynamic
   allocation after warm-up.** Game code may allocate, but engine systems may
   only consume game input through bounded preallocated APIs.
2. **Every intentional engine allocation is classified and bounded by the
   logical `EngineMemory` policy.** Page-owned hot storage uses `EngineMemory`
   arenas/pools or fixed subsystem rings; unavoidable browser/Three/codec
   boundaries use the machine-checked effect manifest plus subsystem byte/count
   admission and telemetry.
3. **Hot operations are O(1) worst-case where practical and O(1) amortized only
   where unavoidable and bounded.** No hot-path linear scans, array shifting,
   full index rebuilds, sorting, or object/string-key construction.
4. **Potentially stalling work is lazy, incremental, cancelable, and governed by
   an explicit time and work budget.** No stage can build an unbounded backlog.
5. **Overflow is visible and deterministic.** A full pool or exhausted budget
   returns a typed status/counter; it never silently grows a collection.
6. **Performance is tested over long cache-fill/churn sessions.** A short rAF
   benchmark is not sufficient evidence.

---

## 2. Feasibility boundary: what “no allocation” means on the web

JavaScript does not expose a custom allocator, and Three.js, promises, Fetch,
WebGPU bindings, and the browser may allocate internally. Game and engine code
share one garbage-collected JS heap. Therefore the enforceable contract is:

> After the engine enters `GameplaySealed`, **engine-authored hot paths create no
> JS objects, arrays, promises, closures, strings, typed arrays, typed-array
> views, maps, sets, or dynamically growing collection entries**. They operate
> on memory and objects allocated during bootstrap/warm-up through
> `EngineMemory`.

This does not claim that V8, Three.js, the WebGPU implementation, or game code
never allocate. The implementation must separately measure:

- engine-authored allocation violations;
- total JS heap growth/GC;
- worker wasm allocations;
- browser/Three.js behavior after warm-up.

For Rust/native/wasm workers the contract is stronger: a tracked global
allocator can prove that selected service loops perform zero allocations after
seal. Each worker has a separate address space/wasm memory, so “single allocator
resource” means one **logical policy and telemetry schema**, with one
`EngineMemoryDomain` instance per execution domain:

```text
page/main EngineMemory resource
texture-worker EngineMemoryDomain
asset-worker EngineMemoryDomain
native worker EngineMemoryDomain
```

Payload ownership still follows the engine's RingBuffer-only communication
rule.

---

## 3. Runtime phases

```ts
export const enum EnginePhase {
  Bootstrap,       // capacities, worlds, workers, arenas, renderer
  Warmup,          // pipelines/materials, first GPU resources, pool filling
  GameplaySealed,  // no engine general allocation on hot paths
  LoadingScreen,   // explicit larger budgets; allocation still tracked
  Shutdown,
}
```

### Bootstrap

General allocation is allowed but accounted. The engine reads capacity
configuration and reserves all pools/arenas/rings. Capacity failures happen
here where possible.

### Warm-up

The engine creates Three.js materials, render pipelines, GPU textures/buffers,
query structures, reusable promises/callback objects where APIs force them, and
all per-frame scratch memory. Warm-up must exercise every known render/material
variant before sealing.

### GameplaySealed

- Hot systems may only use preallocated storage.
- Structural work uses bounded command buffers and budgets.
- Async completion is copied into preallocated slots/rings.
- A capacity miss is reported; no implicit growth is allowed.
- Browser APIs known to allocate are only invoked by explicit budgeted boundary
  systems, never accidentally from arbitrary frame code.

### LoadingScreen

Allows explicitly declared slow work with higher budgets. It is not a loophole:
all allocation is still counted and reason-tagged.

---

## 4. `EngineMemory` resource

### 4.1 Public shape

```ts
interface EngineMemoryConfig {
  maxEntities: number;
  maxStructuralCommands: number;
  maxWorkerCompletions: number;
  maxAssetRequests: number;
  maxAssetBytesInFlight: number;
  maxVtPages: number;
  maxVtRequests: number;
  maxVtReads: number;
  maxVtFetchedPages: number;
  maxVtReadyUploads: number;
  frameScratchBytes: number;
  renderScratchBytes: number;
}

interface EngineMemory {
  readonly phase: EnginePhase;
  readonly frame: LinearArena;
  readonly render: LinearArena;
  readonly structural: StructuralCommandPool;
  readonly workerCompletions: CompletionPool;
  readonly assets: AssetRequestPool;
  readonly vt: VirtualTextureMemory;
  readonly metrics: AllocationMetrics;

  sealGameplay(): void;
  beginFrame(frameId: number): void;
  endFrame(): void;
  requestSlowPath(reason: SlowPathReason): SlowPathPermit | null;
}
```

`EngineMemoryRes` is installed eagerly while creating the world. Resource
factories must not lazily allocate after gameplay begins. `Resource.get()` is
changed to reject missing resource construction in `GameplaySealed`.

### 4.2 Linear arenas

`LinearArena` owns one preallocated `ArrayBuffer` and returns integer offsets,
not new typed-array views. Systems use persistent views created at warm-up or
read/write methods over the shared buffer. `reset()` only rewinds an integer.

```ts
interface LinearArena {
  readonly buffer: ArrayBuffer;
  readonly capacity: number;
  readonly used: number;
  allocateBytes(size: number, alignment: number): number; // offset or INVALID
  reset(): void;
}
```

Frame scratch may be used only for data whose lifetime ends before
`endFrame()`. Persistent data must come from a fixed pool.

### 4.3 Fixed pools

Every persistent record pool uses:

- a fixed typed-array structure-of-arrays;
- a free-index stack;
- generation counters for stale-handle rejection;
- O(1) acquire/release;
- high-water and overflow counters.

No pool method returns a fresh object. Handles are packed integers or caller
provided output records.

### 4.4 Explicit slow-path permits

Some browser APIs force allocation (`fetch`, `Blob`, `createImageBitmap`, shader
compilation). They are invoked only by systems with a `SlowPathPermit`:

```ts
const permit = memory.requestSlowPath(SlowPathReason.AssetFetch);
if (permit === null) return WorkStatus.Deferred;
assetBoundary.startFetch(requestId, permit);
```

Permits enforce count, bytes, and time admission. They do not make allocation
free; they make it intentional, bounded, and observable.

### 4.5 Worker allocators

Rust worker binaries install a tracked global allocator that records:

- allocation/deallocation count;
- allocated/freed bytes;
- current/high-water bytes;
- engine phase and service method;
- violations after seal.

Worker services preallocate codec/transcode scratch and response storage.
`Vec`/`String` growth during a sealed service call is a regression unless the
method carries an explicit allocation permit. Tests wrap selected calls in
`assert_no_alloc`.

Custom collection allocators are not required initially. The first milestone is
preallocation plus a global allocation counter that makes accidental growth
fail tests.

---

## 5. Allocation hygiene and enforcement

### 5.1 Hot-path manifest

Create `engine-hot-paths.json` listing modules/functions that must be allocation
free, beginning with:

- `prepareAfterglowFrame`;
- `RenderAdapter.prepareFrame` and upload flush;
- dirty queue drain/clear;
- transform and hierarchy steady-state sync;
- worker completion drain;
- VT feedback decode/consume;
- VT scheduler tick;
- VT cache touch/admit/evict/commit;
- input update;
- frame-budget scheduler.

Every new engine system must declare one of:

```text
@hot-no-alloc
@budgeted-boundary(reason)
@bootstrap-only
@game-facing (allocation policy belongs to game)
```

### 5.2 Custom ESLint plugin

Add `tools/eslint-plugin-afterglow` with rule
`afterglow/no-hot-path-allocation`. In hot functions it rejects:

- `new` expressions except approved pooled-handle accessors;
- object and array literals;
- closures and function expressions;
- template strings and dynamic string concatenation;
- spread/rest;
- `Promise`, `async`, `await`, `.then`, `.catch`, `.finally`;
- `.map`, `.filter`, `.reduce`, `.flatMap`, `.slice`, `.splice`, `.concat`;
- `Array.from`, `Object.*` materialization;
- `Map`/`Set` insertion after seal;
- typed-array construction and `.subarray()`/`.slice()` where they create views;
- APIs annotated `@allocates`;
- exception construction in normal control flow.

The rule is syntax/call-metadata enforcement, not proof of V8 behavior. Approved
exceptions require a local suppression containing a reason code and issue link;
CI rejects bare disables.

### 5.3 Allocation-effect metadata

Add generated TypeScript declarations for engine functions:

```ts
/** @alloc-effect none */
function markDirty(...): void;

/** @alloc-effect pooled */
function reserveAssetRequest(...): RequestId;

/** @alloc-effect browser @budget AssetFetch */
function startFetch(...): void;
```

A second lint rule rejects calls from `none` functions to stronger effects.
This gives a small allocation-effect call graph without requiring a custom
TypeScript compiler.

### 5.4 Runtime and CI checks

- CDP allocation-sampling profiles for scripted hot-path tests.
- Heap snapshots before/after 10-minute stable and traversal tests.
- `PerformanceObserver` for long tasks and GC where available.
- Runtime pool high-water/overflow counters.
- Rust tracked-allocator tests.
- A sealed-mode test that monkey-patches selected engine boundary constructors
  only as a supplemental signal; it is not treated as complete proof.

CI fails on:

- lint violation;
- worker allocation in `assert_no_alloc` regions;
- pool overflow;
- monotonic engine-owned memory growth after warm-up;
- an unbounded queue high-water mark.

---

## 6. Constant-time data-structure policy

### Required choices

| Need | Required structure | Forbidden hot-path pattern |
|---|---|---|
| Entity lookup | direct typed array by entity ID | object/map search |
| Handle ownership | generational index | object identity/string key |
| Free records | fixed LIFO index stack | `includes`, `indexOf`, scan |
| Dirty entities | fixed queue + queued bit | Set/object allocation |
| Residency lookup | pre-sized hash/direct page table | slot scan |
| VT eviction | fixed clock/second-chance | array LRU splice/rebuild |
| Asset page lookup | calculated directory index | chunk `.find()` |
| Completion queues | fixed SPSC/MPSC ring | promises in frame loop |
| Priority classes | fixed bucket queues | per-frame sort |
| Timers/deadlines | numeric arrays/min wheel if needed | object timers per job |
| Stats | incrementally maintained counters | per-frame `.filter()` |

### Worst-case versus amortized

Hash maps are expected O(1), not worst-case O(1). Prefer direct addressing when
the key domain is bounded (entity IDs, slot IDs, texture IDs, per-texture page
indices). Where hashing is unavoidable:

- allocate once at configured maximum;
- use open addressing in typed arrays;
- cap probes;
- report overflow/probe high-water;
- never resize during gameplay.

Any bounded linear pass must state its maximum work. “At most 32 completion
slots” is acceptable constant bounded work; “scan every resident page” is not.

### Structural operations

Spawn/despawn/reparent are not steady-state free. They enter a fixed structural
command ring. `FrameBudgetScheduler` applies at most the configured command and
time budget. Hierarchy maintenance becomes incremental where possible. A full
O(entity count) hierarchy rebuild is loading-screen/bootstrap work or a
budgeted multi-frame job with stable intermediate semantics.

---

## 7. Frame-budget scheduler

### 7.1 Frame structure

```text
beginFrame
  critical input snapshot                 fixed work
  critical worker wake/completion drain   bounded count
  structural command slice                deadline + count
  asset state-machine slice               deadline + count + bytes
  VT feedback/scheduler slice              deadline + count
  VT upload commit slice                   deadline + count + bytes
  transform/render synchronization         dirty-count bound
  GPU command submission                   fixed phase
render
endFrame
```

No stage owns an unbounded loop. Every loop is one of:

- fixed count;
- queue until numeric deadline;
- fixed dirty count capped by pool capacity;
- bootstrap/loading only.

### 7.2 Budget resource

```ts
interface FrameBudgetConfig {
  targetFrameMs: number;
  reserveRenderMs: number;
  structuralUs: number;
  workerDrainUs: number;
  assetUs: number;
  vtScheduleUs: number;
  vtUploadUs: number;
  maxStructuralOps: number;
  maxWorkerCompletions: number;
  maxAssetTransitions: number;
  maxVtAdmissions: number;
  maxVtUploads: number;
  maxVtUploadBytes: number;
}
```

Each stage checks both a work-unit cap and a monotonic deadline. Time checks are
performed every small fixed batch (for example eight operations), not after
every trivial operation.

### 7.3 Budget behavior

- Critical input/render work never spills.
- Optional work is deferred without reallocating or losing queue position.
- Unused optional budget may not cause an unbounded burst next frame.
- A small capped credit can improve throughput but must preserve worst-case
  frame cost.
- Loading-screen budgets are separate and explicit.
- Worker CPU time is measured separately; page-thread completion processing
  remains budgeted.

### 7.4 Lazy work

Lazy means “only after demand and only enough to meet the next visible need,”
not “start unlimited asynchronous work and await it later.”

Examples:

- VT directory loaded when first material references it.
- Page read admitted only if still in the persistent requested set.
- Fine page admitted only after required coarse safety residency.
- Material/GPU pipeline warmed when a loading manifest predicts use, not on the
  first visible frame.
- Debug snapshots are generated on request at a low frequency, not every HUD
  frame.

---

## 8. Engine subsystem migration

## 8.1 Core resources and frame orchestration

1. Install `EngineMemoryRes` and `FrameBudgetRes` eagerly.
2. Make `Resource.get()` allocation-free after seal; missing resources become a
   deterministic error.
3. Replace object-based frame inputs with preallocated structs/typed arrays.
4. Remove stale optional VT prediction/frame-time API from `frame.ts`.
5. Add phase assertions to every system entry point.
6. Make diagnostics consume counters without creating snapshots every frame.

## 8.2 ECS, dirty state, and hierarchy

The existing typed dirty queue is a good model. Extend the approach:

- all capacities configured at bootstrap;
- no bitECS query materialization in steady-state paths;
- persistent query membership/index arrays;
- incremental hierarchy ordering for bounded structural changes;
- fixed structural and pose rings;
- no dynamic Three.js proxy creation during gameplay without a slow-path permit;
- prewarmed object pools for common render proxies.

Game code may request spawn. The engine either consumes a preallocated command
slot or returns `CapacityExceeded`; it does not grow.

## 8.3 AssetStore

Current `processPendingLoads()` attaches new `.then()` handlers to every pending
promise on every poll, allocating callbacks and potentially duplicating
continuations. Replace promises in the frame loop with numeric request states
and completion rings.

Target states:

```text
Free -> Queued -> Reading -> ParsingWorker -> ReadyToPublish -> Published
                                     \-> Failed
```

- Handles are generational integers backed by fixed arrays.
- Asset cache uses fixed-capacity IDs, not runtime string-map growth in hot code.
- Path interning occurs during manifest/bootstrap; gameplay uses `AssetId`.
- Model parsing and image bitmap creation are budgeted boundary/loading work.
- No full model parse begins during an ordinary gameplay frame.

## 8.4 RPC and workers

1. Remove per-call auto-poll timers from `AsyncWorker`.
2. One transport pump per client/worker.
3. Use ring completions and numeric task slots, not per-task JS promises in hot
   engine code.
4. Fixed maximum in-flight tasks and bytes.
5. Worker request/response scratch preallocated at bootstrap.
6. Service implementations reserve vectors/scratch before seal and reuse them.
7. Cancellation/stale generation is a first-class frame, not a dropped promise.
8. Payload-free `postMessage` remains wake-only.

User/game-facing convenience APIs may return promises outside hot paths, but
they adapt to the fixed engine task pool rather than owning engine state.

## 8.5 Virtual texturing

Implement the audit remediation as a clean replacement rather than extending the
array LRU.

### Fixed metadata

- Compact `.big` v5 VT directory.
- `AssetId`/`TextureId`, no path strings in hot requests.
- Direct page index from mip/x/y.
- Directory arrays allocated once when the material is admitted, under a
  budgeted allocation permit or loading phase.

### O(1) residency

- direct `PageKey -> SlotId` fixed hash/direct map;
- fixed slot records;
- free slot stack;
- clock/second-chance eviction with bounded probes;
- incremental counters;
- no `find`, `findIndex`, `indexOf`, `includes`, splice, unshift, or map rebuild.

### Bounded pipeline

```text
feedback table (fixed)
  -> priority buckets (fixed)
  -> read slots (fixed count/bytes)
  -> fetched slots (fixed count/bytes)
  -> transcode slots (worker bounded)
  -> ready material groups (fixed)
  -> frame upload budget
```

- Do not reserve an atlas slot until a material page group is ready.
- Drop stale fine requests before reads/transcodes.
- Atomic group commit.
- Pack roughness/AO/metallic masks offline.
- One shared resolved fallback mip per material set.
- Propagated fallback page-table entries or another O(1) shader lookup.
- GPU-compacted feedback or reused allocation-free CPU decode buffers.
- Continuous derivatives before UV wrapping.
- Explicit sRGB/linear channel handling.

## 8.6 Renderer and Three.js boundary

Three.js may allocate internally. Contain it:

- create scenes, materials, geometries, bind groups, and pipelines during
  bootstrap/warm-up;
- preallocate/pool render proxies;
- avoid material mutation that triggers pipeline recompilation;
- use persistent vectors/matrices; never instantiate math objects per frame;
- batch dirty GPU uploads;
- avoid `toJSON`, debug snapshots, or object enumeration in frame HUDs;
- isolate unavoidable Three.js allocation APIs behind budgeted boundary calls;
- use GPU timestamp queries for main/feedback passes where supported.

A “sealed renderer” diagnostic records JS heap and pipeline creation events
after warm-up. Any new pipeline during normal gameplay is reported.

---

## 9. API policy for game code

The requirement permits game-code allocation. The engine must not accidentally
adopt arbitrary game objects into hot ownership.

- Game code submits commands by value into fixed engine buffers.
- Engine APIs return numeric handles and status codes in hot paths.
- Convenience object/promise APIs are explicitly game-facing wrappers and not
  used internally.
- Variable-length game data must be copied into a reserved engine blob slot or
  rejected/deferred.
- Game callbacks execute outside engine no-allocation assertions, but their
  duration and resulting GC still appear in whole-frame telemetry.

Example:

```ts
const result = engine.spawnCommands.tryPush(
  archetypeId, positionX, positionY, positionZ,
);
if (result !== WorkStatus.Accepted) {
  // Game chooses whether to retry, degrade, or drop.
}
```

No engine array grows because game code spawned more objects than configured.

---

## 10. Implementation sequence

### Phase 0 — freeze and measure

- [x] Replace the original correctness prototype with the audited bounded VT runtime.
- [x] Add configurable stable/traverse/thrash soak scenarios with raw CDP trace capture.
- [x] Add fixed current/cumulative/maximum CPU timing arrays for frame stages.
- [x] Add unified queue-depth, heap/long-task, frame, and CPU-stage trace capture.
- [x] Add WebGPU timestamp-query capture for feedback, main, and aggregate render passes.
- [x] Record baseline at cold, half-full, full, and churned atlas states.
- [x] Require stage/counter attribution for further performance tuning.

### Phase 1 — memory contract and lint foundation

- [x] Add `EnginePhase`, `EngineMemoryConfig`, `EngineMemoryRes` foundation.
- [x] Add fixed arenas/pools/free stacks and allocation telemetry foundation.
- [x] Rewind `EngineMemory.frame` at the start of `prepareAfterglowFrame()`.
- [x] Add `ResourceManifest` eager initialization and completeness validation before seal.
- [x] Add marked hot regions and allocation lint (expand coverage during migration).
- [x] Migrate authored browser/runtime/worker/demo JavaScript to TypeScript and add generated-artifact drift checks.
- [x] Add a machine-checked allocation-effect manifest for migrated hot regions and boundaries.
- [x] Classify every authored engine module and migrated hot entry point by allocation effect.
- [x] Add native Rust `TrackingAllocator` and initial ring `assert_no_alloc` regression.
- [x] CI: generated-artifact check, allocation lint, fixed-storage tests, and Rust `assert_no_alloc` tests.

### Phase 2 — frame scheduler

- [x] Add `FrameBudgetRes`, frame-rate-scaled cumulative deadlines, and typed deferral.
- [x] Bound frame-owned structural, dirty-root, hierarchy, unique-proxy, worker, asset, and VT loops.
- [x] Add a fixed AssetStore completion ring with bounded per-poll publication.
- [x] Add a fixed structural-command ring with typed overflow and bounded drain.
- [x] Bound wasm worker completion queues to 256 reserved entries and JS drains to 32 per poll.
- [x] Remove AssetStore promise attachment/scanning from frame orchestration.
- [x] Add typed frame-stage deferred statuses and fixed telemetry counters.
- [x] Add typed overflow/deferred statuses to engine-owned queues; Promise wrappers reject at game-facing boundaries.

### Phase 3 — constant-time core

- [x] Migrate AssetStore to fixed numeric IDs/state tables with bootstrap path interning.
- [x] Consolidate async worker polling to one completion pump.
- [x] Remove repeated promise handler attachment in `processPendingLoads()`.
- [x] Migrate `AsyncWorker` pending calls and browser fetch tracking to fixed 256-slot tables.
- [x] Route all generated worker clients through fixed 256-task slots and bounded completion drains.
- [x] Make hot telemetry incremental and return a stable allocation-free stats view.
- [x] Remove steady-state ECS query materialization; remaining hierarchy query is structural-only.
- [x] Incremental double-buffered hierarchy rebuild (512 ops / 0.2 ms per frame).

### Phase 4 — VT replacement

- [x] Build an O(1) numeric page index at container-header load (compact on-disk directory remains Phase 5).
- [x] O(1) residency lookup/touch with fixed clock eviction.
- [x] Preallocated feedback expansion/deduplication/capacity fitting (no per-frame Maps or channel objects).
- [x] Bounded persistent scheduler, stale replacement, and ready-time slot acquisition.
- [x] Explicit 64-page and 8 MiB in-flight caps.
- [x] Cooperative cancellation at read/transcode boundaries and fixed serial transcode ring.
- [x] Treat one 136×136 Basis transcode as bounded atomic work; cancel before dispatch and discard stale post-dispatch output (codec API is non-preemptible).
- [x] Route bounded page reads through AssetLoader workers (native FS / web Worker fetch).
- [x] Pack roughness/AO masks offline and sample one shared mask page.
- [x] Atomic material visibility through shared resolve plus group-wide clock eviction.
- [x] Bounded frame upload transaction (operation and wall-clock caps).
- [x] Reuse double-buffered feedback maps, pooled request records, and mip scratch.
- [x] Keep pooled CPU uniqueness/compaction: measured feedback GPU cost is 0.018 ms and readback decode is fixed/reused; an extra GPU compaction pipeline is not justified.
- [x] Shared four-channel material fallback resolve and exact-level sampling.

### Phase 5 — offline/container

- [x] `.big` v5 compact per-VT mip directories (764,192 B → 123,768 B for the dungeon).
- [x] Move VT page payloads into writer without cloning.
- [x] Precompress once and directly index chunks; remove quadratic finish scans.
- [x] Spool admitted payloads to disk and assemble output with fixed 64 KiB scratch.
- [x] Encode/spool output in bounded 64-page parallel batches.
- [x] Walk one mip at a time and retain at most 64 raw bordered pages.
- [x] Keep up to 16 native asset handles open in a fixed round-robin worker cache.
- [x] Rebuild bundled assets and delete v4 compatibility; no migration shim retained.

### Phase 6 — renderer sealing

- [x] Add explicit scene/camera variant compilation and warm-render passes before seal.
- [x] Prewarm fixed instanced shards and unique-proxy pools; reuse persistent math scratch.
- [x] Detect and count render/compute pipeline creation after renderer seal.
- [x] Batch dirty GPU writes through fixed coalesced slot ranges.
- [x] Define unavoidable browser/Three/codec boundaries and measure heap, queues, long tasks, pipelines, and GPU time.

### Phase 7 — enforcement and rollout

- [x] Enforce module effect classification plus no-allocation lint errors for all sealed hot primitives in CI.
- [x] Require sealed EngineMemory and warmed/sealed RenderAdapter before frame preparation.
- [x] Run corrected 10/30/60-minute real-GPU stable/traverse/teleport soak tests with raw traces.
- [x] Remove array LRU and dynamic engine queues; VT pending/scheduler/cache identity is fixed numeric, with string maps confined to structural APIs.
- [x] Update canonical API docs and mdBook with final capacities, statuses, boundaries, and release evidence.

---

## 11. Test matrix and release gates

### Allocation gates

- Zero engine-authored allocation-lint violations in hot paths.
- Zero Rust allocations in sealed worker service-loop tests.
- No growth of engine pool backing storage after seal.
- Engine-owned JS heap reaches a plateau after warm-up.
- Every unavoidable boundary allocation has reason, bytes/count, and budget.

### Complexity gates

- Cache touch/admit/evict time independent of resident count.
- Page lookup time independent of pages per asset.
- Frame worker-drain cost bounded by configured completion count.
- No per-task timer/poll loop.
- No frame-time sorting or full-cache scan.

### Budget gates

- Every optional stage reports used budget, deferred work, and overrun.
- Queue depth/bytes remain within configured maxima under impossible workloads.
- Stale requests are dropped; backlog age is bounded.
- Upload work respects time, count, and byte limits.
- Loading-screen budget cannot leak into gameplay mode.

### Long-run gates

1. 30-minute close-wall traversal with cache fill and eviction.
2. 30-minute mixed asset spawn/despawn under configured capacity.
3. Repeated teleport/thrash workload.
4. Worker failure/restart and stale completion handling.
5. Full pools and command rings with deterministic degradation.
6. GPU timestamp and presentation proxy metrics remain stable from minute 1 to
   minute 30.
7. No monotonic lag, heap, timer, pending-task, or queue-depth increase.

Target acceptance should be stated per device. For the current 60 Hz gameplay
target, a minimum release gate is p99 within 16.67 ms under the sustained
scenario, zero unbounded queues, and no increasing trend over time. Higher
refresh targets require their own budget configuration; they must not be inferred
from rAF alone.

---

## 12. Completed deletions

The redesign removed these runtime patterns:

- array LRU and rebuilt index maps;
- string page keys in frame code;
- per-RPC `setTimeout(0)` polling;
- frame-loop promises and repeated `.then()` attachment;
- linear `.big` chunk lookup;
- monolithic near-1MiB VT manifest read;
- early atlas reservation/eviction before data readiness;
- unbounded transcode promise chains;
- independently resolved PBR fallback loops;
- full debug snapshot generation every frame;
- v4 VT container compatibility after migration.

---

## 13. Definition of done

This plan is complete. Release evidence demonstrates that:

- gameplay-frame engine code is sealed and lint-clean;
- all engine allocations are startup/pool allocations or explicit budgeted
  boundary events classified in the effect manifest and tracked by fixed
  subsystem telemetry;
- hot lookup/cache/queue operations are constant-time and capacity bounded;
- stall-capable work is lazy, incremental, cancelable, and deadline-limited;
  the non-preemptible 136×136 codec call is bounded atomic work with stale
  completion rejection;
- workers and page thread use one bounded transport pump each;
- VT and general asset queues cannot grow without bound;
- long-running tests show no elapsed-time-dependent degradation;
- docs/API/book state capacities, overflow behavior, budgets, and unavoidable
  allocation boundaries honestly.
