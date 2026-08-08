# Unified paged resources — completion plan

**Status:** approved for implementation; UPR-DEC-001–005 accepted 2026-07-25  
**Date:** 2026-07-25  
**Scope:** close the remaining model/texture gaps after introducing
`FixedResourceRegistry`, `VirtualTextureSystem`, `MemoryVirtualTextureSource`,
and `ModelSystem`.

### Progress (2026-07-25)

Implemented foundations:

- append-only model/mutable/storage telemetry descriptors and metric cells;
- fixed leased mutable page outputs plus allocation-free dirty refresh scratch;
- portable sparse mutable snapshots with atomic unpublished restore;
- generic bounded `PersistentBlobStore`, checksummed OPFS generations, and a
  generated-RPC native `BlobStorageWorker` with 512 KiB chunking;
- fixed Three-compatible `GeometryArena`, `ModelSystem` atomic arena swaps, and
  a native-shell Avocado arena smoke run reaching renderer-ready;
- Dungeon and rigged VT migrated to `VirtualTextureSystem`; native source-key
  routing regression coverage and renderer-ready smoke runs pass with zero page
  failures.

Still open before this plan is complete: move OPFS I/O into its RingBuffer-driven
Worker, finish the cooked rig/morph GLB extension, run rigid/skinned/morph GPU acceptance, and collect
all listed long public-web/native soaks.

Related references:

- [`clean-unified-engine-convergence-plan.md`](clean-unified-engine-convergence-plan.md) — final ownership/deletion/readiness convergence umbrella
- [`no-runtime-allocation-constant-time-budget-plan.md`](no-runtime-allocation-constant-time-budget-plan.md)
- [`../api/virtual-texturing.md`](../api/virtual-texturing.md)
- [`../api/static-lod.md`](../api/static-lod.md)
- [`../api/allocation-boundaries.md`](../api/allocation-boundaries.md)
- [`../api/runtime-capacities.md`](../api/runtime-capacities.md)

## 1. Objective

Finish the seven currently open items without creating texture- or model-specific
infrastructure where a generic bounded primitive is sufficient:

1. bound model GPU storage instead of allowing untracked Three.js buffer growth;
2. persist mutable texture canonical pages on public web and native targets;
3. cook complete skinned/morphed model LODs offline;
4. remove mutable-page output allocation from the sealed refresh path;
5. add stable model/mutable-texture telemetry;
6. migrate Dungeon and rigged VT to `VirtualTextureSystem`;
7. produce current long native and public-web validation evidence.

Completion means both models and textures follow this shape:

```text
fixed generational handle
  -> policy-free source / canonical revision
  -> bounded asynchronous stages
  -> atomic publication
  -> fixed CPU and GPU residency
  -> stable telemetry and deterministic overflow
```

This plan does not add bone LOD, automatic save policy, editor undo/networking,
or a private Three.js renderer fork.

## 2. Recorded user decisions

UPR-DEC-001–005 were accepted as recommended on 2026-07-25.

### UPR-DEC-001 — Model GPU ownership

**Recommended:** implement a logical `GeometryArena` made of fixed, prewarmed,
bucketed `BufferGeometry` slots. Each slot owns persistent typed arrays and the
Three/WebGPU buffers created from them. Publication copies a model revision into
an admitted slot, updates draw ranges/groups, and atomically swaps the handle's
published slot. This bounds GPU bytes/counts while preserving standard Three
PBR, skinning, morphing, shadows, and feedback.

Alternatives:

- true suballocation from one `GPUBuffer`, only if the prototype in UPR-001
  proves that Three r185 public APIs can consume it for all required vertex,
  index, skin, and morph paths;
- a private renderer/backend patch — **not recommended** because it creates a
  version-fragile second rendering path.

### UPR-DEC-002 — Mutable texture persistence policy

**Recommended:** persistence is explicit and game-facing:
`save(handle, key)` / `load(key, descriptor, capacities)`. The engine provides a
portable sparse snapshot and generic bounded blob storage; it does not choose
save cadence, conflict resolution, undo, networking, or cloud synchronization.

Alternatives: automatic background journaling, or snapshot codec only with all
storage supplied by the game. Automatic policy is not recommended for the core.

### UPR-DEC-003 — Cooked rig/morph source format

**Recommended:** add a versioned `EXT_afterglow_mesh_lods` record to the
self-contained runtime GLB. Each primitive references complete per-level index,
attribute, morph-target, group, ratio, and error records; the base glTF retains
one skin, animation graph, material identity, and node hierarchy.

Alternatives: `MSFT_lod` node duplication, or detached model records in BIG.
Both duplicate or disconnect glTF ownership and are not recommended.

### UPR-DEC-004 — Persistence failure policy

**Recommended:** atomic fail-closed load/save. A corrupt, incompatible, partial,
or over-capacity snapshot does not mutate the live texture. Save writes a new
generation and publishes it only after complete validation. The previous valid
generation remains usable.

Alternative: recover valid individual pages from a corrupt snapshot. This adds
partial-state semantics and is not recommended initially.

### UPR-DEC-005 — Required release targets

**Recommended:** require current evidence on public-web Chromium and
`afterglow-shell`; require native storage/model workers on the shell. Keep CEF as
compile/bootstrap smoke coverage while it remains transitional, but do not make
new CEF-only infrastructure.

Alternative: make final CEF composition a blocking release target as well.

## 3. Technical gates (measured, not product decisions)

### UPR-TG-001 — Three-compatible bounded geometry

Prototype one rigid, one skinned, and one morphed mesh using prewarmed bucketed
geometry slots. Prove:

- no new `GPUBuffer` or render pipeline after seal while revising/selecting LODs;
- index, normal, tangent, UV, color, joints, weights, and every morph target
  remain correct;
- material groups, shadows, VT feedback, skeleton sharing, and animation remain
  correct;
- upload and selection stay within explicit byte/operation budgets;
- old publication remains visible on capacity or upload failure.

Also test a true shared-buffer path using public Three APIs. Select it only if it
passes the same gates without private fields or backend patching; otherwise use
the recommended fixed-slot arena.

### UPR-TG-002 — Cross-target generic blob store

Prototype a policy-free byte store with explicit item/byte/write capacities:

```text
open(namespace)
get(key, caller capacity)
putAtomic(key, byte source)
remove(key)
clear(namespace)
stats(stable output)
```

Public web uses an OPFS worker. Native uses a generated `afterglow-rpc` client
and real OS worker with native files. Page↔worker payloads use only RingBuffer
RPC; wake-up messages remain payload-free. Measure bounded chunk sizes,
throughput, crash behavior, and shutdown on both targets before fixing the
transaction format.

### UPR-TG-003 — GLB extension round trip

Cook and reload fixtures containing multiple primitives/material groups, shared
skins, multiple clips, sparse accessors, normalized integer attributes, and
multiple morph targets. Compare base and every LOD against the runtime-generated
path before deleting runtime simplification for cooked assets.

## 4. Work sequence

## Phase 0 — freeze contracts and baselines

### UPR-000 — Regression inventory

- Add contract tests that enumerate the seven open items and reject false
  completion claims.
- Capture current model GPU buffer/count/byte behavior for rigid, skinned, and
  morphed revisions.
- Capture mutable-page allocations, dirty latency, page counts, and heap floor.
- Record current Dungeon/rigged direct `VirtualTextureStore` dependencies.
- Add versioned result schemas before collecting performance evidence.

**Done when:** every later phase has a failing regression and a numeric baseline.

## Phase 1 — telemetry before optimization

### UPR-010 — Append-only telemetry ABI

Append, without renumbering existing descriptors:

- model optimize queued/completed/failed/stale;
- model CPU/GPU bytes, arena slots, upload bytes/time, publication revision;
- mutable writes/bytes, dirty pages generated/published/deferred;
- mutable page-pool/output-pool high-water and overflow;
- persistence read/write bytes/time/result/generation;
- LOD selection changes and rejected geometry admissions.

Add a `Model` telemetry category only at the end of the enum. Expose stable,
preallocated stats views from `ModelSystem`, `GeometryArena`,
`MemoryVirtualTextureSource`, and the blob store. No dynamic labels or per-frame
snapshots.

**Tests:** descriptor ABI stability, exact counters, histogram cell admission,
stable object identity, overflow, disabled-recorder behavior, and AGTB decode.

## Phase 2 — allocation-free mutable output

### UPR-020 — Caller-owned page generation

Add `MemoryVirtualTextureSource.readPageInto(request, target)` and make
`drainDirty` use persistent output scratch instead of `new Uint8Array`.
Generation must validate target size before touching dirty state.

For immediate resident replacement, reuse one source-owned slot because
`replaceResidentPage` synchronously copies/submits the bytes. For asynchronous
source reads, introduce a generic fixed `PagePayloadPool` with leases retained
until commit, cancellation, stale discard, or failure. Pool capacity and bytes
are explicit and cannot exceed the store's pending/ready capacities.

Do not add a texture-specific allocator. The payload lease is reusable by disk,
procedural, mutable, and future generated page sources.

**Failure:** output-pool exhaustion defers the dirty/requested page and keeps the
old resident page visible.

**Tests:** stable backing-buffer identities over 100,000 refreshes, cancel/stale
lease release, no double release, full-pool deferral, R8/R16F/RGBA output,
border/address modes, and allocation-effect coverage.

**Done when:** sealed mutable refresh has no engine-authored output allocation;
remaining Promise/WebGPU boundary allocation is classified and measured.

## Phase 3 — generic persistence plus mutable snapshots

### UPR-030 — Generic bounded blob storage

Implement UPR-TG-002 as a reusable subsystem, not a VT cache. Required
properties:

- namespace/key policy supplied by the consumer;
- fixed in-flight operations, bytes, chunk size, and completion drain;
- atomic replace and previous-generation retention;
- cancellation and stale generation rejection;
- stable telemetry;
- explicit unsupported/quota/corrupt/I/O/capacity statuses;
- idempotent close and bootstrap rollback.

Add `docs/api/persistent-blob-store.md` and the matching book chapter only when
the API exists. Remove the stale `persistent-blob-cache` references that no
longer correspond to repository code.

### UPR-031 — Portable sparse mutable-texture snapshot

Versioned snapshot contents:

- dimensions, format, address mode, mip filter, default texel;
- canonical mip-zero sparse page coordinates and exact bytes;
- content revision and checksum/index metadata;
- no derived mips, borders, atlas slots, page-table state, or GPU format.

Derived pages are regenerated after load. Decode validates all lengths,
dimensions, duplicate coordinates, cumulative offsets, checksums, and caller
page/dirty capacities before committing anything.

Expose explicit save/load operations through `VirtualTextureSystem`; persistence
remains optional and does not change `writeMemoryRegion()` semantics.

**Tests:** round trip for all mutable formats/address modes, empty/sparse/full
snapshots, deterministic bytes, corrupt/truncated/duplicate data, quota and
capacity failure, canceled save/load, crash between generations, previous-value
retention, and web/native backend parity.

## Phase 4 — complete offline model records

### UPR-040 — Cook deformation-aware LODs

Move the existing runtime `simplifyWithAttributes` policy into the offline
pipeline and emit the accepted GLB extension. Preserve:

- all primitive/material groups;
- indices and every base attribute with component/normalization semantics;
- JOINTS/WEIGHTS and incompatible-joint collapse locks;
- every morph attribute and relative/absolute morph mode;
- bounds, ratio, measured error, and deterministic level ordering;
- one unchanged skeleton, node hierarchy, and animation graph.

Use all available logical cores during offline simplification. Bump the
extension version, not BIG, unless the outer container actually changes.

### UPR-041 — Runtime adoption

Teach GLTF parsing to expose typed cooked LOD records and let `ModelSystem`
adopt them without runtime simplification. Delete the rigid one-primitive
`static-lod` format and compatibility code in the same change; rebuild bundled
assets.

Runtime-generated LOD remains available only for genuinely runtime-created or
mutated RAM geometry.

**Tests:** deterministic cook, malformed extension rejection, all supported
attribute component types, multi-primitive/material, skinned/morphed animation
parity, old-format rejection, and cooked/runtime LOD equivalence tolerances.

## Phase 5 — bounded model GPU residency

### UPR-050 — `GeometryArena`

Implement the result selected by UPR-TG-001 as a generic renderer resource.
Bootstrap configuration explicitly lists buckets such as maximum vertices,
indices, morph targets, attributes, slots, and bytes. The arena owns:

- fixed generational geometry slots;
- persistent CPU upload arrays and warmed Three geometry objects;
- exact admitted CPU/GPU byte accounting;
- fixed upload command/completion storage;
- atomic old-slot/new-slot publication;
- deterministic no-bucket, no-slot, byte-budget, and attribute-layout failure;
- high-water/overflow telemetry and idempotent disposal.

`ModelSystem` retains source/LOD policy; `GeometryArena` only stores and
publishes geometry records. `ModelLodBinding` references arena publications and
continues to share skeleton/morph influence/animation state.

Revision flow:

```text
source/cooked revision
  -> CPU LOD completion
  -> preflight every required geometry slot
  -> bounded upload into unpublished slots
  -> warm/validate if bootstrap, otherwise use prewarmed layouts
  -> atomic model revision swap
  -> retire old slots through fixed deferred-release ring
```

No partial LOD set becomes visible. On any rejection, the previous complete
revision remains published.

**Tests:** every geometry layout, all bucket boundaries, exact GPU byte
accounting, rollback at each stage, stale completion, destroy during upload,
slot reuse generations, no post-seal GPU buffer/pipeline creation, and
allocation-free LOD selection.

## Phase 6 — migrate remaining consumers

### UPR-060 — Engine binding APIs consume handles/views

Change `VirtualGltfBinding`, `VirtualMaterialBinding`, and
`VirtualPomSceneBinding` to consume `VirtualTextureSystem` handles/views rather
than requiring demos to reach a concrete store. Keep `VirtualTextureStore` as an
internal/low-level mechanism for tests and composition.

`EngineAssets` composes one `VirtualTextureSystem` with the selected BIG source
and explicit format pools. It must not expose a second competing VT ownership
path.

### UPR-061 — Dungeon

Register all cooked channels and resident/mutable layers through the system,
pass the system to the feedback coordinator, attach all pools once, and delete
direct store construction/lookup from the demo. Keep scene, material choice,
movement, POM policy, and benchmark scenarios in game code.

### UPR-062 — Rigged VT

Adopt cooked rig/morph LODs through `ModelSystem`, register every virtual image
through `VirtualTextureSystem`, and use handle-based glTF bindings. Delete the
runtime optimization path for these cooked fixtures and all direct store access
from the demo.

**Gates:** architecture lint forbids direct `VirtualTextureStore` construction
or generated worker/client assembly in visual demos. Both demos report only
public handles, stable telemetry, and typed capacity failures.

## Phase 7 — validation and release evidence

### UPR-070 — Automated correctness/GPU suites

Required on public web and native shell:

- mutable paint visible after bounded refresh for RGBA8, R8, and R16F;
- save, restart, load, and exact canonical-content recovery;
- corrupt latest generation retains previous valid data;
- cooked rigid/skinned/morphed LOD transitions and animation parity;
- repeated model revision fills/retirements without new buffers or pipelines;
- mixed disk/RAM shader layers produce matching visible and feedback sampling;
- capacity exhaustion exercises typed degradation without stale publication;
- worker cancellation/restart leaves no leased payloads or pending jobs.

### UPR-071 — Soaks

Run artifact-hash-matched scenarios:

1. 30-minute mutable painting plus continuous camera traversal;
2. 30-minute save/load generation churn in a non-render-blocking worker;
3. 30-minute rigged animation/model switching/LOD traversal;
4. 30-minute combined Dungeon streaming plus mutable overlay;
5. hostile model-arena and texture-page capacity thrash;
6. forced persistence, meshopt, and texture-worker failure/restart.

For the current 60 Hz gate require:

- p99 frame interval at or below 16.67 ms for admitted normal scenarios;
- zero unexplained GPU errors/device loss/post-seal pipelines;
- no monotonic heap, GPU-byte, lease, timer, pending-task, or queue growth;
- all queue depths return to zero after idle;
- fixed CPU/GPU byte totals agree with resident records;
- no stale model/texture revision publication;
- expected overflow only in explicit hostile-capacity scenarios.

Record hardware, driver, browser/shell, Three version, git commit, artifact
hashes, capacities, raw telemetry, and result JSON under `docs/benchmarks/`.
A smoke run is not soak evidence.

### UPR-072 — Final docs and deletion pass

Update in the same final changes:

- `docs/api/static-lod.md` (rename to a model-system API page if appropriate);
- `docs/api/virtual-texturing.md`;
- `docs/api/allocation-boundaries.md`;
- `docs/api/runtime-capacities.md`;
- generic blob-store/model-renderer API pages;
- matching mdBook chapters and `AGENTS.md` inventory.

Delete the old rigid cook, direct demo VT ownership, temporary prototype code,
stale cache documentation, and superseded tests. Rebuild web artifacts and all
bundled assets.

## 5. Required commands per phase

```sh
bun test crates/afterglow-web/web/src scripts/contracts.test.ts
bun scripts/lint-hot-allocations.ts
bun scripts/lint-import-boundaries.ts
bun scripts/lint-demo-architecture.ts
bun scripts/build-web.ts
bun scripts/build-web.ts --check
nix-shell shell.nix --run "CARGO_BUILD_JOBS=32 cargo run -p xtask -- test"
cd book && nix-shell -p mdbook mdbook-mermaid --run "mdbook build"
```

Rendering/storage phases additionally require current browser integration,
native-shell GPU tests, and the listed soak result JSON. New Rust workers require
tracked-allocator tests and native-target boundary tests proving that the shell
uses generated native clients and OS threads, never WASM workers.

## 6. Definition of done

All items must be true:

- [x] User decisions UPR-DEC-001–005 are recorded.
- [x] Mutable sealed refresh allocates no engine-authored page output.
- [ ] Mutable snapshots survive restart on web and native with atomic failure.
- [ ] Cooked skinned/morphed models require no runtime simplification.
- [ ] Model CPU and GPU bytes/counts are fixed, admitted, and telemetered.
- [ ] Model revision publication is complete and atomic across every LOD.
- [x] Dungeon and rigged VT use `VirtualTextureSystem` exclusively.
- [x] New telemetry has stable descriptors and no hot snapshots.
- [ ] Unit, contract, allocation, Rust, browser, GPU, and docs gates pass.
- [ ] Current 30-minute public-web and native soaks plateau.
- [ ] API docs and mdBook describe capacities, ownership, failure, and targets
      without claiming unimplemented shared-buffer or persistence behavior.
