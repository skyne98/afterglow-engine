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
| Owned predicted avatar | Locally controlled player body predicts immediately through the collision-aware controller and corrects by replaying unacknowledged commands through the same collision-aware path on server snapshots. | Strategy documented; no FPS demo implementation remains. |
| Remote avatar snapshot | Non-local avatar mirrors latest replicated state directly. | Available as a simple fallback for non-critical debug mirrors. |
| Buffered interpolation | Remote physics/presentation objects render from delayed snapshot buffers. | `NetworkTransformSample` and `NetworkTransformInterpolationBuffer`. |
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

The FPS controller demo is local-only. It no longer exposes
`FpsDemoNetworkPlugin`, FPS-specific replicated avatar state, remote avatar
visualization, local Lightyear runners, or native `--connect`/`--host` launch
modes.
