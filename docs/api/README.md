# Afterglow Engine API Surface

## Status

The public API has been simplified around Bevy, Lightyear, Leafwing Input
Manager, deterministic fixed-tick simulation, and fixed server input delay. The
older custom networking stack and the experimental server-rewind history layer
have been deleted.

## Workspace Crates

| Crate | Target status |
|---|---|
| `afterglow-engine` | Main engine library |
| `agx` | Binary launcher |
| `engine-rpg-harness` | Primary Lightyear RPG integration harness for fixed delay, prediction, combat, physics, PreSpawned, transport scenarios, and multiplayer_boxes visible sync regressions |

## Target Module Shape

```text
afterglow-engine
├── core/                 stable IDs, chunks, schedule sets
├── console/              clap-backed dev console overlay/commands/autocomplete
├── input/                Leafwing wrapper and `AfterglowAction`
├── network/
│   ├── connection/       UDP/netcode identity, auth, link lifecycle, ownership binding
│   ├── lightyear/        Lightyear plugin/config/protocol glue
│   └── interpolation.rs  small transform interpolation buffer
├── controller/           first-person controller
├── physics/              Avian-backed physics authoring
└── perf_hud/             diagnostics and metrics
```

The legacy `network::{commands, authority, session, handshake, iroh, steam,
replication, prediction, reconciliation, baseline, local_server, rollback,
rewind, interest}` modules were removed.

## Runtime Plugins

| Plugin | Purpose |
|---|---|
| `AfterglowCorePlugin` | Stable entity IDs, chunk IDs, and core schedule resources |
| `DevConsolePlugin` | Source-style console overlay plus clap-backed parser/executor, cvars, command queue, network requests, and autocomplete support |
| `AfterglowNetworkPlugin` | Lightweight network context/plugin boundary; real Lightyear connection plugins are installed explicitly by apps/demos |
| `AfterglowLightyearPlugin` | Lightyear client/server setup, shared protocol registration, Leafwing input networking, and frame interpolation when the `lightyear` feature is enabled |
| `AfterglowConnectionPlugin` | Real UDP/netcode server/client spawning, identity, auth messages, input timeline setup, and ownership binding helpers |
| `AfterglowInputPlugin` | Leafwing action mapping for `AfterglowAction`; idempotent when Lightyear already installed the same input manager |
| `AfterglowPhysicsPlugin` | Avian-backed physics authoring and runtime integration |
| `AfterglowEnginePlugin` | Runtime composition plus perf HUD, tracing, and metrics |
| `demo::AfterglowDemoPlugin` | Optional demo content only |

## Detailed API Notes

| File | Scope |
|---|---|
| `plugins.md` | Runtime plugin composition |
| `console.md` | Source-style dev console overlay, parser, executor, cvars, and autocomplete |
| `input.md` | Leafwing/Lightyear input target API |
| `network.md` | Lightyear, fixed input delay, prediction, interpolation, and PreSpawned API |
| `session-api.md` | Historical/external session API notes; session discovery is no longer engine-owned |
| `session-workflows.md` | Historical/external Steam, NonSteam listen-server, and Local workflow notes |
| `controller.md` | First-person controller |
| `physics.md` | Avian-backed physics |
| `world.md` | Current world-adjacent core IDs plus planned cell/lifecycle API status |

## Core Public Concepts

| Item | Owner | Purpose |
|---|---|---|
| `StableEntityId` | `core::identity` | Durable save/load identity and cross-peer gameplay references for entities that may also be replicated |
| `StableIdAllocator` | `core::identity` | Allocates stable IDs while avoiding authored/reserved IDs |
| `RuntimeOnly` | `core::identity` | Entity is excluded from automatic stable ID assignment |
| `DevConsoleState` | `console` | Console open state, input buffer, history, and scrollback |
| `ConsoleNetworkRequest` | `console` | Typed console request for connect/disconnect/server/network debug operations |
| `AfterglowAction` | `input` | Leafwing `Actionlike` enum for gameplay controls |
| `HistoryTick` | `network` | Plain fixed-step tick counter for deterministic scenario systems |
| `AfterglowLightyearConfig` | `network::lightyear` | Tick duration, client/server role, input rebroadcast, and shared protocol settings |
| `register_afterglow_lightyear_protocol` | `network::lightyear` | Shared protocol helper for `StableEntityId`, `Transform`, `LinearVelocity`, and `HistoryTick`; player/control ownership uses upstream Lightyear `ControlledBy` / `Controlled` and Leafwing `InputMap` / `ActionState` |
| `AfterglowConnectionPlugin` | `network::connection` | Real UDP/netcode server/client spawning from external connection params, auth messages, link lifecycle events, and input timeline setup |
| `ConnectionConfig` | `network::connection` | Runtime input-delay, auth, rebroadcast, and link-conditioner settings |
| `LocalIdentity` / `LocalPlayerId` | `network::connection` | Stable `PlayerId` source and client-local player id resource; `PlayerId` is the netcode `client_id` |
| `ConnectionEvent` / `MemberLinkMap` | `network::connection` | Join/leave observer event and server player-to-link map |
| `PlayerOwned` / `ControlledEntityPlugin` | `network::connection` | Ownership tags and safe `ControlledBy` binding once `ReplicationSender` exists |
| `NetworkTransformInterpolationBuffer` | `network` | Bounded delayed transform interpolation for remote avatars and replicated physics presentation |
| `PhysicsBreakable` / `PhysicsGrabbedState` / `PhysicsGrabSpringConfig` | `physics` | Server/master-authoritative physics interaction state for impact/break and damped grab/link/release flows |

## Target Schedules

Lightyear owns the network-critical phase ordering. Afterglow systems should fit
inside that model:

| Phase | Target responsibility |
|---|---|
| `PreUpdate` | Lightyear packet/message receive and replicated state application |
| `FixedPreUpdate` | Leafwing/Lightyear input restore and buffering |
| `FixedUpdate` | Authoritative gameplay, prediction-safe movement/combat, physics-driving state |
| `FixedPostUpdate` | Lightyear prediction history and post-simulation reconciliation hooks |
| `PostUpdate` | Lightyear message/replication send, predicted cue confirmation, and cleanup |

## Benchmark Commands

Networking benches have been removed and need to be recreated. Target benchmark
coverage:

| Command | Purpose |
|---|---|
| `cargo bench -p afterglow-engine --bench lightyear_integration` | Lightyear replication/input/prediction pressure under Afterglow schedules |
| `cargo bench -p engine-rpg-harness --bench fixed_delay` | Fixed-input-delay scenario pressure, if/when a harness bench is added |
| `cargo bench -p afterglow-engine --bench persistence_streaming` | Chunk streaming and persistence pressure |

## Test Commands

| Command | Purpose |
|---|---|
| `cargo test -p afterglow-engine` | Engine unit and renderless app tests |
| `cargo test -p engine-rpg-harness` | Primary Lightyear RPG integration harness |
| `cargo test -p engine-rpg-harness multiplayer_boxes` | One-server/two-client multiplayer_boxes visible player, block, and rope sync regression |
| `cargo test -p afterglow-lightyear-avian3d transform_mode_writes` | Avian 0.6 + Lightyear Transform-mode writeback regression |
| `bun run test` | Build-system test wrapper |

## Dependencies

Target networking/input dependencies:

| Crate | Purpose |
|---|---|
| `lightyear` | Multiplayer substrate: connection, channels, replication, prediction, interpolation |
| `lightyear_inputs_leafwing` | Leafwing action networking and ticked input history |
| `leafwing-input-manager` | Local input binding and `ActionState` model |
| `bevy` | ECS, schedules, app/plugin model |
| `avian3d` | Physics |
| `serde` / `serde_json` | Persistence, protocol config, test payloads |
| `clap` | In-engine dev console command parsing and testable command API |

Legacy optional `iroh`, `steamworks`, `ggrs`, and custom macro dependencies were
removed. `tokio` and `bytes` are no longer direct workspace dependencies; they
exist only as transitive deps via Lightyear/Bevy.
