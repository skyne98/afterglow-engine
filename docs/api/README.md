# Afterglow Engine API Surface

## Status

The public API has been simplified around Bevy, Lightyear, Leafwing Input
Manager, and an Afterglow server rewind layer. The older custom networking stack
has been deleted.

## Workspace Crates

| Crate | Target status |
|---|---|
| `afterglow-engine` | Main engine library |
| `agx` | Binary launcher |
| `mock-rpg-network-tests` | Living integration harness for latency, replay, and correction scenarios |

## Target Module Shape

```text
afterglow-engine
├── core/                 stable IDs, chunks, schedule sets
├── console/              clap-backed dev console overlay/commands/autocomplete
├── input/                Leafwing wrapper and `AfterglowAction`
├── network/
│   ├── lightyear/        Lightyear plugin/config/protocol glue
│   ├── interest/         chunk/area peer interest fanout
│   └── rewind/           authoritative server rewind history/replay/corrections
├── controller/           first-person controller
├── physics/              Avian-backed physics authoring
├── persistence/          stable-ID chunk deltas and save/load
├── world/                cell manifests and chunk lifecycle
└── perf_hud/             diagnostics and metrics
```

The legacy `network::{commands, authority, session, handshake, iroh, steam,
replication, prediction, reconciliation, interpolation, baseline, local_server,
rollback}` modules were removed. `network::interest` was rebuilt as a tiny
chunk-interest adapter instead of reviving the old per-entity interest map.

## Runtime Plugins

| Plugin | Purpose |
|---|---|
| `AfterglowCorePlugin` | Stable entity IDs, chunk IDs, and core schedule resources |
| `DevConsolePlugin` | Source-style console overlay plus clap-backed parser/executor, cvars, command queue, network requests, and autocomplete support |
| `AfterglowNetworkPlugin` | Lightyear boundary plus server rewind and chunk-interest plugins |
| `AfterglowLightyearPlugin` | Lightyear client/server setup when the `lightyear` feature is enabled |
| `AfterglowInputPlugin` | Leafwing action mapping for `AfterglowAction`; idempotent when Lightyear already installed the same input manager |
| `ServerRewindPlugin` | Authoritative server component history skeleton |
| `AfterglowPhysicsPlugin` | Avian-backed physics authoring and runtime integration |
| `AfterglowPersistencePlugin` | Stable-ID chunk deltas and save/load helpers |
| `AfterglowWorldPlugin` | Cell manifest loading and chunk lifecycle |
| `AfterglowEnginePlugin` | Runtime composition plus perf HUD, tracing, and metrics |
| `demo::AfterglowDemoPlugin` | Optional demo content only |

## Detailed API Notes

| File | Scope |
|---|---|
| `plugins.md` | Runtime plugin composition |
| `console.md` | Source-style dev console overlay, parser, executor, cvars, and autocomplete |
| `input.md` | Leafwing/Lightyear input target API |
| `network.md` | Lightyear + server rewind target API |
| `controller.md` | First-person controller |
| `physics.md` | Avian-backed physics |
| `world.md` | Cell manifests and lifecycle |

## Core Public Concepts

| Item | Owner | Purpose |
|---|---|---|
| `StableEntityId` | `core::identity` | Durable save/load, replication, and rewind identity |
| `ChunkId` | `core::identity` | Stable chunk/cell identifier |
| `ChunkMembership` | `core::identity` | Streaming, persistence, and chunk-interest filtering membership |
| `Persistent` | `core::identity` | Entity participates in stable persistence |
| `Replicated` | `core::identity` | Entity participates in networked gameplay truth |
| `RuntimeOnly` | `core::identity` | Entity is excluded from automatic stable ID assignment |
| `DevConsoleState` | `console` | Console open state, input buffer, history, and scrollback |
| `ConsoleNetworkRequest` | `console` | Typed console request for connect/disconnect/server/network debug operations |
| `AfterglowAction` | `input` | Leafwing `Actionlike` enum for gameplay controls |
| `RewindedEntity` | `network::rewind` | Stable server-rewind entity marker |
| `RewindDomainId` | `network::rewind` | Authoritative replay domain, such as a combat bubble or cell subsystem |
| `RewindHistoryBudget` | `network::rewind` | Retained tick/window budget for server rewind |
| `AfterglowLightyearConfig` | `network::lightyear` | Tick duration, client/server role, and Lightyear protocol settings |
| `ChunkInterestPeer` / `PeerChunkInterest` | `network::interest` | Per-peer chunk/area interest fanout for replication routing |
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
| `FixedPostUpdate` | Server rewind history capture and Lightyear prediction history |
| `PostUpdate` | Lightyear message/replication send, correction/cue publication, cleanup |

## Benchmark Commands

Current networking benches are legacy and should be replaced. Target benchmark
coverage:

| Command | Purpose |
|---|---|
| `cargo bench -p afterglow-engine --bench server_rewind` | Typed component history, restore, replay, and correction diff costs |
| `cargo bench -p afterglow-engine --bench lightyear_integration` | Lightyear replication/input/prediction pressure under Afterglow schedules |
| `cargo bench -p afterglow-engine --bench persistence_streaming` | Chunk streaming and persistence pressure |

## Test Commands

| Command | Purpose |
|---|---|
| `cargo test -p afterglow-engine` | Engine unit and renderless app tests |
| `cargo test -p mock-rpg-network-tests` | Mock RPG network-boundary and correction harness |
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
removed. `tokio` and `bytes` remain workspace dependencies only where still
needed by active crates or transitive feature work.
