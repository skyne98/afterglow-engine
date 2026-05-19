# Network API

## Status

The network API is now narrowed to a Lightyear integration boundary plus a small
Afterglow server rewind layer. The previous custom transport/session/command/
replication/prediction/interpolation stack has been deleted.

## Plugin Surface

| Item | Purpose |
|---|---|
| `AfterglowNetworkPlugin` | Adds `AfterglowLightyearPlugin`, `ServerRewindPlugin`, and `ChunkInterestPlugin`. |
| `AfterglowLightyearPlugin` | Initializes `AfterglowLightyearConfig`; with the `lightyear` feature, adds Lightyear client/server plugin groups and Leafwing input networking. Concrete link/transport entity setup is deferred. |
| `ChunkInterestPlugin` | Recomputes per-peer chunk interest from `ChunkInterestPeer` plus `ChunkMembership`. |
| `ChunkInterestPeer` | Marks a connected player/avatar entity as an interest source and configures a raw chunk-ID radius. |
| `PeerChunkInterest` | Runtime resource mapping peer `StableEntityId`s to interested `ChunkId`s and fanout entities. |
| `AfterglowLightyearConfig` | Engine-facing Lightyear config: role, server/remote addresses, tick rate, prediction window, protocol id, optional connect token, and link-conditioner settings. |
| `ServerRewindPlugin` | Registers rewind identity/history types, budget resources, typed component registration, and fixed-post-update history capture. Replay systems remain the next slice. |
| `ComponentHistory` / `HistoryEntry` | Opaque per-component tick history ring used by server rewind and the mock RPG harness. |
| `RewindComponentRegistry` | Domain-scoped list of registered rewind component serializers. |
| `RewindHistoryStore` | Runtime resource keyed by `(StableEntityId, type_key)` that stores captured `ComponentHistory` rings. |
| `RewindHistoryBudget` / `RewindTick` | Retained history budget and current authoritative rewind tick. |

## Universal Identity

`StableEntityId` is the only engine-level entity ID source. It is used for
persistence, Lightyear replication identity, and server rewind history. Raw Bevy
`Entity` values are local handles only and must not appear in network payloads,
rewind correction payloads, save data, or cross-peer gameplay references.

Entities that are `Persistent`, `Replicated`, or `RewindedEntity` receive a
`StableEntityId` automatically unless they are marked `RuntimeOnly`. The
allocator skips IDs already authored in the world, so auto-generated IDs do not
collide with scene/persistence/network IDs.

## Replication Pattern

Register networked components through Lightyear, not the old `Replicate` macro:

```rust
app.register_component::<Health>();
app.register_component::<CombatTransform>();
app.register_component::<ShieldState>();
```

Spawn replicated entities with Lightyear replication markers plus the Afterglow
stable identity and optional rewind markers:

```rust
commands.spawn((
    StableEntityId::new(...),
    RewindedEntity { domain, budget_override: None },
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
| Owned predicted avatar | Locally controlled player body predicts immediately through the collision-aware controller and corrects by replaying unacknowledged commands through the same collision-aware path on server snapshots. | FPS demo `FpsMovementHistory`, `FpsPendingReplay`, `ReplayCommand`, `FpsDemoPredictionBuffer`, `FpsDemoInputCommand`, `FpsDemoPlayerState::authoritative_tick`, and owned snapshot handling. |
| Remote avatar snapshot | Non-local avatar mirrors latest replicated state directly. | Available as a simple fallback for non-critical debug mirrors. |
| Buffered interpolation | Remote physics/presentation objects render from delayed snapshot buffers. | `NetworkTransformSample` and `NetworkTransformInterpolationBuffer`; FPS remote avatars use the buffer for delayed transform presentation. |
| Kinematic remote observer | Server/master-driven physics object writes authoritative transform samples for remote presentation. | `PhysicsKinematicRemote` plus `NetworkTransformInterpolationBuffer`. |
| Chunk interest filter | Replication routing by chunk/area before large-player fanout. | `ChunkInterestPeer` and `PeerChunkInterest`, backed by `ChunkMembership` and `StableEntityRegistry`. |
| Rewind tracked gameplay | Server captures authoritative component history for late-input replay/correction. | `RewindedEntity`, `RewindDomainId`, `RewindHistoryStore`, registered rewind components. |
| Local only | Cameras, UI, debug helpers, and local presentation children stay off the network. | Absence of `Replicated`/Lightyear replication/rewind markers. |

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

## Server Rewind Pattern

Register only gameplay truth that can affect late-command correction:

```rust
app.register_rewind_component::<CombatTransform>(domain);
app.register_rewind_component::<Health>(domain);
app.register_rewind_component::<ShieldState>(domain);
app.register_rewind_component::<Hurtbox>(domain);
```

The current rewind layer stores opaque `ComponentHistory` checkpoints under
`StableEntityId`, registers component serializers through the app extension, and
captures matching `RewindedEntity` components into `RewindHistoryStore` during
`FixedPostUpdate`. Entity lifecycle recording, replay, and correction diff
publication remain the next implementation slice.

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
| `InterestMap` | `ChunkInterestPeer` + `PeerChunkInterest`; Lightyear target filtering will consume this adapter in later replication-routing slices |
| `ReconnectBaselineStore` | Lightyear connect/replication plus Afterglow persistence |

The old `afterglow-engine-macros` crate and networking benches were also removed.

## Required Regression

The key server rewind scenario:

```text
T100: A raises shield.
T108: B arrow appears to kill A; corpse and food loot spawn provisionally.
T109: B picks up the food; inventory changes provisionally.
T111: A's late-but-valid shield input arrives.
Replay: shield blocks arrow.
Correction: A lives; corpse, loot, pickup, inventory delta, death cue, and stale projectile hit vanish.
```

`crates/mock-rpg-network-tests` now runs this through an actual Lightyear
client/server Crossbeam boundary for the core late-input correction path:
`AfterglowLightyearPlugin`, `ClientPlugins`, `ServerPlugins`, `CrossbeamIo`, a
registered reliable channel, `ClientInput` message registration,
`MessageSender` / `MessageReceiver`, Lightyear component replication, and
Lightyear prediction/confirmation state.
Those Lightyear-delivered inputs feed the authoritative Afterglow server rewind
simulation, which proves `StableEntityId`, `RewindHistoryStore` capture,
deterministic replay, correction outputs, corpse and loot despawns, pickup fact
removal, inventory rollback, replicated entity removal, and confirmed
authoritative correction after a real Lightyear message and replication transfer.
The same harness is also driven through real console commands by
`ConsoleNetworkedRpg`: `connect local` creates the connected Lightyear
client/server pair, `net latency --ms` controls input delay, `disconnect` tears
down the connection, and `net stats` reads live sent/received counters from mock
player traffic.

The local packet simulator remains for broader packet behavior coverage:
delayed, reordered, duplicated, dropped, and stale inputs plus adversarial
movement that tries to expand spell reach under latency. The remaining mock RPG
network proof gap is the native UDP/netcode socket path.

The physics interaction regression target is now:

```text
T100: Server declares a PhysicsBreakable barrel at position P.
T105: Player A or a projectile produces a server-side PhysicsImpactEvent.
T106: Relative speed exceeds the barrel threshold; server decrements health.
T108: Health reaches zero; server emits PhysicsBreakEvent and gameplay decides
      whether to despawn, swap, loot, or replicate destruction.
```

Grab/link/release uses `PhysicsGrabCommand` and `PhysicsReleaseCommand` keyed by
`StableEntityId`. The server/master validates range and writes
`PhysicsGrabbedState` with an authoritative tick; remote presentation should use
kinematic samples plus interpolation rather than client-authored transforms.

## Chunk Interest

`ChunkInterestPlugin` is the small replacement for the deleted legacy
`InterestMap`. It does not own replication transport. Instead, it computes a
deterministic peer-to-chunk fanout that Lightyear target filtering or demo/server
replication code can consume.

| Item | Purpose |
|---|---|
| `ChunkInterestPeer` | Attach to the player/avatar entity representing a connected peer. Its `radius` expands interest around the entity's current `ChunkMembership`. |
| `PeerChunkInterest` | Resource queried by server replication code: `is_interested(peer, chunk)`, `peer_chunks(peer)`, `interested_peers(chunk)`, and `interested_entities(peer, registry)`. |

The MVP neighborhood uses contiguous raw `ChunkId` values clamped away from
`ChunkId::INVALID`. World-specific 2D/3D chunk graph expansion can replace the
neighborhood producer later without changing consumers of `PeerChunkInterest`.

## FPS Demo Networking

The FPS controller demo installs `FpsDemoNetworkPlugin` through
`FpsControllerDemoPlugin`. The demo defaults to `FpsDemoNetworkConfig::local()`;
native `agx --name fps-controller --connect <addr>` uses
`FpsDemoNetworkConfig::remote(addr)`, and
`agx --name fps-controller --host <addr>` uses
`FpsDemoNetworkConfig::server(addr)`.

Current FPS demo network API:

| Item | Purpose |
|---|---|
| `FpsDemoNetworkConfig` | Launch intent: local in-process server/client, remote client address, or native UDP/netcode server bind address. |
| `FpsDemoNetworkStatus` | Testable status for connection mode, local server running state, Lightyear links, replicated avatar state, latency, and tick count. |
| `FpsDemoNetworkPlugin` | Consumes `ConsoleNetworkRequest`, starts/stops the FPS local Lightyear runner, spawns native UDP/netcode client/server links when Lightyear runtime plugins are installed, syncs visible player state into the network runner, and mirrors replicated remote avatars into the scene. |
| `FpsDemoPlayerState` | Replicated FPS avatar state paired with `StableEntityId`: millimeter position, milliradian yaw/pitch, and the authoritative input tick that produced the snapshot. |
| `FpsDemoRemoteAvatar` | Visible scene-side avatar entity created from replicated non-local FPS player state. |
| `NetworkTransformSample` | One ticked replicated transform sample for interpolation buffers. |
| `NetworkTransformInterpolationBuffer` | Bounded delayed transform interpolation buffer for remote avatars and arbitrary replicated physics objects. |

With the `multiplayer` feature, local FPS launch creates an actual Lightyear
Crossbeam server with two local clients. The visible FPS player sends raw-input
`FpsDemoInputCommand` messages over a Lightyear client-to-server channel; those
commands no longer carry a client-authored state snapshot. The client keeps a
small pending-command prediction buffer keyed by command tick. The server keeps
its own `FirstPersonMotorState`, integrates command samples through the shared
first-person controller motor math, writes the resulting `FpsDemoPlayerState`
avatar component with the producing authoritative tick, and Lightyear replicates
that avatar state back to clients. The controlled local player predicts via
the normal fixed collision-aware controller path. When an owned authoritative
snapshot arrives, the FPS network layer drops acknowledged commands, replays
unacknowledged commands from the input buffer (collision-free), and applies a
correction: direct snap if error > 2m, exponential smoothing (10% per frame) if
0.25m < error ≤ 2m, and no correction if error ≤ 0.25m. This matches the
standard server-authoritative model used by Source, Quake, and Overwatch.
Render-rate look and camera presentation remain local and are not driven by
network avatar materialization.
Non-local FPS avatars store received snapshots in
`NetworkTransformInterpolationBuffer` and render delayed interpolated transforms
instead of snapping directly to every replicated tick. The native remote path
creates a Lightyear `NetcodeClient` over `UdpIo` and sends the same raw-input
`FpsDemoInputCommand` messages once connected. For presentation
correctness, the native server keys each integrated authoritative avatar state by
the authenticated netcode client ID in the `StableEntityId` native-player
namespace instead of trusting a client-supplied player ID or transform. The host
process does not put Lightyear replication on the actual physics-controlled host
player; it mirrors that player into a separate non-physics host avatar proxy with
a dedicated native-host stable ID namespace and replicates the proxy instead.
Native server avatars are targeted through explicit connected Lightyear link
entities: the host proxy goes to every connected client, and client-owned avatars
are also returned to their owner as correction snapshots. The demo mirrors
replicated non-local avatar state into the Bevy scene as buffered
`FpsDemoRemoteAvatar` entities with simple renderable meshes when render assets
are present; native clients shield the owned correction snapshot and any
replicated state whose `StableEntityId` is already owned by a local controlled
`FpsDemoPlayer`, so local presentation stays controller-owned instead of spawning
duplicate avatars. Full server-side collision/physics authority remains a later
slice.

Windowed native hosts use the engine run helper's `WinitSettings::continuous()`
configuration, so the server keeps ticking when its window loses focus.
