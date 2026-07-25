# Runtime capacities and degradation

This is the final cross-system capacity reference for the sealed runtime.
Project-specific ECS/pool sizes remain constructor configuration; defaults and
hard transport bounds are listed below.

| System | Capacity / budget | Exhaustion behavior |
|---|---:|---|
| Async web RPC task slots | 256 | Call rejects before dispatch |
| Wasm worker completion queue | 256 outstanding | Export returns `-2`; no queue growth |
| JS completion drain | 32/poll | Remaining completions stay in worker queue |
| Browser fetch slots | 256 | Fetch registration returns `0` |
| Generic persistent cache | caller-configured; dungeon = 1 GiB / 65,536 entries / 64 writes | LRU evicts to 75% low water; one async two-generation compaction; oversized values/failures return `false` |
| Persistent cache index | `2 × maxEntries` fixed slots | Open rejects invalid configuration; no growth |
| AssetStore IDs | 1,024 default | `AssetAdmission.CapacityExceeded` / path wrapper throws |
| AssetStore publication | ring = asset capacity; 32/poll | Deferred suffix remains queued; counters record high-water/overflow |
| Native open asset sources | 16 | Round-robin descriptor replacement |
| VT scheduler | physical atlas slot count | Typed counters report overflow; visible generations persist |
| VT in-flight pages | 64 and 8 MiB | Admission rejected and retried from scheduler |
| VT ready uploads | 64 | Fixed completion ownership |
| VT transcode workers / waiting ring | 2–4 independent SPSC workers / 64 shared jobs | Promise boundary rejects capacity; each worker remains one-in-flight |
| VT upload commit | adaptive 1–4 pages and 0.10–0.35 ms/poll; starts 2 / 0.20 ms | Ready suffix deferred; overload resets promoted settings |
| VT scheduling | 8 admissions, 0.25 ms/poll, 22 priority lanes | Highest nonempty exact-rung/center lane resumes next frame |
| VT stale horizon | 2 feedback epochs | Read/transcode canceled or stale output discarded |
| Feedback readback | 1 outstanding | New submit returns `false` until consume |
| Structural renderer slice | 256/frame | Fixed ring suffix retained |
| Dirty root slice | 4,096/frame | Dirty flags remain on ring suffix |
| Hierarchy rebuild | 512 ops and 0.2 ms/frame | Double-buffered old order remains published |
| Hierarchy child sync | 4,096/frame | Rotating cursor continues next frame |
| Continuous unique proxies | 512/frame | Dense-list cursor continues next frame |
| Instanced proxies | `shardCapacity × maxShards` | `RenderAttachStatus.CapacityExceeded` |
| Unique proxies | descriptor `poolCapacity` | `RenderAttachStatus.CapacityExceeded` |
| Frame stage deadlines | 15/35/45/55/95% cumulative | Optional stages return typed deferral; required stages record overruns |
| Unified trace capture | caller-configured 40-byte records per producer | Existing prefix retained; new records dropped with exact counter |
| Unified metrics | caller-configured scalar cells; log2 histograms use 32 cells | Configuration rejects insufficient backing; updates never grow |

The current Radeon 680M adapter admits a 60×60 physical page grid: 3,600
136×136 slots in an 8160² atlas. Other adapters derive this value from
`maxTextureDimension2D`.

## Sealing sequence

1. Construct `EngineMemory` and a complete `ResourceManifest`.
2. Register asset paths and renderer descriptors.
3. Warm AssetStore/VT resources, descriptor shards/object pools, scene/camera
   variants, and every render-target format.
4. Install `RendererSeal`, perform warm renders, then seal it.
5. Call `RenderAdapter.sealGameplay()`, `EngineMemory.sealGameplay()`, and
   `ResourceManifest.seal()`.
6. Enter `prepareAfterglowFrame`; unsealed memory/adapter use is an error.

## Release evidence

- Cold/half/full/churn: `docs/benchmarks/vt-atlas-baseline-2026-07-16.md`.
- Corrected 10/30/60-minute soaks: `docs/benchmarks/vt-soak-2026-07-16.md`.
- Allocation boundaries: `docs/api/allocation-boundaries.md`.
- Machine policy: `crates/afterglow-web/web/contracts/engine-allocation-effects.json`.

No engine-owned queue in the migrated runtime grows during gameplay. String
Maps are confined to manifest/load/unload and game-facing lookup; frame VT
identity is numeric. Browser/game-facing allocations remain explicit bounded or
diagnostic boundaries.
