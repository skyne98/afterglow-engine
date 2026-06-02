# Network API

## Status

The network API is now a Lightyear integration boundary plus small Afterglow
helpers. The old custom transport/session/command/replication/prediction stack
has been deleted. The old server-rewind history layer has also been removed.

The current authoritative multiplayer baseline is:

```text
client predicts ActionState immediately
server processes the same tick after a fixed input delay
server derives gameplay results from deterministic fixed-tick simulation
Lightyear replicates authoritative state and reconciles client prediction
```

There is no main-path server restore/replay/correction-diff system and no main-
path physics lag compensation.

## Plugin Surface

| Item | Purpose |
|---|---|
| `AfterglowNetworkPlugin` | Adds `AfterglowLightyearPlugin`. |
| `AfterglowLightyearPlugin` | Initializes `AfterglowLightyearConfig`; with the `lightyear` feature, adds Lightyear client/server plugin groups and Leafwing input networking. Concrete link/transport entity setup is still test/demo-owned. |
| `AfterglowLightyearConfig` | Engine-facing Lightyear config: role, server/remote addresses, tick rate, prediction window, protocol id, optional connect token, and link-conditioner settings. |
| `HistoryTick` | Plain `u32` resource used by deterministic fixed-step tests and scenario systems. It is not rewind history. |

## Universal Identity

`StableEntityId` is the only engine-level entity ID source. It is used for
persistence, Lightyear replication identity, and cross-peer gameplay references.
Raw Bevy `Entity` values are local handles only and must not appear in network
payloads, save data, or cross-peer gameplay references.

Systems that need durable identity should require or insert `StableEntityId`.
Entities marked `RuntimeOnly` are excluded from automatic stable-ID assignment in
helpers that perform that pass. The allocator skips IDs already authored in the
world, so generated IDs do not collide with scene, persistence, or network IDs.

## Replication Pattern

Register networked components through Lightyear, not the old `Replicate` macro:

```rust
app.register_component::<Health>();
app.register_component::<CombatTransform>();
app.register_component::<ShieldState>();
```

Spawn replicated entities with Lightyear replication markers plus stable identity:

```rust
commands.spawn((
    StableEntityId::new(...),
    Health::new(100),
    ShieldState::default(),
    Replicate::to_clients(NetworkTarget::All),
));
```

Per-entity sync strategy is explicit in design even when the current code still
uses marker combinations. See
[`docs/research/network-sync-strategy.md`](../research/network-sync-strategy.md)
for the taxonomy and assignment rules.

## Sync Strategy Taxonomy

| Strategy | Purpose | Current API surface |
|---|---|---|
| Owned predicted avatar | Locally controlled player body predicts immediately and reconciles to server state. | Strategy documented; reusable non-demo implementation remains future work. |
| Remote avatar snapshot | Non-local avatar mirrors latest replicated state directly. | Available as a simple fallback for non-critical debug mirrors. |
| Buffered interpolation | Remote physics/presentation objects render from delayed snapshot buffers. | `NetworkTransformSample` and `NetworkTransformInterpolationBuffer`. |
| Kinematic remote observer | Server/master-driven physics object writes authoritative transform samples for remote presentation. | `PhysicsKinematicRemote` plus `NetworkTransformInterpolationBuffer`. |
| Pre-spawned predicted interaction | Locally predicted interaction/cue entities spawn immediately, then match or expire against server-spawned Lightyear entities. | Lightyear `PreSpawned`, `Replicate`, `PredictionTarget`, and `Confirmed<T>`; covered by `engine-rpg-harness` scenarios. |
| Input-delayed gameplay truth | Combat, doors, projectiles, cooldowns, status effects, and inventory decisions are derived by the server after fixed input delay. | `ActionState<AfterglowAction>`, deterministic fixed systems, and Lightyear replication/reconciliation. |
| Chunk interest filter | Replication routing by chunk/area before large-player fanout. | Planned; no current public API. |
| Local only | Cameras, UI, debug helpers, and local presentation children stay off the network. | Absence of Lightyear replication markers. |

Lightyear remains the substrate for transport, replication, prediction metadata,
and interpolation hooks. Afterglow's layer decides which strategy consumes a
replicated update for a given entity class.

## Input Pattern

Networked input is entity-scoped Leafwing state:

```rust
app.add_plugins(lightyear_inputs_leafwing::InputPlugin::<AfterglowAction>::default());
```

Gameplay reads `ActionState<AfterglowAction>` in fixed schedules. Afterglow no
longer serializes custom command DTOs for movement/combat input.

The server should process inputs after a fixed delay sized to the expected
latency budget. This keeps authoritative simulation deterministic without
rewinding the world for late commands.

Two test paths exercise UDP input delivery:

- **Native Leafwing path** (`udp_scenarios/native_input.rs`): Installs
  `InputPlugin::<AfterglowAction>` and `InputPlugin` (Bevy). Writes desired
  input in `FixedPreUpdate` via `InputSystems::WriteClientInputs`. Inserts
  `default_gameplay_input_map()` on controlled client entities. Configures
  `InputTimelineConfig::default().with_input_delay(InputDelayConfig::fixed_input_delay(2))`
  on each client link after UDP connect, then waits for
  `IsSynced<InputTimeline>`. Verifies movement, combat, shield, and edge
  semantics (just-pressed/just-released) over real UDP — no manual
  `MessageSender<ActionState<AfterglowAction>>`.

- **Manual message path** (`udp_scenarios/full_stack.rs`): Older comparison
  path that sends `ActionState<AfterglowAction>` over a manual Lightyear
  `MessageSender`/`MessageReceiver`. Retained as a regression baseline against
  the native input route.

## Predicted Interaction Pattern

Use Lightyear `PreSpawned` for local interaction entities that need instant
feedback and server confirmation:

```rust
commands.spawn((
    DoorGrab { player, door },
    PreSpawned::new(door_grab_hash(player, door)).for_receiver(client_link),
));
```

The server computes the same deterministic hash only when it accepts the
interaction. Matching server replication confirms the predicted entity; no match
means Lightyear expires the local entity. Use this for grab links, projectiles,
door-use cues, hit markers, damage numbers, beams, decals, and other reversible
presentation when they need prediction.

## Regression Harness

`crates/engine-rpg-harness` is the current primary integration harness. It covers
Crossbeam and UDP transport (including full entity replication over real
sockets), UDP client-to-server `ActionState<AfterglowAction>` delivery into
authoritative gameplay systems, fixed input delay, retention windows,
PreSpawned confirmation/expiration, controller/physics behavior, combat, RPG
status effects, doors, adversarial input, and replication stress. The harness
now includes **28 UDP scenario variants** (lockstep, adversarial, RPG,
PreSpawned, combat, doors, stress, full-stack manual input, and native Leafwing
input) in `scenarios/udp_scenarios/` and native Leafwing input over Crossbeam
in `scenarios/native_input.rs`, bringing the total test count to **80
tests** across both transport backends.  Two additional infrastructure tests
in `scenarios/native_input.rs` verify that the native input pipeline (timeline
sync, LeafwingBuffer creation, `WriteClientInputs` ordering) initializes
correctly over Crossbeam transport, even though full end-to-end input delivery
requires the UDP/Netcode connection lifecycle.

For UDP server links, the harness lets Lightyear/netcode create and own the link
entity, `Transport`, and `MessageManager`. The harness decorates server-side
`LinkOf` entities with only `ReplicationSender`; a regression test verifies that
UDP entity replication works and repeated `connect()` calls do not replace
Lightyear-owned link components.

The legacy `crates/mock-rpg-network-tests` crate still exists as a frozen oracle
for older rollback-style scenarios. It now owns its own local snapshot/replay
logic and no longer depends on engine rewind-history APIs.

## Legacy Removals

Removed old public APIs:

| Legacy API | Replacement |
|---|---|
| `NetworkTransport`, `MemoryTransport`, `PacketHeader`, `NetworkPacket` | Lightyear transport/channels/messages |
| `NetworkSession`, `PeerId`, `NetworkPlayerId` custom stack | Lightyear peer/client state plus `StableEntityId` avatar mapping |
| `ServerCommandBuffer`, `PlayerCommand`, command wire DTOs | Leafwing action state through Lightyear input networking |
| `ReplicationWorld`, `WorldSnapshot`, `WorldDelta`, `Replicate` macro | Lightyear component replication |
| `ClientPredictionBuffer`, `ClientReconciliationQueue` | Lightyear prediction/reconciliation |
| `RemoteInterpolationBuffer` | `NetworkTransformInterpolationBuffer` for small transform presentation buffers, or Lightyear interpolation for full replicated entity streams |
| `ServerRewindPlugin`, `RewindHistoryStore`, `RewindedEntity` | Fixed input delay plus deterministic simulation; `HistoryTick` remains as a plain tick counter |
| `InterestMap` | Planned Lightyear replication filtering or a small future Afterglow adapter |
| `ReconnectBaselineStore` | Lightyear connect/replication plus Afterglow persistence |

The old `afterglow-engine-macros` crate and networking benches were also removed.

## Lag Compensation

Physics lag compensation is not part of the current engine path. The focused
prototype in `crates/prototypes/prototype-physics-lightyear` proves that
`lightyear_avian3d::LagCompensationPlugin` can query historical Avian colliders,
but the production baseline does not use historical collider queries for spells,
projectiles, or melee. Revisit this only if a future FPS/twitch fairness
requirement needs client-view hit validation.

## Chunk Interest

The old legacy `InterestMap` was deleted and no replacement chunk-interest API is
currently exposed from `network`. Replication filtering by chunk/area remains a
future Lightyear routing slice.

## FPS Demo Networking

The FPS controller demo is local-only. It no longer exposes
`FpsDemoNetworkPlugin`, FPS-specific replicated avatar state, remote avatar
visualization, local Lightyear runners, or native `--connect`/`--host` launch
modes.
