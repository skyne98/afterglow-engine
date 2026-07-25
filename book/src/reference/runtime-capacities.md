# Runtime Capacities

The sealed defaults are intentionally finite:

| Work | Default limit |
|---|---:|
| Web worker calls / fetches / wasm completions | 256 each |
| Worker completion publication | 32 per poll |
| Asset IDs | 1,024 |
| Native open sources | 16 |
| VT in flight | 16 pages / 2 MiB |
| VT transcode waiting | 12 jobs behind 2–4 active workers |
| VT bulk deadlines | 1 ms urgent / 16 ms exact-quality |
| VT feedback | 55 ms cadence / one readback |
| VT uploads | adaptive, at most 4 pages / 0.35 ms |
| VT admissions | at most 8 per poll / 0.25 ms inside the 16-page cap |
| Structural changes | 256 per frame |
| Dirty roots / hierarchy children | 4,096 per frame each |
| Hierarchy rebuild | 512 operations / 0.2 ms |
| Continuous unique proxies | 512 per frame |
| Unified trace records | caller-configured, 40 bytes each |
| Unified metric cells | caller-configured; histograms use 32 cells |

Descriptor capacity is explicit: instanced population is
`shardCapacity × maxShards`; unique population is `poolCapacity`. Exhaustion
returns a typed status or game-facing Promise rejection and never grows engine
storage.

Warm resources, descriptors, scene/camera variants, and render targets before
sealing. Frame orchestration rejects unsealed `EngineMemory`; `RenderAdapter`
rejects preparation before descriptor warm-up and seal. See
`docs/api/runtime-capacities.md` for the canonical table and degradation rules.
