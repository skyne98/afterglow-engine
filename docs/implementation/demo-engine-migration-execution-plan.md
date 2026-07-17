# Demo-to-engine migration execution plan

**Status:** concrete execution plan; implementation not yet complete
**Date:** 2026-07-16
**Source audit:** [`demo-to-engine-feature-audit.md`](demo-to-engine-feature-audit.md)

## 1. Objective

Make every first-party visual demo a thin consumer of one canonical, bounded,
sealed engine runtime, then make architectural regression mechanically difficult.

This plan is complete only when:

- all first-party visual demos have `canonical` conformance status;
- the temporary legacy baseline is deleted;
- no demo owns engine lifecycle, frame scheduling, VT feedback orchestration,
  worker pools, BIG sessions, POM shader assembly, glTF VT replacement, or
  renderer-private profiling;
- every registered engine frame callback participates in allocation-effect
  analysis;
- static CI, browser integration tests, and the real-GPU release gate pass.

A green unit-test run alone is not completion.

## 2. Rules for executing the plan

Every change below follows these rules:

1. **Regression test first.** Add a test that fails for the observed demo-local
   behavior before moving the behavior.
2. **One owner.** The replacement primitive must own its storage, listeners,
   workers, GPU state, and disposal. Do not leave a second demo implementation.
3. **Mandatory capacities.** Public constructors accept explicit capacities;
   they do not silently grow or infer an unbounded maximum.
4. **No compatibility path.** Migrate one consumer and delete its old path in
   the same change. Temporary adapters may exist only inside the change, not on
   `master`.
5. **Mechanism only.** Scene coordinates, visual choices, camera choreography,
   model choice, and benchmark scenarios stay in demos.
6. **Docs in the same change.** Any public API or behavior change updates the
   relevant `docs/api/` page and `book/src/` chapter.
7. **Generated output in the same change.** Run `bun scripts/build-web.ts`; CI
   must pass `--check`.
8. **No unproved completion claims.** A migration item becomes complete only
   when its machine-readable conformance gate and listed tests pass.

## 3. Work tracking

Use the IDs below in commits and review notes. The order is a dependency order,
not a menu.

| Wave | IDs | Result |
|---|---|---|
| A | DME-001–004 | truthful status, debt freeze, enforceable contracts |
| B | DME-010–013 | canonical runtime and reference demo |
| C | DME-020–025 | VT, assets, glTF, POM become reusable engine APIs |
| D | DME-030–034 | all remaining demos migrate; legacy paths deleted |
| E | DME-040–044 | tooling, browser tests, soak/release enforcement |

No Wave C work starts by copying another demo helper. Wave B establishes the
ownership/lifecycle pattern first.

### Progress record

Implemented on 2026-07-17:

- DME-001–003: authoritative artifact/conformance manifests, architecture debt
  fingerprinting, protected-branch comparison support, and CI/xtask gates;
- DME-004 foundation: strict TypeScript and source-hygiene ratchets plus whole-
  function syntax and TypeScript-symbol call-effect checking for
  `@alloc-effect none`; legacy marker regions still need migration to JSDoc
  effects before whole-frame reachability is complete;
- DME-010–011: fixed diagnostics, owned renderer host, and sealed
  `EngineRuntime` with fixed worker/render-pass registrations;
- DME-012: `engine-demo.ts` is the first `canonical` visual entrypoint and uses
  the runtime, render adapter, renderer host, explicit capacities, and one Three
  module identity;
- DME-013: fixed-capacity `FrameBench` capture with explicit diagnostic finish;
- DME-020 mechanism: fixed-capacity `VirtualTextureFeedbackCoordinator`,
  exception-safe renderer state, and atomic multi-pass feedback publication are
  implemented and tested;
- DME-021 mechanism: `BigAssetSession` now owns bounded header admission,
  transcoder startup/rollback, raw assets, one VT store, and reverse shutdown;
- DME-022: `parseGLTFAsset` retains stable parser material indices;
  `VirtualGltfBinding` owns fixed-capacity replacement, factors/alpha/depth,
  imported-image release, UV channels/KHR texture transforms, sampler address
  modes, feedback swaps, and rollback. Unrepresentable nearest/asymmetric
  sampler state fails during bootstrap;
- DME-023: the cook moves material texture metadata into
  `AFTERGLOW_virtual_textures`, strips image buffer views from runtime GLBs,
  remaps remaining references, and compacts BIN bytes before `.big` packing;
- DME-025: fixed model collection, exact deformed bounds, pivot normalization,
  bounded animation actions, and disposable skeleton diagnostics are implemented;
  dungeon/rigged demo migrations remain DME-030–032.

The machine-readable source of truth remains `engine-conformance.json`; this
progress note does not override its gates.

## 4. Wave A — freeze the debt and make status truthful

### DME-001 — Reopen the sealed-runtime rollout

**Change**

- Correct the status of
  `docs/implementation/no-runtime-allocation-constant-time-budget-plan.md`:
  fixed primitives are implemented, but whole-demo conformance is not.
- Link the audit and this plan.
- Add a machine-readable file:
  `crates/afterglow-web/www/engine-conformance.json`.

Initial shape:

```json
{
  "version": 1,
  "releaseStatus": "migration",
  "visualEntrypoints": {
    "engine-demo.ts": "legacy",
    "lod-demo.ts": "legacy",
    "vt-demo.ts": "legacy",
    "dungeon.ts": "legacy",
    "rigged-vt-demo.ts": "legacy"
  }
}
```

`releaseStatus: "conformant"` is legal only when every entry is `canonical` and
no legacy baseline file exists.

**Tests/gates**

- `scripts/check-engine-conformance.ts` rejects unknown states, missing visual
  entrypoints, stale entries, and an impossible `conformant` claim.
- CI runs this script.

**Done when**

Repository status cannot claim full sealed-runtime conformance while a visual
entrypoint remains legacy.

### DME-002 — Make one artifact/entrypoint manifest authoritative

**Change**

Create `crates/afterglow-web/www/web-artifacts.json` with, for every generated
artifact:

- authored source;
- generated output;
- role: `runtime`, `worker`, `visual-demo`, `diagnostic`, or `test`;
- owning HTML page when applicable;
- whether it is production-shippable.

Change `scripts/build-web.ts` to read this file instead of its hard-coded
`targets` array. Change the conformance checker to derive visual entrypoints
from the same file.

The checker must reject:

- an authored entrypoint or HTML page missing from the manifest;
- a manifest source/output that does not exist;
- duplicate outputs;
- inline authored `<script>` blocks;
- local generated `.js` imports from TypeScript;
- an external authored script not represented by the artifact manifest.

**Tests/gates**

Add script fixtures under `scripts/tests/web-artifact-contract/` covering each
failure above. Run them with `bun test` in CI and `xtask test`.

**Done when**

Adding a new demo requires declaring its role and conformance state; it cannot
silently escape architecture checks.

### DME-003 — Add a ratcheting architecture lint

**Change**

Create `scripts/lint-demo-architecture.ts`. For visual entrypoints it reports
these rule IDs:

| Rule | Forbidden in a canonical demo |
|---|---|
| `AG-DEMO-001` | direct `requestAnimationFrame` or `setAnimationLoop` |
| `AG-DEMO-002` | construction of renderer/runtime/memory/budget/seal/worker pools |
| `AG-DEMO-003` | direct `VirtualTextureFeedbackPass` orchestration |
| `AG-DEMO-004` | `window.THREE`, `window.Afterglow*`, or engine globals |
| `AG-DEMO-005` | private Three backend fields such as `_renderContexts` |
| `AG-DEMO-006` | unbounded error/waiter arrays used by frame/test control |
| `AG-DEMO-007` | direct BIG prefix/header/session assembly |
| `AG-DEMO-008` | local glTF source-texture replacement/disposal tables |
| `AG-DEMO-009` | local POM WGSL/material graph assembly |
| `AG-DEMO-010` | frame callback without an allocation effect declaration |

During migration only, commit
`crates/afterglow-web/www/demo-architecture-baseline.json`, containing the exact
current `(file, rule, line fingerprint)` set. CI compares the candidate report
both to this file and, on pull requests, to the merge-base report from the
protected default branch. CI fails if:

- a violation is added, even if the candidate also edits its baseline;
- a violation moves without explicit baseline update;
- a violation count increases;
- a new entrypoint is marked `legacy`;
- a canonical entrypoint has any violation.

Protect `master`: require the conformance/test checks and disallow direct pushes
that bypass the merge-base ratchet. Emergency administrator bypasses must be
followed by the same release-gate evidence before packaging.

Removing violations is always accepted. Delete the baseline in DME-043.

Suppressions require all of:

```text
@architecture-allow AG-DEMO-NNN issue=DME-NNN expires=YYYY-MM-DD reason=...
```

The checker rejects expired, malformed, or issue-less suppressions. No
suppression is allowed when `releaseStatus` becomes `conformant`.

**Done when**

The existing debt is frozen exactly and can only decrease.

### DME-004 — Replace marker-only allocation lint with effect coverage

The current lint scans only manually marked regions under `engine/`; omitting a
marker is a loophole. Replace it incrementally with TypeScript AST/checker-based
analysis.

**Change**

- Add pinned `typescript` as a development dependency.
- Create `scripts/lint-allocation-effects.ts`.
- Use JSDoc effects on authored functions:

```ts
/** @alloc-effect none */
/** @alloc-effect pooled */
/** @alloc-effect budgeted AssetFetch */
/** @alloc-effect bootstrap */
/** @alloc-effect gameFacing */
/** @alloc-effect diagnostic */
```

- Build an authored call graph with the TypeScript checker.
- A `none` function may call only `none`/approved `pooled` functions.
- Unknown authored callees from `none` are errors.
- External calls require an allowlisted boundary classification.
- Any function passed to `EngineRuntime.start`, a registered frame stage, or a
  registered hot input/feedback callback must be `none`; this is inferred from
  the call site, not an optional marker.
- Keep the existing syntax bans, now applied to complete function ASTs rather
  than line regex regions.
- Keep `engine-allocation-effects.json` temporarily as the migration source;
  delete region markers and the old lint only after coverage reaches 100%.

**Tests/gates**

Compiler-fixture tests must cover local calls, imported calls, aliases, methods,
callbacks, unknown callees, boundary permits, stale metadata, and suppression
expiry.

**Done when**

Every engine-owned function reachable from the runtime frame loop has a checked
effect. CI reports zero unknown nodes in the reachable call graph.

## 5. Wave B — establish the canonical runtime

### DME-010 — Fixed diagnostics and lifecycle ownership

**New files**

- `www/engine/diagnostics.ts`
- `www/engine/renderer-host.ts`
- tests beside both files

**API**

```ts
interface EngineDiagnosticsCapacity {
  eventSlots: number;
}

class EngineDiagnostics {
  tryRecord(code: DiagnosticCode, source: DiagnosticSource, detail: unknown): DiagnosticStatus;
  readInto(index: number, out: DiagnosticRecord): boolean;
  clear(): void;
  readonly count: number;
  readonly dropped: number;
  readonly highWater: number;
}

class RendererHost {
  static create(options: RendererHostOptions): Promise<RendererHost>;
  resize(width: number, height: number, pixelRatio: number): void;
  warm(variants: readonly RendererWarmVariant[]): Promise<void>;
  seal(): void;
  dispose(): void;
}
```

Requirements:

- diagnostics storage is fixed at construction;
- overflow drops newest with a counter; it never grows;
- `RendererHost` exclusively owns canvas attachment, resize, device loss,
  uncaptured GPU errors, renderer seal, and listener cleanup;
- bootstrap rollback invokes `dispose()` exactly once;
- no dynamic HUD strings are part of diagnostics.

**Tests**

Capacity/overflow, listener removal, double disposal, failed initialization,
device loss, resize, warm-before-seal, and post-seal pipeline violation.

### DME-011 — `EngineRuntime`

**New file**

`www/engine/runtime.ts`

**API contract**

```ts
interface EngineRuntimeCapacities {
  memory: EngineMemoryConfig;
  diagnostics: EngineDiagnosticsCapacity;
  maxWorkerInputs: number;
  maxRenderPasses: number;
}

interface EngineFrameClient {
  /** @alloc-effect none */
  update(frame: Readonly<RenderFrame>): void;
}

class EngineRuntime {
  static create(options: EngineRuntimeOptions): Promise<EngineRuntime>;
  registerWorker(input: RenderWorkerInput): RegistrationStatus;
  registerRenderPass(pass: EngineRenderPass): RegistrationStatus;
  enterWarmup(): void;
  warm(): Promise<void>;
  sealGameplay(): void;
  start(client: EngineFrameClient): void;
  stop(): void;
  dispose(): void;
}
```

Ownership/order:

1. construct `EngineMemory`, `FrameBudget`, `RenderAdapter`, diagnostics, and
   renderer host;
2. initialize a complete `ResourceManifest` on the adapter world;
3. register fixed worker/render-pass slots during bootstrap;
4. enter warm-up and compile all declared variants;
5. seal resources, adapter, memory, renderer, then runtime;
6. on each rAF mutate one persistent `RenderFrame` record;
7. call `prepareAfterglowFrame` once;
8. call the game update once;
9. execute registered render passes in fixed order;
10. schedule the next frame only if still running.

The runtime owns the only rAF callback. `start()` before seal, duplicate start,
late registration, and late warm variants fail deterministically. Render passes
are registered bootstrap objects, not per-frame closures.

**Tests**

- exact lifecycle transition table;
- exact frame-stage ordering;
- stable frame object identity across 10,000 synthetic frames;
- one `beginFrame` and one render submission per frame;
- stop during update;
- disposal during device loss;
- worker/pass capacity overflow;
- no registration or pipeline creation after seal;
- allocation-effect reachability from the owned rAF callback.

**Documentation**

Add `docs/api/engine-runtime.md` and a matching mdBook chapter. Update
`docs/api/engine-memory.md` so direct manual orchestration is documented as a
low-level test API, not the normal application path.

### DME-012 — Make `engine-demo` the reference consumer

Rewrite `www/engine-demo.ts` to use only:

- `EngineRuntime`;
- one predeclared instanced render descriptor;
- `RenderAdapter` entities/transforms/dirty marking;
- fixed bootstrap scene resources;
- the diagnostic benchmark from DME-013.

Delete from the demo:

- local component arrays;
- local matrix composition;
- direct instance-buffer ownership;
- direct rAF;
- duplicate frame benchmark;
- direct renderer lifecycle/listeners.

Demo capacities are explicit and checked in:

```text
entities: 5,000
instanced descriptor capacity: 5,000
structural commands: explicit configured bound
workers: 0
VT requests: 0
```

Do not invent large fallback capacities; choose the remaining values from a
recorded startup/soak high-water run and record them in
`docs/api/runtime-capacities.md`.

**Tests/gates**

- static conformance changes `engine-demo.ts` from `legacy` to `canonical`;
- a browser integration test checks 5,000 attached instances, sealed phase,
  zero queue overflow, and zero post-seal pipelines;
- the CEF minimal example comment and user-facing book agree with 5,000.

### DME-013 — Bounded diagnostic benchmark

Rewrite `engine/bench.ts`:

- constructor allocates one fixed `Float64Array(maxFrames)`;
- `start(sampleCount)` rejects counts above capacity;
- `tick()` only writes numeric slots;
- result calculation occurs after capture under a diagnostic slow-path permit;
- results write into a caller-owned stable `BenchResults` record;
- formatting stays diagnostic and outside the frame callback.

Delete the duplicate implementation from `engine-demo.ts`. Add capacity,
restart, percentile, dropped-frame, and no-per-tick-allocation tests.

## 6. Wave C — migrate reusable rendering and asset mechanisms

### DME-020 — VT feedback coordinator

**New file**

`www/engine/virtual-texture-feedback-coordinator.ts`

**Required API**

```ts
interface FeedbackRenderable {
  readonly passCount: number;
  beginFeedbackPass(pass: number): void;
  endFeedbackPass(pass: number): void;
}

class VirtualTextureFeedbackCoordinator implements EngineRenderPass {
  constructor(capacities: {
    renderables: number;
    passes: number;
    feedbackEntries: number;
  });
  register(renderable: FeedbackRenderable): RegistrationStatus;
  resize(width: number, height: number): void;
  warm(...): Promise<void>;
  consumeInto(store: VirtualTextureStore): FeedbackStatus;
  render(...): void;
  dispose(): void;
}
```

It owns all feedback targets, cadence, consume/submit order, atomic multi-pass
merge, camera-layer preservation, render-target restoration, and temporary
shadow suppression. All state restoration uses `try/finally`.

**Tests**

- four-pass merge cannot cancel another pass's pages;
- missing/late/stale pass handling;
- renderable/pass/entry overflow;
- state restoration after a thrown pass;
- resize and disposal;
- no allocations in merge/consume hot functions;
- warm variants complete before renderer seal.

Update `docs/api/virtual-texturing.md` and the book in the same change.

### DME-021 — Owned BIG asset session

**New file**

`www/engine/big-asset-session.ts`

`BigAssetSession.open()` must accept explicit worker, request, byte-in-flight,
completion, and cache capacities. It owns:

- range source and parsed header;
- raw `AssetStore` loader;
- bounded transcoder clients/pool;
- selected device format;
- optional generic persistent blob cache;
- page provider;
- cancellation and idempotent shutdown.

Opening is bootstrap/loading async. Frame-time methods are synchronous bounded
poll/drain operations. Failure at every bootstrap step unwinds already-created
workers and resources.

**Tests**

- prefix/header/range failures at each stage;
- partial worker startup rollback;
- request/byte/worker capacity overflow;
- cache unavailable/failure fallback;
- stale completion/cancellation;
- double close;
- no pending workers or requests after failure.

Update `docs/api/asset-system.md`, `docs/api/runtime-capacities.md`, and the book.

### DME-022 — Stable-index glTF VT binding

**New file**

`www/engine/virtual-gltf-binding.ts`

**API**

```ts
class VirtualGltfBinding implements FeedbackRenderable {
  static create(asset: GltfAsset, store: VirtualTextureStore,
                options: VirtualGltfBindingOptions): Promise<VirtualGltfBinding>;
  warmVariantsInto(out: RendererWarmVariantSink): void;
  dispose(): void;
}
```

Requirements:

- bind only by stable glTF material index;
- concrete typed primitive/material records; no `any`;
- preserve scalar factors, alpha mode/cutoff, side, depth behavior, normal scale,
  transforms, skinning, and morph configuration;
- own source-material/source-texture disposal exactly once;
- represent materials without a virtual base color explicitly;
- implement feedback variants without demo material tables.

**Tests**

Duplicate/empty names, shared textures, all channel combinations, alpha modes,
normal scale, transforms, multi-material meshes, skinned meshes, morph targets,
disposal, and capacity failures.

### DME-023 — Strip VT-replaced images from runtime GLBs

Change the offline pipeline/container manifest so runtime glTF parsing does not
decode image payloads that the VT binding replaces.

**Required work**

- pipeline emits stable material-index→image-role records;
- runtime GLB omits replaced image payload references;
- loader exposes the typed role manifest to DME-022;
- container version changes semantically; delete the old runtime path and
  rebuild bundled assets.

**Tests/gates**

- pipeline round-trip fixture;
- duplicate material names;
- mixed virtual/resident material;
- Dragon asset reports zero browser-decoded VT source images;
- visual GPU regression remains identical;
- bootstrap peak heap is recorded before/after.

Update pipeline/asset/VT API docs and book chapters.

### DME-024 — Engine-owned POM material adapter

**New file**

`www/engine/virtual-pom-material.ts`

Implement `createVirtualPomMaterialPair()` with fixed base/POM visible and
feedback variants. Move all POM graph/TBN/VT fallback wiring out of dungeon.

Correct self-shadowing by applying visibility to the current light contribution,
never to accumulated `reflectedLight` fields.

**Tests**

- generated WGSL contract for all four variants;
- two-light oracle where shadowing light B cannot change light A;
- tangent missing/invalid failure during bootstrap;
- bounded layers/steps/offset options;
- linked PBR fallback uses one resolved level;
- no material/pipeline creation after seal.

Update `docs/api/pom.md`, `docs/api/virtual-texturing.md`, and their book pages.

### DME-025 — Model utility primitives

Add focused utilities, not a character subsystem:

- `collectModelPrimitives()`;
- `computeDeformedBoundsInto()` with caller-owned scratch;
- `normalizeModelPivot()`;
- `AnimationSet` with fixed clip slots and explicit active state;
- disposable skeleton debug adapter.

Tests cover static/skinned/morph bounds, empty models, hidden animation updates,
normalization policies, clip capacity, and disposal. Presentation height, active
clip selection, shadows, and floor remain demo policy.

## 7. Wave D — migrate each remaining demo and delete legacy code

### DME-030 — Dungeon migration

Replace local infrastructure with:

- `EngineRuntime`;
- `RendererHost` diagnostics/profiling;
- `BigAssetSession`;
- `VirtualTextureFeedbackCoordinator`;
- `createVirtualPomMaterialPair()`;
- bounded action input from DME-034.

Delete local worker pool/header parsing, duplicate feedback scene mechanics,
shader-module monkey patch, private timestamp-pool access, POM graph, rAF,
waiter array, error array, and renderer lifecycle.

Keep wall layout, camera policy, movement, collision fixture, lighting, and test
scenarios local.

**Gate**

`dungeon.ts` becomes `canonical`; existing dungeon GPU validation plus a
multi-light regression passes with zero post-seal pipelines and zero overflows.

### DME-031 — Rigged VT migration

Replace local infrastructure with runtime/session/coordinator/glTF binding/model
utilities. Delete material-name lookup, manual texture disposal tables, material
swap records, feedback pass arrays, worker setup, bounds duplication, waiters,
errors, direct rAF, and renderer state toggles.

Only the active model's mixer updates. Model selection, camera presentation,
lighting, floor, clip selection, and credits remain local.

**Gate**

`rigged-vt-demo.ts` becomes `canonical`; both assets load, all expected textures
become resident, shadows remain active, source VT image decode count is zero,
and the existing GPU regression reports no pipeline/queue errors.

### DME-032 — VT demo migration

Replace fabricated CPU feedback with the real coordinator and bounded
procedural page source. Move procedural generators and raw GPU fixtures from
`engine/` to `examples/support/` or `tests/support/`.

Delete per-frame `Map`, request objects, and dynamic page keys.

**Gate**

`vt-demo.ts` becomes `canonical`; all existing east/west/rotation/upload GPU
checks pass and the hot allocation call graph is clean.

### DME-033 — Real static-mesh LOD

Delete the browser-generated GLB/custom legacy model path. Add:

- offline static-mesh LOD records;
- fixed `LodSet` descriptors;
- screen-error/coverage selection with hysteresis;
- fixed-capacity residency/selection state;
- explicit exclusion of skinned simplification.

The demo must show one model switching levels based on camera coverage, not four
manual side-by-side meshes. Move sphere fixtures offline/tests. Remove the
undefined `texHandle` path rather than patching it.

**Gate**

`lod-demo.ts` becomes `canonical`; deterministic camera trajectories verify
level transitions and hysteresis with no allocation or pipeline creation.

### DME-034 — Bounded input and dev harness

Add:

- fixed action/button/axis storage with bootstrap bindings and blur reset;
- composable orbit and first-person controllers;
- dev-only `BrowserHarness` with fixed scenario and waiter capacities;
- rate-limited diagnostic HUD under a slow-path permit.

The harness exports one typed CDP namespace and is absent from production
bundles. Programmatic step/idle/scenario behavior uses fixed slots and explicit
timeouts. Delete each demo's custom `window.__afterglow*`, waiter arrays, key
sets, HUD formatter, and input listener ownership.

Update relative-pointer/input docs and book.

## 8. Wave E — tooling and permanent prevention

### DME-040 — Remove global bundle bridge

- Add typed `www/engine/index.ts` core exports plus tree-shakeable
  `model-api.ts` and `virtual-texturing-api.ts` subsystem barrels.
- Bundle each entrypoint with exactly one Three module graph.
- Delete `engine-bundle-input.ts`, `engine-bundle.js`, all `window.Afterglow*`
  writes, and redundant HTML script tags.
- Add a build metafile check that each page contains one Three package identity.
- Add import fences: engine cannot import demos/support; production demos cannot
  import tests/support or engine internals.

### DME-041 — Clean examples/tooling

- Replace every CEF redirect script with `AppBuilder::root()`.
- Change worker test/benchmark to generated typed clients; label raw transport
  cases explicitly.
- Add bounded `DevAssetServer`/`xtask serve`; remove the unbounded
  thread-per-connection example loop.
- Give every example idempotent shutdown and bootstrap rollback tests.

### DME-042 — One local/CI command

Add these `xtask` commands:

```text
cargo run -p xtask conformance
cargo run -p xtask test
cargo run -p xtask release-gate
```

`conformance` runs:

1. artifact/entrypoint manifest check;
2. demo architecture lint;
3. allocation-effect analysis;
4. import-boundary and single-Three checks;
5. generated artifact drift;
6. docs/API inventory consistency.

`test` additionally runs Rust and Bun tests.

`release-gate` additionally requires:

- all visual entries `canonical`;
- no legacy baseline/suppressions;
- mdBook build;
- browser integration result JSON for all demos;
- current real-GPU result JSON matching hardware, driver, artifact hash, and
  required thresholds;
- required soak evidence.

CI runs `conformance` and `test` on every push/PR. Release packaging refuses to
run unless `release-gate` passes.

### DME-043 — Delete the migration escape hatch

When all demos are canonical:

- delete `demo-architecture-baseline.json`;
- reject `legacy` as a schema value;
- reject all architecture suppressions in visual demos;
- set `releaseStatus` to `conformant`;
- update the canonical implementation plan only after `release-gate` produces
  passing evidence.

This is the point at which the ratchet becomes a permanent zero-tolerance gate.

### DME-044 — Generate new demos from the canonical template

Add:

```text
cargo run -p xtask new-demo <name>
```

It creates:

- authored `.ts` importing `EngineRuntime`;
- markup/style-only `.html` with external generated script;
- explicit capacity block;
- lifecycle/disposal skeleton;
- allocation-effect-declared update callback;
- unit/browser test skeleton;
- artifact and conformance manifest entries.

The command refuses duplicate names. CI rejects manually added unregistered
demos, so the generated shape is the easiest and safest path.

## 9. Required test layers

### Per-commit tests

- focused unit/regression tests for the moved primitive;
- architecture and allocation-effect checks;
- generated artifact check;
- Rust workspace tests when Rust/pipeline code changes;
- API docs and book build when behavior changes.

### Browser integration tests

Every visual demo exposes through the dev-only harness:

- engine phase;
- resource/renderer seal state;
- pipeline violation count;
- all pool/queue overflow and high-water counters;
- pending worker/request counts;
- deterministic scenario start/finish;
- bounded error records.

Tests fail on nonzero overflow unless that scenario explicitly asserts the
corresponding deterministic degradation status.

### Real-GPU tests

Enhance `test-vt-gpu.sh`, `test-dungeon-gpu.sh`, and
`test-rigged-vt-gpu.sh` to write versioned JSON containing:

- git commit and built artifact hashes;
- GPU adapter/vendor/architecture;
- browser/CEF/Three versions;
- scenario and duration;
- frame percentiles and misses;
- main/feedback GPU timings;
- heap floor/range;
- queue high-water/overflow;
- pending tasks at end;
- pipeline violations and GPU errors.

A validator script checks the schema and thresholds. Human-readable console
output is not release evidence.

### Soak tests

Before declaring the migration complete, run:

1. 30-minute stable dungeon;
2. 30-minute traversal dungeon;
3. 30-minute dungeon teleport/thrash;
4. 30-minute rigged model switching/animation/camera movement;
5. 30-minute VT pan/teleport/full-cache churn;
6. forced worker failure/restart;
7. full-capacity deterministic degradation.

Each run must end with bounded heap floor, zero unexplained pending work, zero
unbounded queue growth, zero post-seal pipelines, and no GPU errors.

## 10. Pull-request completion checklist

A DME item is mergeable only if all applicable answers are yes:

- [ ] Was the failing regression test added first?
- [ ] Is there exactly one owner after the change?
- [ ] Was the demo-local implementation deleted?
- [ ] Are capacities explicit, bounded, and visible in telemetry?
- [ ] Is overflow typed and deterministic?
- [ ] Are bootstrap failure and double-disposal tested?
- [ ] Are hot functions allocation-effect checked transitively?
- [ ] Are all async stages cancelable/stale-aware and bounded by count/bytes/time?
- [ ] Were `docs/api/` and the mdBook updated?
- [ ] Were generated JS artifacts rebuilt and checked?
- [ ] Did `xtask conformance` and `xtask test` pass?
- [ ] If rendering changed, is current GPU result JSON attached?
- [ ] Did the conformance ratchet decrease or remain at zero?

The checklist aids review; CI independently enforces the mechanically checkable
items.

## 11. Final definition of done

Run:

```sh
nix-shell shell.nix --run "cargo run -p xtask release-gate"
```

The command must prove all of the following:

```text
visual demos canonical:               5/5
legacy baseline files:                0
architecture suppressions:            0
unknown allocation-effect callees:    0
post-seal pipeline violations:        0
unexpected pool/queue overflows:      0
pending work after idle:              0
browser integration suites:           pass
real-GPU suites:                       pass
required soak suites:                  pass
generated artifact drift:              none
API/book build:                         pass
```

Only then may the canonical runtime plan return to a fully complete status.
