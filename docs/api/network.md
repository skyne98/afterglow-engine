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
| `AfterglowNetworkPlugin` | Initializes the current network context layer. Lightyear connection plugins are installed explicitly by demos/apps that need networking. |
| `AfterglowLightyearPlugin` | Initializes `AfterglowLightyearConfig`; with the `lightyear` feature, adds the selected Lightyear client or server plugin group, installs Leafwing input networking, frame interpolation for `Transform`, and registers the shared engine protocol. |
| `AfterglowLightyearConfig` | Engine-facing Lightyear config: role, tick rate, input rebroadcast, and input delay. Session discovery and matchmaking are external to the engine. |
| `AfterglowConnectionPlugin` | Real UDP/netcode connection boundary. The server variant spawns/listens with `ServerListenAddr`; the client variant connects to `ServerAddr` using `LocalIdentity`. |
| `ConnectionConfig` | Runtime auth/input/link configuration for `AfterglowConnectionPlugin`. |
| `ConnectionEvent` | Observer event emitted when a `PlayerId` connects/disconnects. Gameplay spawns/despawns authoritative player entities from this event. |
| `LocalIdentity` / `LocalPlayerId` | `PlayerId` source and local client resource. `PlayerId` is the netcode `client_id`. |
| `MemberLinkMap`, `PlayerOwned`, `ControlledEntityPlugin` | Server-side ownership helpers for mapping players to ready Lightyear links and binding `ControlledBy` safely. |
| `register_afterglow_lightyear_protocol` | Shared protocol helper that registers `StableEntityId`, `Transform`, `LinearVelocity`, and `HistoryTick` support for Lightyear. Call gameplay component registrations after Lightyear plugins are present. |
| `HistoryTick` | Plain `u32` resource used by deterministic fixed-step tests and scenario systems. It is not rewind history. |

## Universal Identity

`StableEntityId` is the only engine-level durable entity ID source. It is used
for persistence, durable identity on replicated entities, and cross-peer gameplay
references. Lightyear still owns live network entity remapping for the current
session.
Raw Bevy `Entity` values are local handles only and must not appear in network
payloads, save data, or cross-peer gameplay references.

Systems that need durable identity should require or insert `StableEntityId`.
Entities may also opt into engine-owned automatic assignment by adding
`AutoStableEntityId`; the marker requires a `StableEntityId` component and the
core plugin fills invalid ids from `StableIdAllocator` in `PreUpdate`. Entities
marked `RuntimeOnly` are for local-only entities and should not be used as
network or persistence targets. Generated IDs must be allocated through
`StableIdAllocator` so predicted runtime ids can be reserved/confirmed without
colliding with authored or replicated IDs.

When `register_afterglow_lightyear_protocol(app)` is called with the `lightyear`
feature enabled, `StableEntityId` is registered through Lightyear's component
registry. Any replicated entity that carries `StableEntityId` will replicate that
component like other registered Lightyear components.

## Replication Pattern

Register the engine protocol helper first, then register gameplay components
through Lightyear:

```rust
register_afterglow_lightyear_protocol(app);
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
| Pre-spawned predicted interaction | Locally predicted interaction/cue entities spawn immediately, then match or expire against server-spawned Lightyear entities. The client does not request authoritative spawns; the server may spawn matching entities only as a result of server-side input/gameplay logic. | Lightyear `PreSpawned`, `Replicate`, `PredictionTarget`, and `Confirmed<T>`; covered by `engine-rpg-harness` scenarios. |
| Input-delayed gameplay truth | Combat, doors, projectiles, cooldowns, status effects, and inventory decisions are derived by the server after fixed input delay. | `ActionState<AfterglowAction>`, deterministic fixed systems, and Lightyear replication/reconciliation. |
| Chunk interest filter | Replication routing by chunk/area before large-player fanout. | Planned; no current public API. |
| Local only | Cameras, UI, debug helpers, and local presentation children stay off the network. | Absence of Lightyear replication markers. |

Lightyear remains the substrate for transport, replication, prediction metadata,
and interpolation hooks. Afterglow's layer decides which strategy consumes a
replicated update for a given entity class.

Do not treat Bevy render assets as network state. Replicate stable logical
components and pose (`Transform`, game ids, gameplay state), then attach
client-local presentation components such as `Mesh3d`, `MeshMaterial3d`, UI,
cameras, and debug helpers from local prefab systems. The multiplayer boxes demo
uses this pattern for replicated `PlayerBox` / `KinematicBox` entities; dynamic
cube colors are local materials derived from replicated `StableEntityId`.

Player actions must have a single command source: Lightyear/Leafwing
`ActionState`. Do not send a parallel gameplay intent for the same input edge.
If an action affects a world entity, the server derives the target from the
authoritative world at the delayed input tick; the owning client may predict the
same derivation locally. The resulting gameplay state uses `StableEntityId` in
replicated components, not in `ActionState` and not in a client-authored target
message.

Simulation systems should be shared where possible. Branch on the global
`AfterglowNetworkContext::get_connection_status()` result for side-specific
facts (`runs_authority`, `runs_client_prediction`, `local_member_owner`) instead
of maintaining separate opaque client/server gameplay implementations.

For locally predicted entities, render Lightyear's `Predicted` copy and let
Lightyear handle rollback/reconciliation and visual correction. The engine's
`AfterglowLightyearPlugin` enables `FrameInterpolationPlugin::<Transform>` so
predicted movement is smooth between fixed ticks; entities must also receive
the `FrameInterpolate<Transform>` component. Protocols that want Lightyear
entity interpolation must register the component with
`.add_interpolation_with(...)` / `.add_linear_interpolation()`; directly calling
`InterpolationRegistry::set_interpolation()` only stores a lerp function and
does not install interpolation systems. Remote actors should render through
`Interpolated` copies where available, but do not target the same replicated
entity to the same client as both `Predicted` and `Interpolated` unless that
lifecycle has explicit tests. Confirmed roots are state anchors, not
player-facing presentation. Pick one canonical networked pose
representation per physics stack. With Avian 0.6, the engine's
`AfterglowPhysicsPlugin` automatically uses the `afterglow-lightyear-avian3d`
fork bridge (Transform mode) when the `lightyear` feature is active, disabling
Avian's own transform/interpolation plugins and coordinating
`PhysicsSchedule` ordering with Lightyear prediction history. If a local
predicted actor can physically contact props/walls, those contact participants
must exist in the local prediction world too; otherwise the actor can visually
penetrate stale server/interpolated objects until correction arrives.

For the multiplayer boxes demo, `PlayerBox` and `KinematicBox` are predicted to
all clients. The visible rendered entity is the `Predicted` copy. The live UDP
demo disables Lightyear input rebroadcast and relies on authoritative
replication/reconciliation for remote presentation; this avoids rebroadcasting
entity-targeted input before late-joining clients have a valid entity map.
Gameplay code must not copy `Confirmed<Transform>` into live predicted physics
bodies; confirmed server state is an old-tick input to Lightyear rollback/replay, not a
current-frame presentation command. The engine installs a generic confirmed-tick
rollback guard so delayed/buffered confirmed state triggers rollback checks
without demo-specific networking branches. `RopeLink` uses a stable
`StableEntityId` component in addition to its `rope_id` field so Lightyear can
confirm/predict the rope entity consistently across multiple clients. Rope
physics joints are created only on the authoritative server and on the owning
predicted client; non-owning clients keep replicated rope links visual-only
until the all-clients deterministic rope-joint model is proven safe. Local rope release has an owning-client suppression path: when the physical
rope key is released while a local rope is active, the client immediately marks
that rope id locally released, disables its visible predicted entity/joint, and
keeps re-hiding stale confirmed/predicted reappearances until authoritative
despawn catches up. A later replayed `ActionState` release still cleans up an
already-hidden local rope, and `sync_rope_joints` despawns orphan local Avian
rope joints so a hidden/missing `RopeLink` cannot leave a block physically
attached without a visible rope. This is local prediction state only; the server
still receives the release through Lightyear's native input buffer.

Use Lightyear's native input stack for player commands. The engine's
`AfterglowLightyearPlugin` installs the Leafwing Lightyear input plugin, which
owns `InputChannel` registration and tick-buffered input message semantics.
Afterglow configures input rebroadcast plus live client-link timeline
components (`InputTimeline`, `IsSynced<InputTimeline>`,
`InterpolationTimeline`, `IsSynced<InterpolationTimeline>`, and
`InputTimelineConfig`; fixed 2-tick delay by default, configurable via
`ConnectionConfig.input_delay_ticks` and
`AfterglowLightyearConfig.rebroadcast_inputs`). Do not register
`ActionState<AfterglowAction>` as a normal replicated component for player
commands. Games only need to add `InputMap<AfterglowAction>` +
`ActionState<AfterglowAction>` on controlled entities. Manual
keyboard-to-action writes must run in `FixedPreUpdate` /
`InputSystems::WriteClientInputs` so Lightyear buffers them before it restores
delayed input snapshots. Input writers that skip during rollback must still
remember physical button edges observed during the guarded window and replay
those edges once normal input writing resumes; otherwise short release edges can
be lost.

Use Lightyear's existing `ControlledBy` / `Controlled` relationship to bind an
entity to the link that controls it, and Leafwing `InputMap<AfterglowAction>` /
`ActionState<AfterglowAction>` on the controlled entity for gameplay input.
Afterglow does not add a separate player/avatar taxonomy.

`ControlledEntityPlugin` provides the reusable join-time binding layer for
entities tagged with `PlayerOwned`. Server-side `ClientOf` links require a
`ReplicationSender` at insertion time, and a readiness repair pass also wires
standard transport channels before replication buffering. `MemberLinkMap` only
exposes links after this sender is present; inserting `ControlledBy` before the
sender exists makes Lightyear reject the controlled entity. If a connection observer will spawn all-client replicated entities, queue the
spawn and apply it after the connection `PreUpdate` flush (the multiplayer boxes
server does this in `PostUpdate`). Prefer explicit-server replication mode for
live UDP spawns so Lightyear skips partially handshaken links without logging a
`ClientOf ... does not have ReplicationSender` error; `handle_connection` adds
those entities to the client once the link has `Connected + ReplicationSender`.

The remaining reusable lifecycle surface is richer orchestration: assign,
revoke, and rebind controlled entities for disconnect, respawn, possession, and
reconnect. It is not a custom action transport or entity-routing protocol.

## Input Pattern

Networked input is entity-scoped Leafwing state:

```rust
app.add_plugins(lightyear_inputs_leafwing::InputPlugin::<AfterglowAction>::default());
```

Gameplay reads `ActionState<AfterglowAction>` in fixed schedules on the server
and for remote/rebroadcast prediction. Afterglow no longer serializes custom
command DTOs for movement/combat input. If a game wants delayed server
consumption, configure `InputTimelineConfig` on the client link, but keep the
local predicted presentation immediate rather than rendering the local player
from a delayed state.

Clients send input only. They do not send authoritative spawn, despawn,
interaction-result, hit, loot, health, or door-state requests. Server gameplay
systems decide those outcomes from the delayed input stream and replicated world
state.

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

- **Production connection plugin path**
  (`udp_scenarios/multiplayer_boxes.rs`,
  `udp_scenarios/multiplayer_boxes_rope.rs`): Builds one server and two clients
  through `AfterglowLightyearPlugin` + `AfterglowConnectionPlugin`, verifies
  bidirectional Lightyear messages over real UDP, sends actual Leafwing input,
  and asserts the server plus both clients' visible predicted `Transform`s move.
  The rope variant attaches with `KeyF`, pulls a predicted block in a
  test-local deterministic physics harness, releases while moving away, and
  asserts the owning client never shows an active rope or duplicate PreSpawned
  rope hash again while the authoritative despawn catches up.

## Predicted Interaction Pattern

Use Lightyear `PreSpawned` for local interaction entities that need instant
feedback and server confirmation. Spawn them from the fixed simulation schedule
(`FixedUpdate` / Lightyear FixedMain), not render/update-only schedules, so the
recorded spawn tick and prediction history align with authoritative
confirmation:

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
presentation when they need prediction. Do not directly despawn Lightyear-tracked
predicted entities for local feedback; use Lightyear's prediction despawn
command for predicted despawns and let authoritative despawn or PreSpawned
timeout reconcile the entity.

## Regression Harness

`crates/engine-rpg-harness` is the current primary integration harness. It covers
Crossbeam and UDP transport (including full entity replication over real
sockets), UDP client-to-server `ActionState<AfterglowAction>` delivery into
authoritative gameplay systems, fixed input delay, retention windows,
PreSpawned confirmation/expiration, controller/physics behavior, combat, RPG
status effects, doors, adversarial input, and replication stress. The harness
includes UDP scenario variants (lockstep, adversarial, RPG, PreSpawned, combat,
doors, stress, full-stack manual input, and native Leafwing input) in
`scenarios/udp_scenarios/`, native Leafwing input over Crossbeam in
`scenarios/native_input.rs`, and a `scenarios/multiplayer_boxes.rs` regression
that runs one server plus two clients with the Avian/Lightyear Transform-mode
bridge installed. The multiplayer boxes scenario verifies that client input
moves the authoritative player and both clients' visible `Transform`s,
server-driven block movement reaches both visible predicted copies, and a
stable predicted `RopeLink` plus player/block movement remains visible on both
clients. The UDP rope regression additionally boxes in local release flicker:
released ropes are hidden immediately on the owning client, stale reappearance
is suppressed repeatedly, and duplicate deterministic PreSpawned hashes are
rejected. Two additional infrastructure tests in
`scenarios/native_input.rs` verify that the native input pipeline (timeline
sync, LeafwingBuffer creation, `WriteClientInputs` ordering) initializes
correctly over Crossbeam transport, even though full end-to-end input delivery
requires the UDP/Netcode connection lifecycle.

For UDP server links, the harness lets Lightyear/netcode create and own the link
entity and `MessageManager`. `AfterglowConnectionPlugin` registers
`ReplicationSender` as a required component of server-side `ClientOf` so
Lightyear replication target hooks never see an unready client in
`Server::collection()`. The readiness repair pass then idempotently wires the
`Transport` channels for Metadata, Updates, Actions, and Lightyear input. A
regression test verifies that UDP entity replication works and repeated
`connect()` calls do not replace Lightyear-owned link components.

The legacy `crates/mock-rpg-network-tests` crate still exists as a frozen oracle
for older rollback-style scenarios. It now owns its own local snapshot/replay
logic and no longer depends on engine rewind-history APIs.

## Connection / Identity Boundary

Afterglow no longer owns matchmaking or a session catalog. Session discovery, lobby membership, invites, and NAT traversal are external to the engine. The engine consumes already-chosen connection parameters and stable player identity.

| Type / Plugin | Purpose |
|---|---|
| `AfterglowConnectionPlugin::server(NetcodeConfig)` | Spawns one Lightyear netcode server from `ServerListenAddr`; requires `ReplicationSender` on each `ClientOf`, configures Lightyear channels/auth state/`MemberLinkMap`, and emits `ConnectionEvent`. |
| `AfterglowConnectionPlugin::client(NetcodeConfig)` | Spawns one Lightyear netcode client from `ServerAddr` + `LocalIdentity`; wires the client `Transport` from `ChannelRegistry`; inserts `LocalPlayerId`; starts the netcode handshake. |
| `ConnectionConfig` | Runtime connection knobs: tick rate, input delay ticks, input rebroadcast, optional link conditioner, and `require_auth`. Demos currently set `require_auth: false` while auth message timing is validated. |
| `LocalIdentity` | Local stable player id plus optional Ed25519 keypair. Steam builds should use `PlayerId = SteamId`; non-Steam builds derive `PlayerId = blake3(Ed25519_public_key)[..8]`. |
| `ConnectionEvent` | Observer event emitted on connect/disconnect with `player_id` and link entity. Games spawn/despawn authoritative player entities from this event. |
| `MemberLinkMap` | Server resource mapping `PlayerId` to ready Lightyear `ClientOf` link entities. |
| `PlayerOwned` / `ControlledEntityPlugin` | Tags entities owned by a `PlayerId` and binds Lightyear `ControlledBy` only once the owning link has a `ReplicationSender`. |
| `LocalPlayerId` | Client resource equal to the netcode `client_id`; used by input, camera, local prediction, and presentation systems. |

`PlayerId` is a `u64` and is the netcode `client_id`. Do not preinsert `LocalId`, `RemoteId`, or `Connected` for real UDP/netcode clients; the netcode plugin inserts those after handshake. Explicit IDs are only for manual/in-process links such as Crossbeam tests.

Production client link invariants:

- build the client `Transport` from the finalized `ChannelRegistry` before inserting it, so replication, auth, ping, and input channels exist in both directions;
- let Lightyear required-component hooks own `MessageManager`. Do not insert a fresh `MessageManager` after `Client`, because that can wipe the private sender/receiver component-id lists and make client-to-server messages appear present while never leaving the client;
- when `Connected` is added, insert `InputTimeline`, `IsSynced<InputTimeline>`, `InterpolationTimeline`, `IsSynced<InterpolationTimeline>`, and `InputTimelineConfig`. Lightyear input send systems filter on both synced timelines when interpolation is compiled in.

Testing note: Bevy harnesses that drive apps manually with schedules must emulate the runner lifecycle after registering Lightyear protocols and before the first update:

```rust
while app.plugins_state() == bevy::app::PluginsState::Adding {}
app.finish();
app.cleanup();
```

Lightyear builds dynamic replication buffer systems in plugin `finish()`; without this, connection/link components may look correct but replicated entity spawns will never be buffered.

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

Physics lag compensation is not part of the current engine path. It was
explored in a now-retired prototype (which proved `lightyear_avian3d::LagCompensationPlugin`
can query historical Avian colliders), but the production baseline does not use
historical collider queries for spells, projectiles, or melee. Revisit this only
if a future FPS/twitch fairness requirement needs client-view hit validation.

## Chunk Interest

The old legacy `InterestMap` was deleted and no replacement chunk-interest API is
currently exposed from `network`. Replication filtering by chunk/area remains a
future Lightyear routing slice.

## FPS Demo Networking

The FPS controller demo is local-only. It no longer exposes
`FpsDemoNetworkPlugin`, FPS-specific replicated avatar state, remote avatar
visualization, local Lightyear runners, or native `--connect`/`--host` launch
modes.

## See Also

- [`session-api.md`](session-api.md) — historical/external session API notes;
  session discovery is no longer engine-owned.
- [`session-workflows.md`](session-workflows.md) — historical Steam/NonSteam
  workflow notes for future platform/admission layers.
