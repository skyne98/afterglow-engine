# Engine Memory & Frame Discipline

Afterglow is migrating engine-authored gameplay hot paths to a sealed runtime
model:

- backing memory is reserved during bootstrap/warm-up;
- gameplay uses fixed arenas, pools, rings, and numeric handles;
- capacity exhaustion is explicit and never silently grows storage;
- potentially stalling work is bounded and deferred;
- there is no loading-screen phase: post-warm-up assets/world data stream
  continuously through fixed budgets and atomic publication;
- game code may allocate, but engine hot paths may not.

`EngineMemory` provides fixed frame/render scratch arenas, a typed structural
command ring, fixed index pools for worker completions, asset requests, and VT
requests, plus caller-owned telemetry trace records and metric cells. Structural pushes return `CapacityExceeded`; bounded drains leave
the remaining suffix queued. Call `warmup()` and `sealGameplay()` after initializing all required
resources. Declare them in a `ResourceManifest`; `initializeAndSeal(world)`
eagerly runs factories, verifies injected resources, and prevents later lazy
ECS-resource creation.

```ts
const memory = new EngineMemory({
  frameScratchBytes: 256 * 1024,
  renderScratchBytes: 256 * 1024,
  structuralCommands: 4096,
  workerCompletions: 1024,
  assetRequests: 128,
  vtRequests: 4096,
  telemetryRecords: 16_384,
  telemetryMetricCells: 512,
});

memory.warmup();
// Warm renderer/material/resource variants.
memory.sealGameplay();
sealResources(world);
```

All visual entrypoints now satisfy the canonical architecture gate with no
architecture baseline. Current VT work has removed the array LRU, indexed `.big`
pages once, bounded pending page work, and consolidated asset-worker polling.
`AssetStore` uses numeric state tables and a fixed completion ring whose
publication count is capped per poll.

Engine browser source is authored under `afterglow-web/web/src/`. `www/` is a
fully generated, disposable deployment tree and contains no authored or vendored
source:

```sh
bun install --cwd crates/afterglow-web/web --frozen-lockfile
bun scripts/build-web.ts
bun scripts/build-web.ts --check
cargo run -p xtask conformance
cargo run -p xtask test
cargo run -p xtask release-gate
```

The conformance command runs several independent gates:

- the authoritative browser artifact/page inventory;
- visual-demo architecture checks and canonical/legacy status validation;
- the convergence deletion ledger, which prevents removed paths from returning;
- hot allocation-effect checks;
- generated JavaScript drift checks.

The architecture checker rejects demo-owned frame loops, renderer/runtime
construction, generated worker-client/RPC assembly, BIG/VT/POM/glTF infrastructure, engine globals, private
Three access, unbounded control collections, direct diagnostic UI, and untyped
frame callbacks. The repository is currently `converging`: canonical source
architecture does not claim completed visual/runtime conformance. Conformant
releases cannot carry a legacy state, architecture baseline, or bridge. Generic
compiler/style debt is not release-gated; unit and vertical tests establish
implementation behavior. `release-gate` additionally builds the book and
requires evidence schema v2: web/native screenshots and `GameReady` for every
visual demo, coherent dimensions, semantic/reference pixel checks, bounded
resource/queue results, and five 30-minute plateaued soak scenarios per target.

The allocation lint covers explicitly marked hot regions and cross-checks them
against `engine-allocation-effects.json`. Budgeted-boundary calls require an
inline reason permit; stale or missing effect entries fail CI. Coverage expands
as systems migrate. Browser, Three.js, and game allocations require separate
profiling; the policy specifically controls engine-authored code.
