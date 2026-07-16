# Engine memory and sealed runtime policy

`crates/afterglow-web/www/engine/engine-memory.ts` provides the first allocation-
discipline primitives for the engine migration described in
`docs/implementation/no-runtime-allocation-constant-time-budget-plan.md`.

## API

```ts
enum EnginePhase {
  Bootstrap, Warmup, GameplaySealed, LoadingScreen, Shutdown,
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
  warmup(): void;
  sealGameplay(): void;
  beginFrame(): void;
  refreshMetrics(): EngineMemoryMetrics;
}

function defineEngineMemoryResource(config: EngineMemoryConfig): Resource<EngineMemory>;
```

Constructors allocate backing buffers during bootstrap. Arena/pool operations
and structural-ring push/drain only mutate preallocated storage. Ring admission
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
bun install --cwd crates/afterglow-web/www --frozen-lockfile
bun scripts/build-web.ts
bun scripts/build-web.ts --check
bun scripts/lint-hot-allocations.ts
```

`build-web.ts` owns the TypeScript-to-JavaScript artifact manifest and rejects
unclassified hand-authored JavaScript. Authored TypeScript imports authored
modules through `.ts` specifiers; importing generated `.js` artifacts is rejected.
`--check` rebuilds into a temporary directory and detects artifact drift.

The allocation lint scans explicit `@hot-no-alloc-begin/end` regions and rejects
common allocation constructs. `engine-allocation-effects.json` must classify
every marked region as `none`; it separately names budgeted boundaries and
bootstrap-only APIs. Calls from a `none` region into a budgeted boundary require
an inline `@alloc-allowed reason=...` permit, and stale/missing manifest entries
fail CI. `.github/workflows/engine-contract.yml` runs the
lint, artifact drift check, fixed-storage browser tests, and Rust tracked-
allocator regressions on pushes and pull requests. Coverage is intentionally
incremental: only
annotated regions are currently enforced. It is a hygiene guard, not proof that
V8, Three.js, or browser APIs allocate nothing.

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
- 64-page and 8 MiB VT in-flight admission caps;
- fixed numeric AssetStore state tables plus a preallocated completion ring;
  Promise callbacks enqueue only, and `poll()` publishes a bounded prefix;
- stable allocation-free VT statistics for per-frame HUD/telemetry reads.

Remaining engine systems have not all migrated to `EngineMemory`. The sealed
runtime guarantee is not complete until the migration plan's release gates pass.
