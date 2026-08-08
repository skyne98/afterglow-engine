# Engine memory and sealed runtime policy

`crates/afterglow-web/web/src/engine/core/engine-memory.ts` provides the first allocation-
discipline primitives for the engine migration described in
`docs/implementation/no-runtime-allocation-constant-time-budget-plan.md`.

## API

```ts
enum EnginePhase {
  Bootstrap, Warmup, GameplaySealed, Shutdown,
}

class LinearArena {
  readonly buffer: ArrayBuffer;
  readonly capacity: number;
  used: number;
  highWater: number;
  overflows: number;
  allocate(size: number, alignment?: number): number; // offset or -1
  reset(): void;
}

class FixedIndexPool {
  readonly capacity: number;
  used: number;
  highWater: number;
  overflows: number;
  acquire(): number;              // index or 0xffffffff
  release(index: number): boolean;
}

class FixedStructuralCommandRing {
  tryPush(kind, entity, argument0?, argument1?): RingPushStatus;
  drain(maxOperations, sink: StructuralCommandSink): number;
  readonly count: number;
  readonly highWater: number;
  readonly overflows: number;
}

class EngineMemory {
  readonly frame: LinearArena;
  readonly render: LinearArena;
  readonly structuralCommands: FixedStructuralCommandRing;
  readonly workerCompletions: FixedIndexPool;
  readonly assetRequests: FixedIndexPool;
  readonly vtRequests: FixedIndexPool;
  readonly telemetryTrace: ArrayBuffer;
  readonly telemetryMetrics: Float64Array;
  warmup(): void;
  sealGameplay(): void;
  beginFrame(): void;
  refreshMetrics(): EngineMemoryMetrics;
}

function defineEngineMemoryResource(config: EngineMemoryConfig): Resource<EngineMemory>;
```

`EngineMemoryConfig` also requires explicit `telemetryRecords` and
`telemetryMetricCells` capacities. Each trace record reserves exactly 40 bytes;
the metric capacity is a count of `Float64` cells. `EngineRuntime` gives these
caller-owned stores to its `EngineTelemetry` resource.

Constructors allocate backing buffers during bootstrap. The engine has no
loading-screen phase: after warm-up, continuous asset/world streaming must use
fixed pools/arenas/rings, bounded budgets and atomic publication without
re-enabling general worker allocation. Arena/pool operations and structural-ring
push/drain only mutate preallocated storage. Ring admission
returns `RingPushStatus.CapacityExceeded`; bounded drain preserves the queued
suffix for a later frame. Storage never grows implicitly.

`resource.ts` exports `ResourceManifest`, `sealResources(world)`, and
`resourcesAreSealed(world)`. A manifest eagerly initializes every declared
factory, verifies injected resources are present, and only then seals. Duplicate
or missing declarations fail bootstrap. After sealing, `Resource.get()` rejects
lazy construction.

`prepareAfterglowFrame(frame, workers, adapter, vt, memory, budget)` accepts an
optional `EngineMemory` and calls `memory.beginFrame()` before polling workers or running
render stages. Passing the memory resource through the frame orchestrator is the
supported way to rewind frame scratch exactly once per frame.

## Enforcement

Authored browser source is TypeScript. Run:

```sh
bun install --cwd crates/afterglow-web/web --frozen-lockfile
bun scripts/build-web.ts
bun scripts/build-web.ts --check
bun scripts/lint-hot-allocations.ts
```

`build-web.ts` owns the artifact manifest in `web/contracts/`, bundles authored
TypeScript from `web/src/`, and reconstructs the disposable `www/` deployment
from `web/public/` and `web/assets/`. Authored TypeScript imports authored
modules through `.ts` specifiers; importing generated `.js` artifacts is rejected.
`--check` stages the complete deployment in a temporary directory and detects
missing, stale, or extra files.

The allocation lint scans explicit `@hot-no-alloc-begin/end` regions and rejects
common allocation constructs. `engine-allocation-effects.json` must classify
every marked region as `none`; it separately names budgeted boundaries and
bootstrap-only APIs. Calls from a `none` region into a budgeted boundary require
an inline `@alloc-allowed reason=...` permit, and stale/missing manifest entries
fail CI. Coverage is intentionally incremental: only annotated regions are
currently enforced. It is a hygiene guard, not proof that V8, Three.js, or
browser APIs allocate nothing.

Additional repository contracts prevent uninspected demo/runtime code from
bypassing that partial coverage:

- `web-artifacts.json` is the only browser artifact/page inventory. The build
  rejects missing/stale entries, unknown JavaScript, inline authored scripts,
  unsafe paths, duplicate outputs, and false conformance claims.
- `engine-conformance.json` records every visual entrypoint as `legacy` or
  `canonical` independently from release status. The repository is currently
  `converging`; only a completed release may use `conformant`, which cannot
  contain legacy entries or a debt baseline.
- `convergence-deletions.json` records pending and removed convergence debt;
  `check-convergence-deletions.ts` fails if removed paths return or an absent
  pending item is not ratcheted to `removed`.
- `lint-demo-architecture.ts` checks direct frame-loop ownership, lifecycle
  construction, VT feedback/BIG/POM/glTF replacement code, engine globals,
  private Three access, unbounded control collections, untyped callbacks, raw
  listeners, direct HUD writes, and implementation-module imports. Its exact
  legacy findings are frozen; candidate changes may only delete findings.
- `lint-hot-allocations.ts` checks both legacy marked regions and complete
  JSDoc `@alloc-effect none` functions across engine and demo sources. For the
  latter it resolves authored callees through the TypeScript checker and rejects
  calls into diagnostic, budgeted, bootstrap, game-facing, or unknown effects
  unless the call line has a reason/issue/expiry permit.

Run the complete local contract:

```sh
cargo run -p xtask conformance
```

`.github/workflows/engine-contract.yml` runs inventory, focused architecture and
allocation contracts, generated-artifact validation, and the complete Rust and
browser test suites on pushes and pull requests. Generic TypeScript diagnostic
and source-style baselines are intentionally not release gates; observable
behavior is established by unit, vertical-integration, browser, GPU, and soak
tests.

## Current migration status

Implemented foundations:

- fixed arenas, index pools, and a typed structural-command ring;
- double-buffered hierarchy traversal capped at 512 operations / 0.2 ms per frame;
- gameplay resource sealing;
- hot-region allocation lint;
- centralized TypeScript artifact build/check;
- O(1) VT resident lookup/touch and fixed clock cache;
- O(1) pre-indexed `.big` VT page lookup;
- one page-thread async-worker pump instead of one timer per RPC;
- persistent fixed-capacity VT request scheduling with stale replacement;
- ready-time VT slot acquisition (slow reads do not evict useful pages);
- explicit 16-page and 2 MiB VT in-flight admission caps;
- fixed numeric AssetStore state tables plus a preallocated completion ring;
  Promise callbacks enqueue only, and `poll()` publishes a bounded prefix;
- stable allocation-free VT statistics for per-frame HUD/telemetry reads.

Remaining engine systems have not all migrated to `EngineMemory`. The sealed
runtime guarantee is not complete until the migration plan's release gates pass.
