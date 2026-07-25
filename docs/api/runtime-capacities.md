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
| AssetStore IDs | 1,024 default | `AssetAdmission.CapacityExceeded` / path wrapper throws |
| AssetStore publication | ring = asset capacity; 32/poll | Deferred suffix remains queued; counters record high-water/overflow |
| Native open asset sources | 16 | Round-robin descriptor replacement |
| VT scheduler | physical atlas slot count | Typed counters report overflow; visible generations persist |
| VT in-flight pages | 16 and 2 MiB | Admission defers in the fixed scheduler; pinned bootstrap overflow is retained there |
| VT ready uploads | 16 | Fixed completion ownership |
| VT transcode workers / waiting ring | 2–4 independent SPSC workers / 12 waiting jobs | Admission cap prevents valid queue overflow; each worker remains one-in-flight |
| VT bulk deadlines | 1 ms urgent / 16 ms exact-quality, non-resettable | Ready lane dispatches within transport byte/span bounds |
| VT upload commit | adaptive 1–4 pages and 0.10–0.35 ms/poll; starts 2 / 0.20 ms | Ready suffix deferred; overload resets promoted settings |
| VT scheduling | up to 8 admissions/poll inside the 16-page total cap; 0.25 ms/poll; 132 priority lanes | Highest nonempty tier/channel/quality/center lane resumes next frame |
| VT stale horizon | 2 feedback epochs (~110 ms plus frame/readback quantization) | Read/transcode canceled or stale output discarded |
| Feedback cadence/readback | 55 ms monotonic / 1 outstanding | No catch-up burst; submit defers until prior readback is consumed |
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
