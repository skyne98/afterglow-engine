# Engine Memory & Frame Discipline

Afterglow is migrating engine-authored gameplay hot paths to a sealed runtime
model:

- backing memory is reserved during bootstrap/warm-up;
- gameplay uses fixed arenas, pools, rings, and numeric handles;
- capacity exhaustion is explicit and never silently grows storage;
- potentially stalling work is bounded and deferred;
- game code may allocate, but engine hot paths may not.

`EngineMemory` provides fixed frame/render scratch arenas, a typed structural
command ring, and fixed index pools for worker completions, asset requests, and
VT requests. Structural pushes return `CapacityExceeded`; bounded drains leave
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
});

memory.warmup();
// Warm renderer/material/resource variants.
memory.sealGameplay();
sealResources(world);
```

This is an active migration, not yet a whole-engine guarantee. Current VT work
has removed the array LRU, indexed `.big` pages once, bounded pending page work,
and consolidated asset-worker polling. `AssetStore` uses numeric state tables
and a fixed completion ring whose publication count is capped per poll.

Engine browser source is authored in TypeScript. JavaScript in `www/` is generated
or vendored:

```sh
bun install --cwd crates/afterglow-web/www --frozen-lockfile
bun scripts/build-web.ts
bun scripts/build-web.ts --check
bun scripts/lint-hot-allocations.ts
```

The allocation lint covers explicitly marked hot regions and cross-checks them
against `engine-allocation-effects.json`. Budgeted-boundary calls require an
inline reason permit; stale or missing effect entries fail CI. Coverage expands
as systems migrate. Browser, Three.js, and game allocations require separate profiling;
the policy specifically controls engine-authored code.
