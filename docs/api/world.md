# World API

## Plugin

| Item | Purpose |
|---|---|
| `AfterglowWorldPlugin` | Registers generic cell loading and chunk lifecycle resources/systems. It does not install demo content. |

## Cell Loading

| Item | Fields | Purpose |
|---|---|---|
| `CellManifest` | `chunk`, `entities` | Authored cell baseline keyed by `ChunkId`. |
| `CellEntityTemplate` | `stable_id`, `name`, `persistent`, `transform`, `kind` | One stable-ID entity to spawn when a cell baseline loads. |
| `CellEntityKind` | `Empty`, `RotatingCube`, `PointLight`, `Camera3d` | Initial built-in template kinds. Game-specific state should live in registered components/resources. |
| `CellManifestRegistry` | `manifests` | Validated chunk-to-manifest registry. |
| `CellLoadRequests` | `pending` | Pending chunk loads. Requests remain pending while lifecycle moves through `Unloaded -> Loading -> Spawned`. |
| `CellLoadTracker` | `baseline_spawned` | Tracks which chunks had authored baseline entities spawned during the current load attempt. |
| `CellLoadReport` | `requested_chunks`, `spawned_chunks`, `completed_chunks`, `missing_chunks`, `spawned_entities`, `errors` | Last cell-loader pass report for tests/editor UI. |
| `CellLoadError` | `chunk`, `message` | Non-panicking cell-loader error. |
| `CellLoadRequestError` | `InvalidChunkId` | Returned by invalid load requests. |
| `CellManifestError` | invalid chunk/stable ID, duplicate stable ID | Returned by invalid manifest registration. |

`process_cell_load_requests()` runs in `AfterglowSet::ApplyGameplay`. It asks
the lifecycle layer to enter `Loading`, spawns the manifest baseline once, then
requests `Spawned` so stored persistence deltas apply through the lifecycle
system.

## Lifecycle

| Item | Fields/Variants | Purpose |
|---|---|---|
| `ChunkLifecycleState` | `Unloaded`, `Loading`, `Spawned`, `GameplayActive`, `Sleeping`, `Unloading` | Generic chunk lifecycle stage. |
| `ChunkLifecycle` | `states` | Stable chunk ID to lifecycle state map. Missing chunks read as `Unloaded`. |
| `ChunkLifecycleConfig` | `save_on_unload`, `apply_saved_delta_on_spawned` | Automatic lifecycle persistence knobs. Defaults save spawned chunks before unload and apply stored deltas on spawn. |
| `ChunkLifecycleRequests` | `load`, `spawned`, `activate`, `sleep`, `unload` | Deduplicated per-frame lifecycle requests. Invalid chunk IDs are rejected; unload wins same-frame conflicts. |
| `ChunkLifecycleReport` | `transitions`, `saved_chunks`, `applied_saved_chunks`, `despawned_entities`, `errors` | Last lifecycle pass report. Persistence failures are reported instead of panicking. |
| `ChunkLifecycleError` | `chunk`, `message` | Non-panicking lifecycle error. |

`process_chunk_lifecycle_requests()` runs in `AfterglowSet::PreparePersistence`.
Unload saves spawned/gameplay/sleeping chunks by default, skips saving partial
`Loading` chunks, despawns all stable entities in the chunk, and returns to
`Unloaded`.
