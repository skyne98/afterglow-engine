# Runtime Capacities

The sealed defaults are intentionally finite:

| Work | Default limit |
|---|---:|
| Web worker calls / fetches / wasm completions | 256 each |
| Worker completion publication | 32 per poll |
| Asset IDs | 1,024 |
| Native open sources | 16 |
| VT in flight | 64 pages / 8 MiB |
| VT uploads | 4 pages / 0.35 ms |
| VT admissions | 8 / 0.25 ms |
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
