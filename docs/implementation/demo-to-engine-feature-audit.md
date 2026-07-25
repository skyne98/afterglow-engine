# Demo/example → engine feature audit

> **Stale.** `afterglow-cef` and its five example launchers have been removed.
> References to `afterglow-cef/examples/*.rs` and `AppBuilder` below are
> historical. Rehoming the demos under `afterglow-shell` is tracked in
> `docs/implementation/shell-promotion-plan.md` (gate G3). This audit is
> retained as the design record of the demo→engine feature mapping.

**Date:** 2026-07-16

Concrete remediation tasks, dependency order, tests, CI ratchets, and final
release gates are defined in
[`demo-engine-migration-execution-plan.md`](demo-engine-migration-execution-plan.md).

## Purpose

The demos currently prove useful behavior, but several of them also implement
runtime infrastructure locally. This audit identifies code that must move into
small, typed, bounded engine mechanisms before a demo can be treated as an
example of the intended public architecture.

This is not a request to turn scene policy into engine policy. Wall placement,
light colors, model selection, benchmark scenarios, and procedural art remain
in demos. Ownership, lifecycle, bounded frame orchestration, asset/worker
bootstrap, render adaptation, VT feedback, material binding, diagnostics, and
test control belong in reusable engine or tooling APIs.

## Scope read completely

Authored browser entrypoints:

- `www/dungeon.ts` and `dungeon.html`
- `www/engine-demo.ts` and `engine-demo.html`
- `www/lod-demo.ts` and `lod-demo.html`
- `www/rigged-vt-demo.ts` and `rigged-vt-demo.html`
- `www/vt-demo.ts` and `vt-demo.html`
- `www/vt-mip-inspector.ts` and `vt-mip-inspector.html`
- `www/worker-test.ts` and `worker-test.html`
- `www/worker-bench.ts` and `worker-bench.html`
- `www/engine-bundle-input.ts`

First-party Rust examples:

- all five `afterglow-cef/examples/*.rs` entrypoints
- `afterglow-rpc-demo/examples/bench_rpc.rs`
- `afterglow-web/examples/coep_server.rs`

Direct demo-support implementations were also inspected where needed to avoid
proposing APIs that already exist: `bench.ts`, `frame.ts`, `frame-budget.ts`,
`render-adapter.ts`, `matrix.ts`, `components.ts`, `renderer-seal.ts`,
`webgpu-only.ts`, `procedural-vt.ts`, and `vt-gpu-test.ts`.

Vendored Three.js examples and `prototype/vt` verification tests are not
runtime demos and are outside this inventory.

## Executive verdict

The largest problem is not missing rendering capability. It is that the demos
frequently bypass capability already present in the engine and assemble their
own unbounded mini-runtime. No current visual demo exercises the complete
canonical path:

`EngineMemory → resource seal → FrameBudget → prepareAfterglowFrame → RenderAdapter → renderer seal`.

As a result, demos can look correct while violating the architecture they are
supposed to demonstrate. The following are P0 migrations:

1. make the canonical sealed frame runtime unavoidable in authored demos;
2. move dungeon POM graph construction into a tested engine material adapter;
3. move glTF→VT binding and source-texture replacement into the asset system;
4. add a bounded VT feedback coordinator instead of per-demo material swaps;
5. remove the global `window.THREE`/`window.Afterglow*` bundle bridge;
6. enforce hot-path allocation linting on entrypoint frame callbacks.

## P0 findings

### P0.1 — Demos bypass the canonical engine runtime

**Evidence**

- `engine-demo.ts:44-123` creates plain array components and duplicates raw
  matrix composition even though `components.ts`, `matrix.ts`, and
  `RenderAdapter` already own typed stores, dirty queues, descriptor capacity,
  proxy pools, and matrix upload.
- `engine-demo.ts:189-241`, `lod-demo.ts:260-296`, `vt-demo.ts:8`,
  `dungeon.ts:177`, and `rigged-vt-demo.ts:408-467` each define a different
  frame loop.
- None of the visual demos constructs `EngineMemory`, seals resources through a
  manifest, uses `FrameBudget`, or calls `prepareAfterglowFrame`.
- `engine-bundle-input.ts:27-30` exports memory and frame-budget types globally,
  but demos do not consume them.

**Why this is an engine issue**

The no-runtime-allocation and bounded-work guarantees cannot be demonstrated or
regressed if examples bypass the only APIs that enforce them. The
`engine-demo.ts` matrix loop is now a parallel legacy renderer path.

**Required feature**

Add one typed `EngineRuntime` ownership primitive that composes the existing
mechanisms rather than replacing them:

- owns the single `EngineMemory` resource, `FrameBudget`, bounded diagnostics,
  registered worker pollers, `RenderAdapter`, optional VT coordinator, and
  renderer seal;
- has explicit `Bootstrap`, `Warmup`, and `GameplaySealed` transitions;
- invokes `prepareAfterglowFrame` in a fixed order and then calls a supplied
  render callback;
- owns resize, device-loss, uncaptured-error, and disposal listeners;
- exposes stable typed telemetry without snapshot allocation.

Scene creation and game systems remain callbacks supplied by the consumer.
Delete the matrix/component/frame implementation from `engine-demo.ts`; rewrite
that demo to use `RenderAdapter` descriptors and dirty marking.

**Acceptance**

- every visual entrypoint reaches `GameplaySealed` before its animation loop;
- entrypoints cannot poll workers or VT stores outside the runtime stage order;
- no post-seal resource/pipeline creation in a 30-minute demo soak;
- `engine-demo` uses no parallel transform or instance implementation.

### P0.2 — Dungeon owns a second POM material system

**Evidence**

`dungeon.ts:80-126` locally implements:

- POM-aware feedback material construction;
- VT sample-at-level and displaced fallback nodes;
- geometric TBN and tangent-space view/light conversion;
- the POM march and self-shadow visibility;
- complete base/POM `MeshStandardNodeMaterial` graphs;
- linked albedo/normal/mask fallback selection.

Only low-level WGSL strings and contract helpers live in `surface-detail.ts`.
This means another game must copy the dungeon to use the engine's validated POM
path.

`dungeon.ts:91-94` also subclasses Three's lighting model and multiplies
`reflectedLight.directDiffuse` and `directSpecular` after `super.direct()` for
each light. Those fields are accumulated lighting, so a later light can
re-attenuate earlier lights. This is the known multi-light correctness bug.

**Required feature**

Create a geometry-agnostic `createVirtualPomMaterialPair()` next to
`createVirtualGltfMaterialPair()`:

- visible base, visible POM, base-feedback, and POM-feedback variants;
- one linked PBR mip and one marched UV;
- geometric TBN with explicit tangent requirement/failure;
- configurable bounded layers, height scale, offset limit, distance policy,
  and self-shadow steps;
- per-light visibility applied to only that light's contribution;
- fixed variant set suitable for renderer warm-up;
- no demo access to Three private lighting internals.

Material policy remains an options object. The engine adapter owns the shader
wiring and tests.

**Acceptance**

- dungeon contains no WGSL plumbing, custom lighting subclass, or VT sampling
  helpers;
- a two-light oracle proves one light's visibility cannot attenuate another;
- base/POM visible and feedback variants are covered by generated-WGSL tests;
- tangent absence fails during bootstrap, not after sealing.

### P0.3 — glTF VT binding is demo-local and transiently decodes all source images

**Evidence**

`rigged-vt-demo.ts:139-209` and `234-298` traverse meshes, infer material
layouts, load virtual image entries, copy glTF scalar factors, replace
materials, collect browser textures, close image data, and dispose source
materials.

Specific hacks include:

- `secondLayouts` is keyed by material **name** (`rigged-vt-demo.ts:253-260`),
  despite `GltfMaterialTextureLayout` already carrying a stable numeric index;
  duplicate/empty names can bind the wrong texture set;
- original embedded images must be decoded by `GLTFLoader` before they are
  disposed (`rigged-vt-demo.ts:197-206`, `288-296`), causing enormous transient
  allocation for 45 source images;
- texture property lists and disposal rules are duplicated for each model;
- missing-base-color materials fall through to a separate source-material path,
  complicating feedback and lifetime ownership;
- all records are `any`, so the adapter contract is not type checked.

**Required feature**

Add an offline/runtime pair:

1. The pipeline writes a runtime GLB without image payload references while
   retaining a compact material-role manifest in the `.big` container. Runtime
   model parsing must never browser-decode textures that VT will replace.
2. `bindVirtualGltfMaterials(asset, vtStore, options)` binds by stable material
   index and returns one owned binding object with visible materials, feedback
   variants, bootstrap warm variants, and `dispose()`.
3. The binding object implements the generic feedback-renderable contract from
   P0.4 and preserves alpha mode, factors, side, depth-write, normal scale, and
   texture transforms.

**Acceptance**

- loading the Dragon model never creates 45 browser image bitmaps;
- duplicate material names are covered by a regression test;
- the demo has no source texture-property list or manual material disposal;
- all material records have concrete public types and one owner.

### P0.4 — VT feedback orchestration is reimplemented per demo

**Evidence**

- Dungeon creates a duplicate feedback scene and duplicate wall meshes
  (`dungeon.ts:79`, `127-137`). Transform/material state can drift from the
  visible object.
- Rigged VT swaps materials on live objects, manually hides unsupported meshes,
  allocates four passes, merges pass results, preserves camera layer masks,
  disables shadows around feedback, and restores all state
  (`rigged-vt-demo.ts:130-133`, `207-210`, `329-375`, `377-395`, `425-444`).
- The atomic multi-pass merge was necessary to prevent one pass from canceling
  another pass's pages. That correctness rule currently exists only in the
  demo.
- `vt-demo.ts:6-8` does not use GPU feedback at all. It builds a new `Map`,
  request object, and dynamic string key for every visible page every frame,
  despite presenting itself as the production VT demo.

**Required feature**

Build a fixed-capacity `VirtualTextureFeedbackCoordinator` over the existing
low-level pass:

- registers bounded `FeedbackRenderable` records during bootstrap;
- owns pass count, targets, resize, cadence, consume, and atomic merge;
- guarantees every pass in one logical snapshot is present before
  `processFeedback`;
- brackets renderer state (target, camera layers, shadow enable) with one
  ownership model;
- supports same-object material override and explicit alternate feedback scene
  adapters without duplicating policy in the coordinator;
- publishes stable pass/mip/overflow telemetry;
- warms every target format before renderer seal.

**Acceptance**

- no demo directly calls `VirtualTextureFeedbackPass.submit/consume`;
- no demo swaps feedback materials or toggles renderer shadow state;
- `vt-demo` uses real GPU feedback;
- missing/late pass regression tests cannot trigger cross-pass stale
  cancellation.

### P0.5 — Global bundle bridge destroys module identity and typing

**Evidence**

`engine-bundle-input.ts:21-45` writes Three, TSL, bitecs, and engine APIs into
ad-hoc `window.*` namespaces. Demos then recover them as `any`. Other demos
also directly import modules that import a second `three` package graph. This is
consistent with the observed "Multiple instances of Three.js" warning and
allows mismatched constructors/type identities.

**Required feature**

Provide a typed engine barrel module and bundle each entrypoint from that single
module graph. Remove `engine-bundle-input.ts`, `window.THREE`,
`window.Afterglow*`, and separate `<script src="engine-bundle.js">` tags.
Generated deployment output remains one bundle per entrypoint (plus worker
scripts), while authored `.ts` uses direct `.ts` imports.

**Acceptance**

- no engine API is obtained from `window`;
- one Three module identity exists per page;
- no `any` is needed at bundle boundaries;
- the build test rejects global engine namespace writes.

### P0.6 — Entrypoint hot paths are outside allocation enforcement

**Evidence**

Examples of sealed-loop allocations or unbounded growth:

- all demos append errors to unbounded arrays;
- `engine-demo.ts:147-168` grows arrays, copies, sorts, reduces, and filters in
  the frame benchmark despite an existing `engine/bench.ts`;
- `vt-demo.ts:6-8` allocates a map, request objects, and string keys every frame;
- `rigged-vt-demo.ts:446-449` and `dungeon.ts:177` splice waiter arrays;
- `rigged-vt-demo.ts:457-466` uses `slice`, `flatMap`, dynamic strings, and
  `innerHTML` in gameplay;
- every visual demo builds dynamic HUD strings in its loop;
- entrypoint animation callbacks are not marked as hot regions, so current lint
  passes without inspecting them.

**Required feature**

- Extend allocation lint scope to every registered engine frame callback.
- Add a fixed-capacity `EngineDiagnostics` error/event ring.
- Treat HUD/benchmark/test work as explicit tracked diagnostic slow paths with
  rate, byte, and operation budgets.
- Replace promise waiter arrays with a dev-only fixed slot harness; exclude it
  from production bundles.
- Make `FrameBench` capacity-bounded and use it instead of the duplicate demo
  implementation.

## P1 findings

### P1.1 — BIG/VT/worker bootstrap needs one owned session

Dungeon (`dungeon.ts:27-70`) and rigged VT (`rigged-vt-demo.ts:81-116`)
repeat worker-count policy, RPC construction, cleanup, prefix reads, manual
`DataView` parsing, header reads, GPU format selection, loader adapters, page
provider creation, tuning, and store construction. Dungeon additionally builds
the derived-cache namespace and fallback behavior locally.

Add `BigAssetSession.open(options)` with explicit capacities and policy hooks.
It should own the range source, parsed header, raw asset loader, bounded
transcoder worker pool, selected GPU format, optional generic persistent cache,
page provider, telemetry, and shutdown. VT-specific cache-key policy remains a
thin consumer configuration; storage stays generic.

Do not hide I/O: `open()` is bootstrap async, capacities are mandatory, and the
returned session exposes ownership and byte limits.

### P1.2 — Renderer/device diagnostics and profiling need public adapters

Renderer creation, canvas attachment, resize, uncaptured errors, global errors,
device loss, and teardown are repeated in every demo. `webgpu-only.ts` handles
only initialization/device loss.

Worse, `dungeon.ts:138-152` reaches into Three private `_renderContexts`,
`timestampQueryPool`, timestamp IDs, and mutable frame arrays. A Three update can
silently break diagnostics.

Add:

- bounded diagnostics to the runtime owner;
- a `GpuPassProfiler` wrapping supported public/isolated backend access in one
  compatibility module;
- named main/feedback pass timing, fixed query capacity, and slow diagnostic
  resolution;
- backend-version contract tests.

No demo may access underscored Three fields or mutate timestamp pools.

### P1.3 — Shader contract interception belongs in renderer validation

`dungeon.ts:166-173` monkey-patches `GPUDevice.createShaderModule`, scans shader
source strings, counts variants, and restores the method manually. This is a
global mutable hook with fragile restoration.

Move generated-WGSL validation into material factory tests and, where runtime
validation is still required, a bootstrap-only `ShaderContractRegistry` owned
by renderer warm-up. It must compose multiple contracts, restore through
`try/finally`, and be unavailable after seal.

### P1.4 — Model presentation has reusable mechanism mixed with scene policy

`rigged-vt-demo.ts:139-177`, `153-165`, `214-227`, `234-252`, and `301-317`
implement reusable model mechanics twice:

- mesh collection and validation;
- engine-owned normalization pivot;
- exact deformed skinned bounds;
- animation mixer/action selection;
- grounding after evaluating the first animated pose;
- skeleton helper lifecycle;
- cast/receive-shadow propagation.

Extract small mechanisms, not a monolithic character system:

- `collectModelPrimitives()` with stable material/primitive metadata;
- `computeDeformedBoundsInto()` using caller-owned scratch;
- `normalizeModelPivot()` with explicit center/height/ground policy;
- `AnimationSet` owning mixer/actions and active-update policy;
- a disposable skeleton-debug adapter.

Model height, chosen clip, light setup, and whether grounding is desired remain
demo/game policy. Hidden mixers must not update each frame as they do at
`rigged-vt-demo.ts:412`.

### P1.5 — Input has raw events but no bounded action state

`RelativePointerInput` correctly owns pointer-lock compatibility, but demos each
create growing `Set<string>` state and custom listeners. Dungeon implements
first-person look/movement at `dungeon.ts:153-163`; rigged VT implements inertial
orbit/zoom at `rigged-vt-demo.ts:408-425`.

Add a fixed action/axis input state with bootstrap bindings, blur/reset,
programmatic disable, and stable button/axis storage. Orbit and first-person
controllers can be composable policy modules over that state. Do not promote
the dungeon's segment collision (`dungeon.ts:154-157`) into an engine collision
API: it is an O(wall-count) scene hack. Replace it eventually with the bounded
physics/character-controller path.

### P1.6 — The LOD demo does not demonstrate a runtime LOD system

`lod-demo.ts:37-100` generates geometry and a GLB in the browser;
`104-125` spawns workers and constructs an in-memory loader;
`140-157` manually creates one texture per mip; and `204-243` creates four
side-by-side meshes. It never selects LOD from screen coverage. It also refers
to undeclared `texHandle` at line 227 when textures are not ready.

This demo exercises a legacy/custom `AssetStore.loadModel` path rather than the
new glTF scene path. Decide one architecture:

- preferably delete the custom path and add a static-mesh `LodSet` produced by
  the offline pipeline, with fixed levels, error/coverage thresholds, residency
  policy, and a bounded selector integrated with render descriptors;
- keep skinned simplification disabled until deformation-aware error exists;
- make the demo show actual hysteretic runtime switching, not four manual
  meshes.

Sphere/GLB fixture generation belongs in a test fixture or offline cook, not
browser bootstrap.

### P1.7 — The "minimal engine" demo is stale parallel code

`engine-demo.ts` duplicates `composeTransformInto`, uses plain growable arrays,
assumes dense entity IDs (`composeMatrices` uses loop index as entity), disables
culling, updates the whole instance buffer, and calls un-awaited
`renderer.renderAsync` each rAF. It also duplicates `FrameBench` instead of
importing `engine/bench.ts`.

Rewrite it as the canonical small example of:

- `EngineRuntime` and sealed resources;
- one warmed instanced descriptor;
- `RenderAdapter` entity/transform APIs;
- bounded dirty updates and upload telemetry;
- the shared diagnostic benchmark permit.

The CEF example comment says 10,000 cubes while the page creates 5,000; fix the
single source of configuration rather than duplicating prose.

### P1.8 — Surface tangents should not be manually fabricated in a scene loop

`dungeon.ts:130-134` installs a literal tangent buffer for each wall so POM has a
stable local tangent. Tangent availability is an asset/geometry contract.

Add a bootstrap geometry preparation helper that validates or generates tangent
attributes and records whether tangents are authored/generated/unavailable.
Procedural plane construction may still choose its local tangent, but material
creation should consume a declared geometry capability rather than rely on a
commented convention.

## P2 tooling and example cleanup

### P2.1 — Deterministic browser test harness

Dungeon, rigged VT, and VT each export unrelated `window.__afterglow*` objects,
implement promise-based `step`, programmatic input suppression, snapshots,
idle loops, and scenarios (`dungeon.ts:175-232`, `rigged-vt-demo.ts:490-543`,
`vt-demo.ts:10`).

Create a dev/test-only `BrowserHarness` with fixed waiter capacity, frame
stepping, timeout/idle predicates, bounded errors, scenario registration, and a
single CDP-visible namespace. Production builds omit it. Scenario bodies remain
with their demos.

### P2.2 — Debug HUD

Repeated `innerHTML`, FPS smoothing, errors, help text, and telemetry formatting
should use one diagnostic overlay adapter. It is not gameplay UI. It must be
rate-limited and run under a diagnostic slow-path permit. HTML markup/style and
credits remain page-owned.

### P2.3 — VT inspector/generator placement

`procedural-vt.ts` and `vt-gpu-test.ts` are under `engine/` but are demo/test
content, not runtime engine mechanisms. `vt-mip-inspector.ts` also duplicates
mip downsampling, BMP encoding, object-URL creation, and panel assembly, without
revoking URLs.

Move procedural terrain/stone generation to `examples/support/`. Move raw GPU
validation to test/diagnostic support. Add a generic offline/debug texture
export utility only if a second inspector needs it; otherwise keep BMP/UI code
local and fix URL disposal. Do not expose procedural stone as engine API.

### P2.4 — Worker test and benchmark must use generated typed clients

`worker-test.ts:7-17` and `worker-bench.ts:6-37` encode payloads manually and call
hard-coded method ID `0`, bypassing the generated service client the project is
meant to validate. Change both to the generated `PhysicsClient`. Keep raw
transport benchmarks only when explicitly labelled as transport benchmarks.
Share payload sizes, warm-up count, validation semantics, and result schema with
`bench_rpc.rs` so native/web comparisons cannot drift.

### P2.5 — CEF examples should use the existing root API

Every CEF visual example embeds authored inline JavaScript solely to redirect:

```rust
.index_html(b"<script>location.href='/...html'</script>")
```

This appears at `afterglow-cef/examples/{minimal,dungeon,lod-demo,rigged-vt-demo,vt-demo}.rs:13-18` and violates the authored-script rule. `AppBuilder::root()` already exists. Replace each redirect with
`.root("/...html")`; no new engine feature is needed. A tiny shared example
configuration helper is justified only if more non-policy options are added.

### P2.6 — COOP/COEP server loop should be reusable tooling

`coep_server.rs:31-75` wraps an existing request handler in an unbounded
thread-per-connection server, reads request headers once into 8 KiB, and
allocates response strings per connection. Move serving ownership into a
bounded `DevAssetServer`/`xtask serve` tool with connection capacity, complete
header reads, graceful shutdown, stable telemetry, and browser launch options.
The `afterglow-assets` source and range mechanisms remain reusable and
policy-free.

### P2.7 — RPC benchmark harness

`bench_rpc.rs` is valid benchmark code, not runtime engine code, but it repeats
thread startup, spin loops, warm-up, row formatting, and statistics. Move it to
a dedicated benchmark/tool harness (or Criterion where appropriate), share the
service RPC case specification with web, and keep low-level directional ring
benchmarks clearly separate from service latency. Do not add benchmark concerns
to `afterglow-rpc`'s runtime API.

### P2.8 — Example lifecycle/disposal

LOD, engine, VT, and worker pages do not consistently own/remove event
listeners, dispose renderer resources, terminate all workers on failure, or
release object URLs. The runtime/session/harness owners above need idempotent
`dispose()` and bootstrap rollback. Tests must force a mid-bootstrap failure and
prove workers/listeners/GPU resources return to baseline.

## What should remain demo-local

The following are policy or fixtures and should **not** become broad engine
subsystems:

- dungeon wall coordinates, spawn poses, key choices, light placement, and
  synthetic atlas stress scenarios;
- Dragon/Decraniated selection, clip choice, presentation height, floor/grid,
  and credit UI;
- procedural terrain/stone appearance functions;
- benchmark display prose and scene-specific telemetry labels;
- sphere generation once moved to test/example support;
- wireframe comparison layout;
- exact camera choreography used by GPU regression scripts.

## Migration order

### Phase 0 — stop adding debt

1. Extend hot-allocation lint to entrypoint frame callbacks.
2. Replace CEF redirect scripts with `.root()`.
3. Make worker tests use generated typed clients.
4. Fix/remove the undefined LOD fallback and mark legacy model APIs for deletion.
5. Add an audit test that every visual demo seals memory/resources/renderer.

### Phase 1 — canonical runtime

1. Compose existing memory, frame budget, render adapter, diagnostics, and
   renderer seal into `EngineRuntime`.
2. Rewrite `engine-demo` first; it becomes the reference architecture.
3. Migrate resize/error/disposal and the dev harness.
4. Migrate remaining demos without changing visuals.

### Phase 2 — rendering/material adapters

1. Implement `VirtualTextureFeedbackCoordinator`.
2. Implement `createVirtualPomMaterialPair` and fix multi-light visibility.
3. Implement stable-index glTF VT binding.
4. Strip runtime GLB image payload references in the pipeline.
5. Add model bounds/normalization/animation helper primitives.

### Phase 3 — asset and worker ownership

1. Implement `BigAssetSession` and bounded worker-pool ownership.
2. Move persistent derived-cache setup behind explicit session policy.
3. Migrate dungeon and rigged VT bootstrap.

### Phase 4 — delete legacy paths and promote tooling

1. Replace/delete the custom LOD model path with real static glTF LOD sets.
2. Move procedural/debug support out of `engine/`.
3. Add bounded dev server and shared benchmark specifications.
4. Remove `engine-bundle-input.ts` and global namespaces.

## Completion gate

This migration is complete only when:

- every visual demo is a thin scene/policy consumer of the same sealed runtime;
- no demo contains worker-pool, BIG-header, VT-feedback, POM graph, glTF material
  replacement, matrix-compose, renderer-private profiling, or global engine
  bundle plumbing;
- all demo frame callbacks pass allocation lint;
- all queues/workers/resources have explicit capacity, telemetry, deterministic
  overflow, and one disposable owner;
- 30-minute demo soaks plateau in heap, pending work, atlas residency, worker
  tasks, timers, and pipelines;
- GPU regression scripts retain their current correctness and zero post-seal
  pipeline results.
