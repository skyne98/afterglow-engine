# Clean unified engine convergence plan

**Status:** in progress; CUE-DEC-001–007 accepted as recommended on 2026-07-25  
**Date:** 2026-07-25  
**Scope:** converge the implemented runtime, rendering, asset, virtual-texture,
model, persistence, diagnostics, and native-host mechanisms into one public
ownership model, then delete every parallel path.

## 1. Why this plan exists

The engine has strong bounded primitives, but the repository is not yet a
single clean system. The black Dungeon/rigged-VT incident exposed the central
problem: subsystem unit tests and a `renderer ready` log could pass while the
application lifecycle and visual result were wrong.

This is an **umbrella convergence plan**, not another competing subsystem plan:

- `no-runtime-allocation-constant-time-budget-plan.md` remains canonical for
  sealed runtime, complexity, and frame-budget rules;
- `unified-paged-resources-completion-plan.md` remains the detailed source for
  cooked rig/morph LOD, geometry, persistence, and soak work;
- `demo-engine-migration-execution-plan.md` remains historical detail for the
  first demo migration;
- `shell-promotion-plan.md` remains the native-host target boundary.

This plan owns the final dependency order, public ownership shape, deletion
ledger, visual/readiness contract, and release definition. When it completes,
the overlapping plans must be marked complete or superseded instead of keeping
several live checklists for the same work.

## 2. Non-negotiable outcome

The final engine has:

1. one application lifecycle root;
2. one renderer owner;
3. one resource handle model;
4. one virtual-texture public API;
5. one model publication path;
6. one persistence contract with target-specific implementations behind the
   same RingBuffer service boundary;
7. one readiness definition;
8. one visual acceptance lane;
9. one release command;
10. no compatibility path inside the production engine API.

“Unified” does **not** mean one giant manager. The target is one ownership tree
composed from narrow mechanisms:

```text
EngineRuntime                         one lifecycle/frame/readiness root
├── RendererHost                     one canvas/device/surface/seal owner
├── EngineMemory + FrameBudget       one logical capacity policy
├── EngineAssets                     one confined source/service owner
│   ├── VirtualTextureSystem         one VT namespace and pool owner
│   ├── ModelSystem                  one model source/publication owner
│   │   └── GeometryArena            mandatory bounded GPU geometry
│   └── PersistentBlobStore          one policy-free byte-store contract
├── registered bounded workers       one poll/drain mechanism
├── registered render passes         one ordered render mechanism
└── EngineDiagnostics                one fatal/telemetry surface
```

Game/demo code owns only scene content and policy: models, lights, camera
choreography, controls, presentation choices, save cadence, and scenarios. It
does not assemble transports, physical stores, worker topology, page routing,
renderer attachment, lifecycle rollback, or test globals.

## 3. Recorded consequential decisions

The user accepted CUE-DEC-001–007 as recommended on 2026-07-25. These are now
implementation policy; changing one requires a new explicit decision record.

### CUE-DEC-001 — Breaking deletion policy

**Recommended:** delete superseded public APIs immediately in the migration
change. The workspace is pre-1.0; do not add deprecation shims, aliases, or a
second compatibility package.

This deletes, among other items, `EngineAssets.createVirtualTextureStore()`, the
public `VirtualTextureStore` export, `VirtualTextureRes`, and
`AssetStore.setVirtualTextureStore()`.

**Alternative:** one release of deprecated wrappers. This is not recommended;
it preserves the exact parallel ownership this plan is intended to remove.

### CUE-DEC-002 — Lifecycle root

**Recommended:** extend `EngineRuntime` into the one lifecycle root. Do not add
an `EngineSession`, feature-specific session, plugin framework, or second
application facade. Construction may use a small bootstrap scope internally,
but public ownership remains `EngineRuntime` plus narrow subsystem handles.

**Alternative:** add a new `EngineApplication` above `EngineRuntime`. This is
not recommended unless a prototype proves `EngineRuntime` cannot own renderer,
readiness, and reverse disposal without becoming internally incoherent.

### CUE-DEC-003 — Official Three.js compatibility mode

**Recommended:** retain unmodified official Three.js examples as an isolated
`afterglow-shell` compatibility/test profile. It may use first-submitted-frame
readiness, but it does not define engine `GameReady`, engine capacities, or
release evidence. Production engine pages use the stricter engine lifecycle.

**Alternative:** remove arbitrary-example support and make the shell engine-only.

### CUE-DEC-004 — Diagnostic harness shipping

**Recommended:** production visual bundles contain no `window.__afterglow*`
globals, frame waiters, scenario objects, or screenshot controls. Build separate
diagnostic entrypoints from the same authored game module and inject one typed,
versioned harness only in test/profile builds.

**Alternative:** ship dormant harness globals in production. This is simpler but
keeps test ownership and allocating diagnostics in the product surface.

### CUE-DEC-005 — Model GPU boundary

**Recommended:** every engine-owned model uses `GeometryArena`; `ModelSystem`
requires an arena configuration and can never silently publish ordinary growing
Three geometries. Direct Three geometry remains game-owned and outside model
streaming/revision guarantees.

**Alternative:** allow an unbounded `ModelSystem` mode. This would make fixed
GPU ownership optional and is not recommended.

### CUE-DEC-006 — Visual release policy

**Recommended:** require both semantic pixel gates and tolerant approved
references. Semantic gates check non-background coverage, luminance/color
variance, expected object regions, and HUD/fatal-panel absence. Tolerant
references catch materially wrong textures, poses, lighting, and sizing without
requiring bit-identical output across drivers.

**Alternatives:** semantic checks only, or exact/golden comparison only. The
former misses plausible but wrong images; the latter is too driver-sensitive.

### CUE-DEC-007 — Truthful conformance status

**Recommended:** change repository release status from `conformant` to
`converging` at the start of execution. Return to `conformant` only when this
plan’s deletion and release gates pass. “Canonical demo source shape” must no
longer imply full visual/runtime conformance.

**Alternative:** keep `conformant` and add a second cleanliness status. This
creates two truths and is not recommended.

## 4. Technical gates, not user decisions

These are answered by prototypes and measurements:

### CUE-TG-001 — Runtime-owned reverse lifecycle

Prototype a fixed-capacity internal owner table that can unwind synchronous
`dispose()` and asynchronous `close()` owners exactly once without storing
per-frame closures or hiding shutdown order. Compare extending the current
`BootstrapGuard` with a typed owner table. Select the smaller mechanism; expose
no general plugin API.

### CUE-TG-002 — Cross-target readiness signal

Prove one engine readiness record can represent:

```text
module evaluation complete
+ runtime sealed
+ no pending bootstrap readiness token
+ first successful game update
+ first successful main render
+ successful surface/canvas presentation
+ zero fatal diagnostics
```

The web harness observes it; `afterglow-shell` consumes it without authored
demos calling native ops. Device loss or a fatal frame error invalidates it.

### CUE-TG-003 — Portable visual capture

Implement a diagnostic-only bounded capture path:

- public web: CDP `Page.captureScreenshot` plus engine status;
- native shell: copy the final composited surface into a fixed staging buffer
  before present, map outside the hot path, and encode PNG diagnostically;
- compositor capture is retained only for OS-window sizing/presentation checks.

Prove capture does not create a production readback path and is absent after a
production build.

### CUE-TG-004 — OPFS RingBuffer worker

Measure chunk size, in-flight bytes, list/open cost, interrupted transactions,
and shutdown for an OPFS worker driven exclusively by bounded RingBuffer
payloads. `postMessage` remains wake/init only. Select the smallest service
implementation that preserves the existing `PersistentBlobStore` contract.

### CUE-TG-005 — Complete geometry layouts

Render rigid, skinned, and morphed cooked fixtures through prewarmed arena
buckets with materials, shadows, VT feedback, animation, and LOD transitions.
Prove no post-seal GPU buffer/pipeline creation and no partial revision.

## 5. Stop-the-line invariants

Every phase must preserve these rules:

- no new `*Session`, compatibility wrapper, service-specific transport, or
  duplicate owner;
- a replacement and deletion land in the same change;
- demos import only public engine APIs and never generated clients/transports;
- capacities are mandatory and overflow is typed;
- production APIs expose handles/views, never physical stores or worker IDs;
- one target-specific implementation may exist behind an interface, but callers
  cannot select a second ownership model;
- a log-only smoke is never visual evidence;
- no phase is complete until focused tests, architecture contracts, generated
  artifacts, API docs, and mdBook agree;
- temporary prototype code and evidence files are removed or promoted before the
  phase closes.

## 6. Execution sequence

The order is mandatory because each phase removes assumptions needed by the
next one.

## Phase 0 — Make status and tests truthful

### CUE-000 — Reopen conformance

**Status:** complete (2026-07-25).

- Add `converging` to the conformance schema and set it while this plan is open.
- Split “canonical source architecture” from “release conformant.”
- Add a machine-readable deletion ledger containing every symbol/path in
  Section 7.
- Make CI fail if a deletion item is reintroduced after removal.
- Correct docs that claim absent GPU scripts or current evidence.
- Version release evidence schema as v2; reject v1’s unaudited `ok: true` field.

### CUE-001 — Capture the current failures

**Status:** in progress. The repeated-wake frame-deadline regression, fatal
browser console diagnostics, strict `GameReady` first-frame contract, and
release-schema black-pixel/size rejection are implemented. The native host now
captures its final composited surface through a diagnostic-only preallocated
staging buffer after a bounded post-`GameReady` settling period. Public-web CDP
capture and cross-layer resize execution remain; Chromium 150 on the current
Nix/NVIDIA host was measured failing to commit even a localhost navigation and
its WebGPU adapter probe hangs, so that host is not accepted as evidence.

Add regressions that fail against the pre-fix behavior:

- shell startup wakes cannot push the next frame deadline beyond one interval;
- `renderer ready` cannot publish before engine `GameReady`;
- a page that presents only black/background pixels fails visual acceptance;
- initial and runtime resize must agree across OS window, surface, canvas
  physical size, camera aspect, and VT feedback dimensions;
- fatal browser/runtime errors fail readiness and remain inspectable.

**Done when:** the repository can no longer call a log-only or black frame a
successful smoke.

## Phase 1 — One lifecycle and readiness root

### CUE-010 — Runtime owns renderer lifecycle

**Status:** complete (2026-07-27). `await EngineRuntime.forScene()` creates and
registers the sole `RendererHost`; its fixed owner table handles synchronous and
asynchronous reverse rollback/shutdown, and all five demos have deleted direct
renderer construction and cleanup graphs.

- Make the normal constructor create/own `RendererHost` from scene, camera,
  container, and renderer capacities.
- Remove demo calls to `RendererHost.create()`, manual renderer registration,
  `BootstrapGuard`, `PageShutdown`, and duplicate shutdown sequences.
- Runtime owns reverse disposal and bootstrap rollback exactly once.
- Keep narrow accessors needed for game presentation (`renderer`, device
  identity, diagnostics), not mutation of private lifecycle state.

### CUE-011 — One readiness state machine

**Status:** in progress. `EngineRuntime` requires exactly one presentation
pass and now publishes `GameReady` only after a complete post-seal
update/presentation, required worker publication (including VT startup
residency), and zero diagnostics. It exposes caller-owned readiness snapshots,
signals the native host once, captures global/GPU/device failures immediately,
and owns one fatal panel. `--compat-three` isolates first-present readiness.
Named startup-token timeout reporting remains.

- Add `Bootstrap -> Warmup -> GameplaySealed -> Starting -> GameReady ->
  Suspended/DeviceLost/Fatal -> Shutdown` to the engine-facing state model.
- Publish `GameReady` only after CUE-TG-002’s complete predicate.
- Keep shell lifecycle and engine lifecycle mapped explicitly; do not infer
  engine readiness from any surface presentation during top-level await.
- Startup timeout diagnostics list pending subsystem tokens and the last
  successful stage.
- A fatal frame stops scheduling, shows one bounded fatal panel, and causes
  nonzero native exit/test failure.

### CUE-012 — One subsystem registration mechanism

**Status:** complete (2026-07-27). Renderer and VT worker/pass admission is
runtime-owned and atomic; the fixed owner table unwinds other subsystem owners
before passes. Demos contain no manual worker/render registration or cleanup
stack.

Keep fixed worker and render-pass tables, but centralize admission, warm, seal,
poll/render ordering, and disposal in runtime-owned records. Do not add dynamic
plugins or event-emitter lifecycle hooks.

**Done when:** each demo has one runtime construction, one game update callback,
and no lifecycle cleanup graph.

## Phase 2 — One virtual-texture public API

### CUE-020 — Make physical stores internal

**Status:** complete (2026-07-25). The public barrel exports only
`VirtualTextureSystem`, opaque handles/material sets, and caller-owned info
queries; physical pool inspection is an internal symbol capability.

Rename or retain the physical store internally, but remove it from public
barrels and game-facing types. `VirtualTextureSystem` becomes the only public VT
owner and exposes:

- generational texture handles;
- immutable descriptor/view queries into caller-owned output;
- material-set handles;
- mutable writes and persistence operations;
- stable stats/telemetry;
- feedback/renderer integration owned internally.

Do not return `VirtualTextureStore` from `resolveMaterialStore()` or
`VirtualTextureView`.

### CUE-021 — One material binding contract

**Status:** complete (2026-07-25). POM, glTF, and procedural bindings accept one
`VirtualTextureSystem` plus opaque handle sets; duplicate store-owner unions and
demo pool resolution are deleted.

Replace the repeated duck-typed
`VirtualTextureStore | { resolveMaterialStore(...) }` unions with one narrow,
non-exported material-resolution mechanism implemented by
`VirtualTextureSystem`. Public bindings consume system handles/material-set
handles:

- `VirtualMaterialBinding`;
- `VirtualGltfBinding`;
- `VirtualPomSceneBinding`;
- shader/node bindings.

Material factories may receive an internal resolved pool only inside the VT
module. Demos never call `getView()`, `resolveMaterialStore()`, or attach a
physical pool.

### CUE-022 — Integrate feedback and renderer attachment

**Status:** complete (2026-07-27). Runtime atomically owns the feedback
coordinator's worker/pass records, performs the atlas-initializing render and
all-pool attachment during warm-up, and routes physical resize. Demos no longer
attach or resize feedback infrastructure.

- `VirtualTextureSystem` owns registration with one coordinator or implements
  the bounded worker/render-pass interfaces itself.
- Renderer attachment applies to all configured pools once during warm-up.
- Demos do not call `attachVirtualTextureStore`, manually pair coordinator/store,
  or calculate physical feedback dimensions.
- Multi-pool feedback routes by stable texture ID without broadcasting every map
  to unrelated pools.

### CUE-023 — Delete the parallel VT path

**Status:** complete (2026-07-25). The legacy constructor, ECS resource,
AssetStore texture switch, public pool/pass exports, duck-typed resolvers, and
demo physical-view calls are removed and ratcheted.

Delete all entries in the VT section of the ledger, update tests to construct
systems/fixtures, and move direct store tests to an internal test module.

**Done when:** `rg` finds no game/demo/public API construction or exposure of
`VirtualTextureStore`; one system owns every texture source and physical pool.

## Phase 3 — One asset and model path

### CUE-030 — Remove texture policy from `AssetStore`

**Status:** complete (2026-07-25). The optional owner, setter/getter,
`loadTexture()` switch, and browser-image parser are deleted.

- Delete `AssetStore.vtStore`, setter/getter, and its VT/non-VT `loadTexture()`
  fallback.
- Keep `AssetStore` as a bounded generic raw/parsed asset mechanism.
- Resident textures use the resident texture API; virtual textures use
  `VirtualTextureSystem`. No method switches semantics based on optional state.

### CUE-031 — Finish cooked deformation-aware LODs

Complete UPR-040/041:

- emit and validate versioned `EXT_afterglow_mesh_lods`;
- preserve all attributes, morph targets, groups, skins, hierarchy, and clips;
- adopt cooked records without runtime simplification;
- delete the old rigid `static-lod` record and bundled assets;
- reserve runtime simplification for an explicitly named runtime-created model
  API, not ordinary cooked loading.

### CUE-032 — Make bounded geometry mandatory

**Status:** complete (2026-07-25). `ModelSystemOptions.geometryArena` is
mandatory; the system creates, owns, reports, and disposes it, and all optional
publication/release branches are deleted.

- `EngineAssets.createModelSystem()` accepts arena bucket capacities in its
  options and creates one mandatory arena.
- `ModelSystem` owns the arena publication and cannot be constructed in an
  unbounded mode.
- Remove optional arena branches and direct disposal of published geometries.
- Make complete revision preflight/publication/retirement the only path.
- Warm every declared layout/material variant before seal.

### CUE-033 — Thin model bindings

`ModelLodBinding` consumes model handles and published views. Skeleton/morph
sharing remains a presentation mechanism; source parsing and GPU ownership do
not leak to demos. Rigged-VT uses cooked model records and no runtime mesh
optimizer.

**Done when:** engine-owned model CPU/GPU bytes are fixed and every cooked model
uses one source -> model system -> arena -> binding path.

## Phase 4 — One persistence service contract

### CUE-040 — Move OPFS off the page

**Status:** complete (2026-07-27). Public web uses the generated
`BlobStorageClient` over the standard shared RingBuffers and a dedicated
`storage-worker.ts`; OPFS exists only in that Worker, with 512 KiB chunks and
eight fixed transaction slots.

- Replace page-side `OpfsPersistentBlobBackend` I/O with the result of
  CUE-TG-004.
- The page owns only `PersistentBlobStore` and a generated/bounded client.
- Worker transactions retain two-generation checksums and atomic pointer
  publication.
- List/read/write/remove/clear have fixed transaction, item, value, chunk, and
  in-flight byte capacities.

### CUE-041 — Enforce native/web parity

Run one backend conformance suite against memory, native OS worker, and web OPFS
worker. The same operation sequence must produce the same typed status,
generation retention, corruption behavior, and telemetry.

### CUE-042 — Remove the page backend

**Status:** complete (2026-07-27). `OpfsPersistentBlobBackend` and its direct
page test/export were deleted and ratcheted; crash-retention tests now exercise
the Worker service.

Delete the page-side OPFS implementation and its direct tests. Keep test-only
fake filesystem adapters only inside the worker’s tests.

**Done when:** both release targets cross a RingBuffer service boundary for
persistent payloads and callers cannot observe backend policy.

## Phase 5 — Separate product, diagnostics, and evidence

### CUE-050 — Diagnostic build surface

**Status:** complete (2026-07-27). Production demo bundles contain no global
harnesses, frame waiters, scenario registries, or bootstrap/error-capture
helpers. Five separate diagnostic pages install the sole typed version-1
`__afterglowDiagnosticV1` protocol.

- Replace per-demo `publishDevHarness("__afterglow...")` calls with one typed,
  versioned diagnostic protocol.
- Build it only for diagnostic pages/artifacts.
- Move frame stepping, scenarios, snapshots, and formatting out of production
  bundles.
- Keep bounded `EngineDiagnostics` in production; keep capture/scenario control
  outside it.

### CUE-051 — Reusable visual acceptance runner

**Status:** blocked on a measured technical gate (2026-07-27). Native
GameReady-driven PNG capture works for the diagnostic pages and has been
byte-inspected. The web runner is not implemented because Chromium 150 on the
current Nix/NVIDIA host leaves CDP navigations at an uncommitted empty document;
`navigator.gpu.requestAdapter()` hangs under Vulkan, GL, and unsafe-WebGPU flag
variants. Do not substitute a screenshot from that process or mark evidence
current. The next acceptance prototype must first prove committed COOP/COEP
navigation plus a hardware adapter in a fresh process.

Add one command:

```text
cargo run -p xtask -- visual
```

For every visual manifest entry and both release targets it:

1. builds current artifacts;
2. launches with a fresh profile/process;
3. waits for `GameReady`, never a console phrase;
4. captures engine status plus pixels;
5. validates semantic pixels and tolerant reference;
6. resizes to two non-proportional sizes and repeats capture;
7. checks window/surface/canvas/camera/feedback dimensions;
8. checks adapter policy, errors, overflows, pending work, and post-seal
   pipelines;
9. shuts down and proves workers/processes exited;
10. writes versioned result JSON and image hashes.

The runner must fail if capture is unavailable. A human looking at a window is a
useful debugging step, not release evidence.

### CUE-052 — Evidence schema v2

**Status:** schema and validator complete; current evidence intentionally absent
until CUE-051 and the soak lanes produce real records.

Each result records:

- commit and all relevant artifact/asset hashes;
- target, OS, host/browser, Three, adapter, driver, and display scale;
- logical window, physical window, surface, canvas, and feedback sizes;
- readiness stages/timestamps;
- semantic pixel metrics and reference diff;
- frame/GPU percentiles;
- heap and CPU/GPU resource totals;
- queue high-water/overflow and pending-at-end;
- screenshot hash/path;
- exact pass/failure reasons.

Delete `ok: boolean` as sufficient evidence.

**Done when:** the regression that produced black demos cannot satisfy the
visual command or release schema.

## Phase 6 — Make demos genuinely thin

Migrate all five visual demos to the final APIs. A demo may contain:

- scene objects and game state;
- explicit capacity profile selection;
- camera/input/game update policy;
- material/model/texture handle declarations;
- visual controls and credits;
- benchmark scenario definitions in diagnostic companions.

A demo may not contain:

- renderer creation/registration/attachment;
- bootstrap cleanup stacks;
- physical VT stores or pool resolution;
- worker/client/source assembly;
- model optimization/GPU publication;
- global test harnesses;
- readiness or screenshot policy;
- repeated engine diagnostics/HUD infrastructure.

Add architecture rules for every forbidden item above. Migrate one demo at a
time and delete the superseded code in the same change. Dungeon and rigged-VT
must pass screenshot review after each migration, not only at phase end.

**Done when:** each entrypoint is primarily scene/presentation code and the
architecture lint reports zero infrastructure ownership.

## Phase 7 — Enforcement, soak, and final deletion

### CUE-070 — One command hierarchy

```text
cargo run -p xtask -- conformance   # static contracts/docs/artifacts
cargo run -p xtask -- test          # conformance + Rust/Bun/integration
cargo run -p xtask -- visual        # current web/native visual evidence
cargo run -p xtask -- soak          # bounded long-run scenarios
cargo run -p xtask -- release-gate  # all of the above + evidence validation
```

Commands discover tests and visual entries from authoritative manifests. Docs
may not name scripts that do not exist. Missing lanes are failures, not skips.

### CUE-071 — Combined soaks

Complete the open UPR/Shell evidence on public web and native:

1. 30-minute Dungeon traversal/streaming;
2. 30-minute mutable painting plus snapshot churn;
3. 30-minute rigged animation/model/LOD switching;
4. 30-minute combined streaming + mutable overlay + model switching;
5. hostile page/geometry/persistence capacity thrash;
6. forced worker cancellation/restart and device-loss handling.

Require plateaued heap, CPU/GPU bytes, queue depths, timers, pending tasks, and
frame/GPU timing; zero unexplained errors, stale publications, post-seal
pipelines, or pending-at-end work.

### CUE-072 — Final documentation/deletion pass

- Delete every completed ledger item and its obsolete tests/docs.
- Mark the overlapping implementation plans complete or superseded with links
  to their final evidence.
- Update `AGENTS.md`, all affected `docs/api/`, and mdBook chapters from the
  implemented API.
- Remove stale CEF commands and historical behavior from current API docs.
- Rebuild bundled assets and generated web output.
- Set conformance to `conformant` only after `release-gate` passes.

## 7. Initial deletion ledger

This ledger is expanded by CUE-000 and then machine checked.

### Virtual texturing

- `EngineAssets.createVirtualTextureStore()`;
- `EngineAssets.store` parallel owner;
- public `VirtualTextureStore` barrel export;
- `VirtualTextureRes`;
- `AssetStore.setVirtualTextureStore()` and `virtualTextureStore`;
- `AssetStore.loadTexture()` optional VT/basic fallback;
- public `VirtualTextureFeedbackPass` export;
- `VirtualMaterialStoreOwner` duplicates in POM/glTF bindings;
- `VirtualTextureStore | VirtualMaterialStoreOwner` public unions;
- demo `getView()`, `resolveMaterialStore()`, and renderer-attach calls;
- store-specific debug adapter exposed to game code.

### Models

- optional `GeometryArena` parameter and unbounded publication branch;
- ordinary cooked-model runtime optimization/simplification path;
- old rigid `static-lod` records and assets;
- demo-owned arena construction for engine-managed models;
- duplicate cooked/runtime publication semantics.

### Persistence

- page-side `OpfsPersistentBlobBackend`;
- direct page OPFS iteration, file arrays, and pointer writes;
- any storage payload `postMessage` path.

### Lifecycle and diagnostics

- demo `RendererHost.create()` and manual renderer registration;
- demo `BootstrapGuard`/`PageShutdown` lifecycle graphs;
- first-present-as-engine-ready logic;
- production `window.__afterglow*` globals;
- per-demo frame waiter/scenario/error-capture ownership;
- log-only renderer-ready smoke acceptance;
- release evidence v1 `ok` booleans.

### Documentation/tooling

- current docs naming nonexistent GPU scripts;
- duplicated open completion claims across implementation plans;
- stale current CEF instructions;
- any generated `www/` source treated as authored.

## 8. Required tests per change

Every implementation change must include the applicable subset:

- regression demonstrating the old duplicate/failure;
- owner construction, partial bootstrap rollback, idempotent disposal;
- exact capacity boundaries and typed overflow;
- stale handle/completion rejection;
- allocation-effect and no-post-seal-resource tests;
- target-boundary test proving native OS worker versus public-web Worker;
- public API compile test proving removed symbols are unavailable;
- architecture test proving demos cannot recreate the deleted path;
- browser/native vertical integration;
- visual screenshots for rendering/resize changes;
- API docs, mdBook, artifact drift, and deletion-ledger checks.

## 9. Phase completion rule

A phase is not complete because its replacement exists. It is complete only
when:

```text
replacement implemented
+ all consumers migrated
+ old path deleted
+ old public symbol unavailable
+ architecture contract prevents reintroduction
+ unit/vertical/visual gates pass
+ docs and book describe only the replacement
```

## 10. Final definition of done

All statements must be true:

- [x] CUE-DEC-001–007 are recorded.
- [x] One `EngineRuntime` owns renderer, readiness, registration, rollback, and
      reverse shutdown.
- [x] `GameReady` requires a successful post-seal gameplay presentation and
      zero fatal diagnostics.
- [ ] Every first-party demo passes web and native semantic/reference screenshot
      checks at two sizes.
- [ ] Window, surface, canvas, camera, and feedback dimensions agree after
      startup and runtime resize.
- [ ] `VirtualTextureSystem` is the sole public VT owner; physical stores are
      internal.
- [ ] Material bindings consume handles and share one resolver mechanism.
- [ ] `AssetStore` contains no optional VT policy or texture fallback.
- [ ] Every engine-owned model publishes only through a mandatory
      `GeometryArena`.
- [ ] Cooked skinned/morphed LODs require no runtime simplification.
- [x] Native and public-web persistence payloads both cross bounded RingBuffer
      services.
- [x] Production bundles contain no dev harness globals or capture/scenario
      machinery.
- [ ] Demos contain game/presentation policy only.
- [x] The deletion ledger is empty and protected by contracts.
- [x] Allocation, complexity, capacity, and target-boundary gates pass.
- [ ] Combined 30-minute web/native soaks plateau with no unexplained failures.
- [ ] `xtask release-gate` runs every declared lane and validates evidence v2.
- [ ] API docs, mdBook, `AGENTS.md`, manifests, generated artifacts, and bundled
      assets describe one implemented system.
- [ ] Repository status is `conformant` only after all checks above pass.
