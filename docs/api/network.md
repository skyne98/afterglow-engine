# Network API

## Status

The network API is now narrowed to a Lightyear integration boundary plus a small
Afterglow server rewind layer. The previous custom transport/session/command/
replication/prediction/interpolation stack has been deleted.

## Plugin Surface

| Item | Purpose |
|---|---|
| `AfterglowNetworkPlugin` | Adds `AfterglowLightyearPlugin` and `ServerRewindPlugin`. |
| `AfterglowLightyearPlugin` | Initializes `AfterglowLightyearConfig`; with the `lightyear` feature, adds Lightyear client/server plugin groups and Leafwing input networking. Concrete link/transport entity setup is deferred. |
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

Prediction and interpolation are Lightyear responsibilities. Owned player
entities should use prediction targets; remote entities should use interpolation
targets.

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
| `RemoteInterpolationBuffer` | Lightyear interpolation |
| `InterestMap` | Lightyear filtering or a later tiny chunk-interest adapter |
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

The local packet simulator remains for broader packet behavior coverage:
delayed, reordered, duplicated, dropped, and stale inputs plus adversarial
movement that tries to expand spell reach under latency. The remaining network
proof gap is the native UDP/netcode socket path.
