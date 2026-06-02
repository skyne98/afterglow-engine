# Bevy Integration: World, Save, Streaming, And Multiplayer

## Scope

This note maps the roadmap's foundation, streaming, persistence, and multiplayer phases onto Bevy `0.18.1`.

Current local state, with multiplayer caveat:

- [lib.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/lib.rs:20) registers core, console, input, network, physics, first-person controller, and perf HUD plugins.
- [core/identity.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/core/identity.rs:1) owns stable IDs, chunk membership, persistence markers, replication markers, and runtime registries.
- `world/` and `persistence/` modules are planned but not currently present in the engine source tree.
- The existing custom multiplayer modules under `network/` are legacy and have
  been replaced by Lightyear + Leafwing input plus fixed server input delay.

## Hard Rule: Stable Identity First

Never serialize or replicate raw Bevy `Entity`.

Bevy entities are world-local and can be remapped during scene instantiation. Relevant sources:

- `bevy_ecs-0.18.1/src/entity/map_entities.rs`
- `bevy_ecs-0.18.1/src/component/mod.rs`
- `bevy_scene-0.18.1/src/dynamic_scene.rs`
- `bevy_scene-0.18.1/src/scene_spawner.rs`

Add engine identity components before save/load or networking:

```rust
pub struct StableEntityId(pub u128);
pub struct ChunkId(pub u64);
pub struct ChunkMembership(pub ChunkId);
pub struct Persistent;
pub struct Replicated;
pub struct RuntimeOnly;
```

Maintain maps:

```text
StableEntityId -> Entity
Entity -> StableEntityId
ChunkId -> loaded entity set
```

All save and replication records should be keyed by `StableEntityId`.

## Phase 1: Playable Cell Foundation

### App/Plugin Structure

Planned plugin shape once world/persistence modules exist:

```text
AfterglowRuntimePlugins (current)
  AfterglowCorePlugin
  DevConsolePlugin
  AfterglowNetworkPlugin
  AfterglowInputPlugin
  AfterglowPhysicsPlugin
  AfterglowFirstPersonControllerPlugin

Planned world/persistence additions
  AfterglowPersistencePlugin
  AfterglowWorldPlugin

AfterglowEnginePlugin
  AfterglowRuntimePlugins
  PerfHudPlugin

AfterglowDemoPlugin
  built-in demo cell manifest/load request
  demo animation systems
```

`AfterglowRuntimePlugins` is intentionally demo-free. The executable `run()`
adds `AfterglowDemoPlugin` explicitly so engine users can start from a clean
runtime by adding only `AfterglowEnginePlugin` or `AfterglowRuntimePlugins`.

Use Bevy schedules, but define engine system sets:

```rust
pub enum AfterglowSet {
    ReadInput,
    BuildCommands,
    Simulate,
    ApplyGameplay,
    PreparePersistence,
}
```

### Scene/Cell Loading

Treat Bevy scenes as spawn templates, not save files.

Useful Bevy sources:

- `bevy_scene-0.18.1/src/scene_spawner.rs`
- `bevy_scene-0.18.1/src/dynamic_scene.rs`
- `bevy_scene-0.18.1/src/dynamic_scene_builder.rs`
- `bevy_asset-0.18.1/src/server/mod.rs`

Cell load path:

1. load cell manifest asset
2. load referenced scene/assets
3. spawn authored scene
4. assign or resolve `StableEntityId`
5. apply persistent deltas
6. mark chunk active

Planned implementation: `CellManifestRegistry` should store chunk-keyed authored
baselines, `CellLoadRequests` should hold pending load requests, and
`process_cell_load_requests()` should drive lifecycle progress. The loader should
request `ChunkLifecycleState::Loading`, spawn the manifest baseline exactly once
for the load attempt, then request `Spawned` so saved deltas apply through the
lifecycle system. If saved-delta application fails, the load request should
remain pending and the baseline should not be duplicated.

Do not use `DynamicScene` dumps as the long-term save format because entity remapping and scene despawn semantics fight persistence.

### Save/Load

Save format should store:

- world metadata
- loaded chunk list
- component snapshots keyed by stable ID
- per-chunk deltas from authored baseline
- tombstones for destroyed authored objects
- runtime-spawned persistent entities

Serialization can use `serde` now. Bevy reflection can help editor/debug workflows later, but durable saves should use explicit schemas.
Register persisted component schemas with explicit stable keys, for example
`app.persist_component_as::<DoorState>("game.door_state.v1")`. The shorter
`persist_component::<T>()` form is acceptable for prototypes, but Rust type
paths are not a durable save-file contract.

## Phase 3: Streaming And Residency

Chunk loading must be multi-layered:

```text
ManifestLoaded
AssetsLoading
Spawned
GameplayActive
PhysicsActive
RenderExtractable
GpuResident
VirtualTextureResident
Sleeping
Unloading
```

The planned engine lifecycle layer should provide the first generic version of
this: `ChunkLifecycleRequests` queue load/spawned/activate/sleep/unload requests,
`ChunkLifecycle` stores stable chunk states, and `ChunkLifecycleReport` exposes
transitions plus non-panicking persistence errors for tests/editor UI. Unload
should save chunk deltas for spawned/gameplay/sleeping chunks and despawn loaded
chunk entities by default. Unloading while still in `Loading` should clean
partial spawns without saving an incomplete delta. Marking a chunk spawned should
apply stored persistent deltas by default. These defaults should be configurable
through `ChunkLifecycleConfig` when tools need manual control.

Use Bevy assets for authored data:

- chunk manifest
- scene handles
- mesh/material handles
- probe data
- VT page metadata
- audio geometry metadata

Use task pools for derived data:

- persistence merge
- nav build/update
- replication packing
- save serialization
- disk-heavy page/chunk IO

Useful Bevy sources:

- `bevy_app-0.18.1/src/task_pool_plugin.rs`
- `bevy_asset-0.18.1/src/server/mod.rs`
- `bevy_asset-0.18.1/src/assets.rs`
- `bevy_tasks-0.18.1/src/*`

## Render/World Boundary

Chunk state should feed render extraction, not bypass it.

Useful Bevy sources:

- `bevy_transform-0.18.1/src/plugins.rs`
- `bevy_camera-0.18.1/src/visibility/mod.rs`
- `bevy_render-0.18.1/src/extract_component.rs`
- `bevy_render-0.18.1/src/sync_world.rs`

Flow:

1. main world updates chunk/entity state
2. transforms propagate in `PostUpdate`
3. visibility/proxy systems update culling inputs
4. extract compact render records
5. prepare GPU buffers/textures
6. render graph consumes residency/culling data

Do not mutate main-world gameplay from render-world systems.

## Phase 5: Multiplayer-Ready Runtime

Single-player should run through the same fixed gameplay systems as multiplayer.
The rewrite target is entity-scoped Leafwing action state, networked by
Lightyear, rather than custom `PlayerCommand` packets:

```rust
Query<(&ActionState<AfterglowAction>, &mut FirstPersonMotorState)>
```

Lightyear replication and save should share stable gameplay truth schemas:

```text
StableEntityId
ComponentKind
TickOrVersion
SerializedDelta
```

Interest management should be chunk-first:

1. compute interested chunks for each client
2. send chunk enter/leave events
3. send spawn baselines for newly interested chunks
4. send component deltas for visible/interested entities
5. omit or proxy entities outside interest

Do not design networking around Bevy scenes. Scenes are authoring/spawn data; replication is stable-ID component data.

## Recommended Module Layout

```text
src/core/
  identity.rs
  schedule.rs
  commands.rs
src/world/
  cell.rs
  manifest.rs
  lifecycle.rs
  residency.rs
src/persistence/
  snapshot.rs
  delta.rs
  save.rs
src/network/
  replication.rs
  interest.rs
  prediction.rs
```

## Implementation Order

1. Add `StableEntityId`, `ChunkId`, and chunk membership.
2. Move demo spawning behind a cell/chunk loader. Planned: the engine does not currently expose `CellManifest` or `process_cell_load_requests()`.
3. Add explicit engine system sets.
4. Implement one-cell save/load by stable ID. Planned: `LoadedCellSave` and chunk delta persistence are not current engine API.
5. Add chunk lifecycle resources and commands. Planned: `ChunkLifecycleRequests` and `process_chunk_lifecycle_requests()` are not current engine API.
6. Split CPU loaded, gameplay active, render extractable, GPU resident.
7. Add local-server command path. Planned: `LocalServerConfig::single_player()` is not current engine API.
8. Add snapshot/delta records using the same schema as save.
9. Add chunk interest and replication tests.
