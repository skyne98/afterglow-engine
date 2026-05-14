# Bevy Integration: World, Save, Streaming, And Multiplayer

## Scope

This note maps the roadmap's foundation, streaming, persistence, and multiplayer phases onto Bevy `0.18.1`.

Current local state:

- [lib.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/lib.rs:20) registers the core, input, network, world, and perf HUD plugins.
- [core/identity.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/core/identity.rs:1) owns stable IDs, chunk membership, persistence markers, replication markers, and runtime registries.
- [world/cell.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/world/cell.rs:1) owns cell manifests, cell load requests, baseline spawning, and the built-in demo manifest.
- [world/lifecycle.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/world/lifecycle.rs:1) owns the generic chunk lifecycle state machine and automatic save/apply hooks.
- [persistence/mod.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/persistence/mod.rs:1) owns stable-ID keyed chunk deltas, one-loaded-cell save/load, registered serializable components, and deleted authored-object tombstones.
- [network/local_server.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/network/local_server.rs:1) provides an opt-in local-server path where single-player `LocalPlayers` are mirrored into `NetworkSession` and local `PlayerCommand`s are submitted through `ServerCommandBuffer`.
- Snapshot/delta replication, rollback timelines, prediction, reconciliation, interpolation, reconnect baselines, and chunk interest now exist under `network/`.

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

Current plugin shape:

```text
AfterglowRuntimePlugins
  AfterglowCorePlugin
  AfterglowInputPlugin
  AfterglowNetworkPlugin
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

Current implementation: `CellManifestRegistry` stores chunk-keyed authored
baselines, `CellLoadRequests` holds pending load requests, and
`process_cell_load_requests()` drives lifecycle progress. The loader requests
`ChunkLifecycleState::Loading`, spawns the manifest baseline exactly once for
the load attempt, then requests `Spawned` so saved deltas apply through the
lifecycle system. If saved-delta application fails, the load request remains
pending and the baseline is not duplicated.

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

The current engine lifecycle layer provides the first generic version of this:
`ChunkLifecycleRequests` queues load/spawned/activate/sleep/unload requests,
`ChunkLifecycle` stores stable chunk states, and `ChunkLifecycleReport` exposes
transitions plus non-panicking persistence errors for tests/editor UI. Unload
saves chunk deltas for spawned/gameplay/sleeping chunks and despawns loaded
chunk entities by default. Unloading while still in `Loading` cleans partial
spawns without saving an incomplete delta. Marking a chunk spawned applies
stored persistent deltas by default. These defaults can be disabled through
`ChunkLifecycleConfig` when tools need manual control.

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

Single-player should run as a local server. That means gameplay should already flow through commands:

```rust
pub struct PlayerCommand {
    pub player: NetworkPlayerId,
    pub tick: u32,
    pub axes: Vec<InputAxisValue>,
    pub actions: Vec<InputAction>,
    pub pointers: Vec<PointerInput>,
}
```

Replication and save should share schemas:

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
2. Move demo spawning behind a cell/chunk loader. Done: the demo cell is now a `CellManifest` loaded by `process_cell_load_requests()`, not a startup spawn system.
3. Add explicit engine system sets.
4. Implement one-cell save/load by stable ID. Done: `LoadedCellSave` captures versioned chunk deltas, merges recorded tombstones, roundtrips through JSON, retains loaded tombstones for later saves, and rejects unknown save versions before mutation.
5. Add chunk lifecycle resources and commands. Done: `ChunkLifecycleRequests` and `process_chunk_lifecycle_requests()` provide generic load/spawn/activate/sleep/unload transitions with automatic persistence hooks.
6. Split CPU loaded, gameplay active, render extractable, GPU resident.
7. Add local-server command path. Done: `LocalServerConfig::single_player()` mirrors local players into the authoritative session and submits local commands through normal server authority.
8. Add snapshot/delta records using the same schema as save.
9. Add chunk interest and replication tests.
