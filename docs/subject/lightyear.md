# Lightyear 0.26.4 — Definitive API Reference

Sources: docs.rs, source code at crates.io, `lightyear/examples/` on GitHub, and Afterglow's own usage.

---

## 1. Overview

Lightyear is a **Bevy-native networking library** for multiplayer games. Instead of exposing a `Client` or `Server` singleton, Lightyear models **peers as entities** with marker components (`Client`, `Server`) and a `Link` for IO. The architecture is layered:

| Layer | Crate | Purpose |
|-------|-------|---------|
| IO | `lightyear_link` + backends (`crossbeam`, `udp`, `websocket`, `webtransport`, `steam`) | Send/receive raw bytes |
| Connection | `lightyear_connection` (+ `netcode`, `steam`, `raw_connection`) | Long-lived `PeerId` identity on top of a `Link` |
| Transport | `lightyear_transport` | Channel-based packet assembly with reliability/ordering |
| Messages | `lightyear_messages` | `Message` trait + `MessageSender`/`MessageReceiver`/`MessageManager` for user-defined serializable types |
| Replication | `lightyear_replication` | Entity state replication, `Replicate` component, authority, visibility |
| Prediction | `lightyear_prediction` | Client-side prediction + rollback with `Predicted` marker |
| Interpolation | `lightyear_interpolation` | Smooth visual interpolation via `Interpolated` marker |
| Frame Interpolation | `lightyear_frame_interpolation` | Interpolates between FixedMain ticks for rendering |
| Sync | `lightyear_sync` | Timeline synchronization between client and server |
| Input | `lightyear_inputs` + backends (`native`, `leafwing`, `bei`) | Networked input queue and plugin |

---

## 2. Feature Catalog

Features from `lightyear/Cargo.toml` (reproduced from `lib.rs` doc attributes):

| Feature | Default | What it enables |
|---------|---------|-----------------|
| `client` | yes | `ClientPlugins`, client connection logic |
| `server` | yes | `ServerPlugins`, server connection logic |
| `deterministic` | no | Deterministic replication (input-only replication + checksums) |
| `replication` | yes | Entity/component replication via `Replicate` |
| `prediction` | yes | Client-side prediction, rollback, `Predicted` |
| `frame_interpolation` | no | Frame-level interpolation between FixedMain steps |
| `interpolation` | yes | Entity state interpolation, `Interpolated`, `ConfirmedHistory` |
| `trace` | no | Tracing instrumentation on key functions |
| `metrics` | no | Metrics collection via `metrics` crate |
| `debug` | no | Debug UI plugin |
| `input_native` | no | Native input backend (user-defined input structs) |
| `leafwing` | no | Leafwing input-manager backend |
| `input_bei` | no | bevy_enhanced_input backend |
| `avian2d` | no | LightyearAvianPlugin for 2D (Avian physics) |
| `avian3d` | no | LightyearAvianPlugin for 3D (Avian physics) |
| `udp` | no | UDP IO backend |
| `crossbeam` | no | Crossbeam channel IO (in-process testing) |
| `webtransport` | no | WebTransport IO |
| `webtransport_dangerous_configuration` | no | Unencrypted WebTransport for testing |
| `websocket` | no | WebSocket IO |
| `steam` | no | Steam networking (IO + connection) |
| `netcode` | no | Netcode.io connection standard |
| `raw_connection` | no | Use IO layer directly as connection |

---

## 3. Module Map

### Top-level crate modules (`lightyear`)

| Module | Feature | Re-exports |
|--------|---------|------------|
| `client` | `client` | `ClientPlugins` plugin group |
| `server` | `server` | `ServerPlugins` plugin group |
| `shared` | (always) | `SharedPlugins` — shared between client/server |
| `protocol` | `replication` | Protocol verification (checksum match between client/server) |
| `core` | (always) | `lightyear_core` — `Tick`, `Timeline`, `LocalTimeline`, `Predicted`, `Interpolated` |
| `link` | (always) | `lightyear_link` — `Link`, `LinkSender`, `LinkReceiver`, `LinkState` |
| `connection` | (always) | `lightyear_connection` — `Client`, `Server`, `PeerId`, `NetworkDirection`, `NetworkTarget` |
| `interpolation` | `interpolation` | `Interpolated`, `ConfirmedHistory`, `InterpolationPlugin` |
| `prediction` | `prediction` | `Predicted`, `PredictionPlugin`, `RollbackPolicy`, `PredictionManager` |
| `frame_interpolation` | `frame_interpolation` | Frame-level interpolation |
| `input` | any input feature | `lightyear_inputs` + sub-modules `native`, `leafwing`, `bei` |
| `crossbeam` | `crossbeam` | `CrossbeamIo` |
| `netcode` | `netcode` | Netcode connection types |
| `steam` | `steam` | Steam networking types |
| `webtransport` | `webtransport` | WebTransport types |
| `websocket` | `websocket` | WebSocket types |
| `avian2d` | `avian2d` | LightyearAvianPlugin (2D) |
| `avian3d` | `avian3d` | LightyearAvianPlugin (3D) |
| `utils` | (always) | `lightyear_utils` (registry, collections, metrics helpers) |

### Prelude structure

`lightyear::prelude` re-exports from:
- `lightyear_connection::prelude` — `Client`, `Server`, `Connected`, `Connecting`, `Disconnected`, `PeerId`, `LocalId`, `RemoteId`, `NetworkDirection`, `NetworkTarget`, `LinkOf`, `ClientOf`
- `lightyear_core::prelude` — `Tick`, `LocalTimeline`, `Predicted`, `Interpolated`, `PeerId`, `LocalId`, `RemoteId`, `Timeline`, `SyncEvent`, `Rollback`, `is_in_rollback`
- `lightyear_link::prelude` — `Link`, `Linked`, `Linking`, `Unlinked`, `LinkStart`, `Unlink`, `LinkStats`
- `lightyear_messages::prelude` — `Message`, `MessageSender`, `MessageReceiver`, `MessageManager`, `AppMessageExt`, `AppTriggerExt`, `EventSender`, `RemoteEvent`
- `lightyear_replication::prelude` — `Replicate`, `Replicating`, `Replicated`, `Confirmed`, `PreSpawned`, `Controlled`, `ControlledBy`, `PredictionTarget`, `InterpolationTarget`, `AppComponentExt`, `ComponentRegistry`, `ReplicationSender`, `ReplicationReceiver`, `NetworkVisibility`, `Room`, `AuthorityBroker`, `HasAuthority`
- `lightyear_serde::prelude` — `Serialize`, `Deserialize`, `ToBytes`, `MapEntities`
- `lightyear_sync::prelude` — `SyncConfig`, `SyncEvent`, `IsSynced`
- `lightyear_transport::prelude` — `Transport`, `ChannelSettings`, `ChannelMode`, `ReliableSettings`, `AppChannelExt`
- `lightyear_prediction::prelude` — `PredictionPlugin`, `PredictionManager`, `RollbackPolicy`, `PredictionHistory`, `PredictionRegistry`, `VisualCorrection`
- `lightyear_interpolation::prelude` — `InterpolationPlugin`, `ConfirmedHistory`, `InterpolationTimeline`, `InterpolationDelay`

Sub-modules within prelude:
- `prelude::client` — `ClientPlugins`, `Client`, `Connected`, `InputTimeline`, `IsSynced`, connection backends (Netcode, Steam, WebSocket, WebTransport, Raw)
- `prelude::server` — `ServerPlugins`, `Server`, `Started`, `ClientOf`, `LinkOf`, connection backends

---

## 4. Core Concepts

### 4.1 Plugin Setup

Two `PluginGroup`s bootstrap everything:

```rust
app.add_plugins(ClientPlugins { tick_duration: Duration::from_secs_f64(1.0 / 60.0) });
app.add_plugins(ServerPlugins { tick_duration: Duration::from_secs_f64(1.0 / 60.0) });
```

`ClientPlugins.build()` adds, in order: `ClientPlugin` (sync), `SharedPlugins` (transport, messages, connection, replication if feature enabled), optional `PredictionPlugin`, IO plugins (WebTransport, WebSocket, Steam), connection plugins (Netcode, Raw).

`ServerPlugins.build()` adds: `ServerPlugin` (sync), `ServerLinkPlugin`, `SharedPlugins`, optional deterministic checksums, `HostPlugin`, optional `HostServerPlugin`, IO plugins, connection plugins.

**Ordering requirement** (from lib.rs doc): Protocol (messages, components, channels) must be added AFTER `ClientPlugins`/`ServerPlugins` but BEFORE any `Client` or `Server` entity is spawned. Source: `lightyear/src/lib.rs:96`.

**HostServer mode**: Running both client and server in the same process. `HostServerPlugin` from `lightyear_replication::host` adds checks so server-side systems skip predicted entities (they check `Has<Predicted>`). Source: `lightyear/src/server.rs:65`, `lightyear_connection/src/host.rs`.

### 4.2 Entity-as-Peer Model

There is no single "Client" or "Server" singleton. Instead:

- **Server entity**: spawn with `Server::default()`, add `Link`, `Transport`, `MessageManager`, etc. Trigger `Start` to start listening.
- **Manual/in-memory client entity**: for local Crossbeam/raw links with no handshake, spawn with `Client::default()`, explicit `LocalId`, `RemoteId(PeerId::Server)`, `Connected`, `Link`, `Transport`, `MessageManager`, `ReplicationReceiver`, `PredictionManager`, then trigger `Connect` or mark linked as appropriate for the test transport.
- **Netcode client entity**: spawn with `Client::default()`, `NetcodeClient`, IO/backend, `Link`, `Transport`, `MessageManager`, `ReplicationReceiver`, and `PredictionManager`, then trigger `Connect`. Do **not** preinsert `LocalId`, `RemoteId`, or `Connected`; `NetcodeClientPlugin` inserts them when the handshake succeeds.
- **Per-client link on server** (`LinkOf`): When a new client connects, a new entity is spawned with `LinkOf`. Use an `On<Add, LinkOf>` observer/system to decorate it with `ReplicationSender`, `MessageSender<X>`, `ClientOf`, etc. Wait for/insert `ReplicationSender` before using the link as a `ControlledBy` owner.

State machine components for links: `Unlinked` → `Linking` → `Linked`.
State machine components for connections: `Disconnected` → `Connecting` → `Connected`.
State machine for server: `Stopped` → `Starting` → `Started`.

### 4.3 Channels, Messages, Components

**Channels** define reliability/ordering:

```rust
app.add_channel::<MyChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    send_frequency: Duration::default(),
    priority: 0.0,
}).add_direction(NetworkDirection::Bidirectional);
```

`ChannelMode` variants (from `lightyear_transport/src/channel/mod.rs`):
- `UnorderedUnreliable` — no ordering, no reliability
- `SequencedUnreliable` — ordering preserved, but no retransmission
- `OrderedReliable(ReliableSettings)` — ordering + retransmission
- `UnorderedReliable(ReliableSettings)` — retransmission, no ordering

Default channels built-in: `PingChannel` (SequencedUnreliable), `MetadataChannel` (UnorderedReliable), `ActionsChannel` (UnorderedReliable — spawn/despawn/insert/remove), `UpdatesChannel` (SequencedUnreliable — component updates).

**Messages** are user-defined `Serialize + Deserialize` structs:

```rust
app.register_message::<MyMessage>().add_direction(NetworkDirection::ClientToServer);
```

`NetworkDirection`: `ClientToServer`, `ServerToClient`, `Bidirectional`.

**Components** registered for replication:

```rust
app.register_component::<MyComponent>()
    .add_prediction()
    .add_interpolation_with(my_interpolation_fn)
    .add_linear_correction_fn::<MyCorrection>()
    .add_map_entities();
```

Chainable on `ComponentRegistration`:
- `add_prediction()` — enable client-side prediction/rollback
- `add_linear_interpolation()` / `add_interpolation_with(fn)` — install Lightyear interpolation systems for the component. Merely writing to `InterpolationRegistry::set_interpolation::<T>(fn)` only stores a lerp function and does not add the systems.
- `add_linear_correction_fn()` — smooth rollback correction (no `Ease` needed)
- `add_should_rollback(fn)` — custom rollback trigger comparator
- `add_map_entities()` — register `MapEntities` for entity references

### 4.4 Replication

To replicate an entity from server to client, add the `Replicate` component:

```rust
commands.spawn((
    PlayerBundle::new(...),
    Replicate::to_clients(NetworkTarget::All),
    PredictionTarget::to_clients(NetworkTarget::Single(client_id)),
    ControlledBy { owner: trigger.entity, lifetime: Lifetime::SessionBased },
));
```

`Replicate` automatically requires `Replicating`, `ReplicationGroup`, and `ReplicationState` components (via `#[require]` attribute hooks). Source: `lightyear_replication/src/send/components.rs:758-762`.

**Targeting**:
- `NetworkTarget::All` — replicate to all clients
- `NetworkTarget::Single(PeerId)` — replicate to one client
- `NetworkTarget::AllExceptSingle(PeerId)` — replicate to all except one
- `NetworkTarget::AllExcept(Vec<PeerId>)` — exclude multiple
- `NetworkTarget::Only(Vec<PeerId>)` — include only listed

`PredictionTarget` and `InterpolationTarget` are type aliases for `ReplicationTarget<Predicted>` and `ReplicationTarget<Interpolated>` respectively.

Marker components on replicated entities:
- `Replicated` — entity came from a remote peer via replication
- `Replicating` — entity is currently being sent to a remote peer (pause replication by removing this)

### 4.5 Prediction & Rollback

On the client, a replicated entity can exist in three forms:

1. **`Confirmed` entity** — directly applies the server's state. Has `Confirmed<T>` wrapper components.
2. **`Predicted` entity** — a client-side copy that is rolled forward using local inputs, rolled back when server corrections arrive.
3. **`Interpolated` entity** — lags behind for smooth visual interpolation.

Flow:
1. Server spawns entity with `Replicate`, `PredictionTarget`
2. Client receives and spawns both a `Confirmed` entity (applies raw server state) and a `Predicted` entity (starts predicting)
3. Client applies inputs to `Predicted` entity in `FixedUpdate`
4. Server sends back authoritative component values
5. Lightyear compares `Predicted`'s history with `Confirmed`. On mismatch, rolls back the `Predicted` entity to the confirmed state and re-applies all pending inputs

The `Predicted` component (`lightyear_core/src/prediction.rs:14`) is the marker. Systems query with `With<Predicted>` to run prediction logic.

### 4.6 PreSpawned

`PreSpawned` lets the client spawn an entity immediately and later match it with the server's authoritative version.

**Client** spawns with a hash:
```rust
commands.spawn((
    MyComponent { ... },
    PreSpawned::new(custom_hash).for_receiver(client_entity),
));
```

**Server** spawns with the same hash:
```rust
commands.spawn((
    MyComponent { ... },
    PreSpawned::new(custom_hash),
    Replicate::to_clients(NetworkTarget::All),
    PredictionTarget::to_clients(NetworkTarget::All),
));
```

When the replicated server entity arrives on the client, Lightyear matches by hash:
1. Removes `PreSpawned` from the client entity
2. Attaches server authority data (the existing predicted entity stays alive)
3. If unmatched within timeout (~60-70 ticks), cleans up the prespawned entity

`PreSpawned` fields:
- `hash: Option<u64>` — if `None`, computed from `(spawn_tick, archetype_components)`
- `user_salt: Option<u64>` — extra salt passed to default hasher
- `receiver: Option<Entity>` — which client link entity is prespawning

Methods: `PreSpawned::new(hash)`, `PreSpawned::default()`, `PreSpawned::default_with_salt(salt)`, `.for_receiver(entity)`.

**Important**: PreSpawned entities MUST be spawned in the `FixedMain` schedule (from the docs).

### 4.7 Input Handling

Three backends:

1. **Native** (`input_native`): User-defined struct with `Serialize + Deserialize`. `InputPlugin::<Inputs>::default()`.
2. **Leafwing** (`leafwing`): `ActionState<Inputs>` via `leafwing_input_manager`. `InputPlugin::<PlayerActions>::default()`.
3. **BEI** (`input_bei`): `bevy_enhanced_input` actions.

Input is buffered in `FixedPreUpdate` → consumed in `FixedUpdate`. The `InputTimeline` tracks when to buffer inputs so they arrive on time at the server.

For Leafwing input, `lightyear_inputs_leafwing::InputPlugin::<A>` registers Lightyear's native `InputChannel` and input messages. Do not also replicate `ActionState<A>` as a normal component just to move player commands; the input plugin owns tick-buffered send/rebroadcast/replay semantics. `ActionState` is still a local component on controlled entities and may be read by gameplay systems.

---

## 5. API Reference Tables

### 5.1 Plugin Setup

| Type | Kind | Description |
|------|------|-------------|
| `ClientPlugins` | `PluginGroup` | All client plugins. Field: `tick_duration: Duration` |
| `ServerPlugins` | `PluginGroup` | All server plugins. Field: `tick_duration: Duration` |
| `SharedPlugins` | `Plugin` | Shared between client/server — transport, messages, connection, replication, interpolation |
| `NetworkTimelinePlugin<T>` | `Plugin` | Configures a network timeline (internal) |
| `TimelinePlugin` | `Plugin` | Sets up `LocalTimeline`, `TickDuration`, `Time<Fixed>` |
| `ConnectionPlugin` | `Plugin` | Connection state machine logic |
| `TransportPlugin` | `Plugin` | Channel/transport setup |
| `MessagePlugin` | `Plugin` | Message send/receive systems |
| `PredictionPlugin` | `Plugin` | Client-side prediction with rollback |
| `InterpolationPlugin` | `Plugin` | Entity state interpolation |
| `NetworkVisibilityPlugin` | `Plugin` | Interest management / visibility |
| `HierarchySendPlugin<T>` | `Plugin` | Propagates replication through hierarchy relationships |
| `AuthorityPlugin` | `Plugin` | Authority transfer between peers |
| `RoomPlugin` | `Plugin` | Room-based interest management |
| `MetricsPlugin` | `Plugin` | Metrics collection + garbage collection |
| `DebugUIPlugin` | `Plugin` | Debug UI (requires `debug` feature) |
| `LightyearAvianPlugin` | `Plugin` | Avian physics bridge (feature `avian2d`/`avian3d`) |
| `LagCompensationPlugin` | `Plugin` | Historical collision queries (lightyear_avian) |
| `HostServerPlugin` | `Plugin` | HostServer mode logic for replication |
| `ProtocolCheckPlugin` | `Plugin` | Verifies client/server protocol match |

### 5.2 Link & Connection

| Type | Kind | Description |
|------|------|-------------|
| `Link` | `Component` | Core IO link — stores send/receive payload buffers |
| `LinkSender` | `Component` | Buffers outgoing payloads |
| `LinkReceiver` | `Component` | Receives and buffers incoming payloads |
| `LinkState` | `Enum` | `Unlinked`, `Linking`, `Linked` |
| `Unlinked` | `Component` | Link is not established |
| `Linking` | `Component` | Link is being established |
| `Linked` | `Component` | Link is established |
| `LinkStart` | `Event` | Trigger to initiate link establishment |
| `Unlink` | `Event` | Trigger to terminate a link |
| `LinkStats` | `Component` | Statistics: bytes/packets sent/received, RTT, jitter |
| `LinkConditioner` | `Component` | Simulates network conditions (latency, jitter, loss) |
| `RecvLinkConditioner` | Type alias | LinkConditioner for `RecvPayload` |
| `SendPayload` | Type alias | `IoPayload` for sending |
| `RecvPayload` | Type alias | `IoPayload` for receiving |
| `Client` | `Component` | Marker: entity is a client |
| `Server` | `Component` | Marker: entity is a server |
| `PeerId` | `Enum` | `Local(u16)` or `Server` or `Netcode(u64)` or `Steam(u64)` |
| `LocalId` | `Component` | Stores local `PeerId` for the connection |
| `RemoteId` | `Component` | Stores remote `PeerId` for the connection |
| `Connect` | `Event` | Trigger to connect the client |
| `Disconnect` | `Event` | Trigger to disconnect the client |
| `Connected` | `Component` | Connection is established |
| `Connecting` | `Component` | Connection is being established |
| `Disconnected` | `Component` | Connection is not established |
| `Start` | `Event` | Trigger to start the server |
| `Stop` | `Event` | Trigger to stop the server |
| `Started` | `Component` | Server is running |
| `Starting` | `Component` | Server is starting |
| `Stopped` | `Component` | Server is stopped |
| `LinkOf` | `Component` | Added to per-client link entities on the server |
| `ClientOf` | `Component` | Marker on per-client link entities |
| `PeerMetadata` | `Resource` | Mapping from `PeerId` to local `Entity` |
| `ConnectionError` | `Enum` | Connection error types |
| `ConnectionSystems` | `SystemSet` | System set enum for connection phases |
| `LinkSystems` | `SystemSet` | System set enum for link phases |

### 5.3 Transport & Channels

| Type | Kind | Description |
|------|------|-------------|
| `Transport` | `Component` | Holds all channel senders/receivers for an entity |
| `Channel` | `Trait` | Marker trait for channel types |
| `ChannelSettings` | `Struct` | `mode`, `send_frequency`, `priority` |
| `ChannelMode` | `Enum` | `UnorderedUnreliable`, `SequencedUnreliable`, `OrderedReliable(ReliableSettings)`, `UnorderedReliable(ReliableSettings)` |
| `ReliableSettings` | `Struct` | Reliable channel configuration |
| `ChannelRegistry` | `Resource` | Registry of all channels |
| `AppChannelExt` | `Trait` | `add_channel::<C>(settings) -> ChannelRegistration` |
| `PingChannel` | `Struct` | Default SequencedUnreliable channel for pings |
| `MetadataChannel` | `Struct` | Default UnorderedReliable channel for metadata |
| `ActionsChannel` | `Struct` | Default UnorderedReliable channel for entity actions |
| `UpdatesChannel` | `Struct` | Default SequencedUnreliable channel for component updates |

### 5.4 Messages

| Type | Kind | Description |
|------|------|-------------|
| `Message` | `Trait` | Auto-implemented for any `Serialize + Deserialize + 'static` type |
| `MessageManager` | `Component` | Manages all `MessageSender<M>` and `MessageReceiver<M>` on an entity. Holds `RemoteEntityMap`. |
| `MessageSender<M>` | `Component` | Buffers messages of type `M` for sending. Methods: `send::<C>(msg)`, `send_with_priority::<C>(msg, priority)` |
| `MessageReceiver<M>` | `Component` | Receives messages of type `M`. Methods: `receive()`, `receive_with_tick()`, `has_messages()`, `num_messages()` |
| `ServerMultiMessageSender` | `SystemParam` | Send messages from server to multiple clients: `send::<M, C>(msg, server, target)` |
| `EventSender<M>` | `Component` | Send triggers of type `M` remotely |
| `AppMessageExt` | `Trait` | `register_message::<M>() -> MessageRegistration`, `register_message_custom_serde::<M>()` |
| `AppTriggerExt` | `Trait` | `register_event::<M>()` |
| `RemoteEvent<M>` | `Event` | Bevy `Event` emitted when a remote message is received. Contains `trigger: M`, `from: PeerId` |
| `MessageRegistry` | `Resource` | Registry of all message types |

`MessageSender.on_add_hook` auto-inserts `MessageManager` (via `#[require(MessageManager)]`).
`MessageReceiver.on_add_hook` auto-inserts `MessageManager` and registers in its registry.

### 5.5 Replication

| Type | Kind | Description |
|------|------|-------------|
| `Replicate` | `Component` | Marks entity for replication. Requires `Replicating`, `ReplicationGroup`, `ReplicationState`. Methods: `to_clients(target)`, `to_server()`, `manual(senders)` |
| `Replicating` | `Component` | Marker: entity is actively being replicated (remove to pause) |
| `Replicated` | `Component` | Marker: entity was received from a remote peer. Field: `receiver: Entity` |
| `ReplicationSender` | `Component` | On server per-client link: manages sending replication updates. `new(interval, mode, bandwith_cap)` |
| `ReplicationReceiver` | `Component` | On client link: manages receiving replication |
| `ReplicationState` | `Component` | Per-entity metadata tracking senders, prediction/interpolation state, visibility |
| `PerSenderReplicationState` | Struct | Tracks per-sender state for an entity: `predicted`, `interpolated`, `visibility`, `authority`, `spawned` |
| `ReplicationMode` | `Enum` | `SingleSender`, `SingleClient`, `SingleServer(NetworkTarget)`, `Manual(Vec<Entity>)`, `Sender(Entity)` |
| `SendUpdatesMode` | `Enum` | `SinceLastAck` (resend until acked), `SinceLastSend` (only send changes) |
| `ReplicationGroup` | `Component` | Replication group assignment |
| `ReplicationGroupId` | `Component` | Per-sender group ID |
| `DEFAULT_GROUP` | `Constant` | Default replication group |
| `PREDICTION_GROUP` | `Constant` | Prediction replication group |
| `MAX_MESSAGE_SIZE` | `Constant` | Max single replication message size (with margin) |
| `ComponentRegistry` | `Resource` | Registry of all registered components |
| `ComponentRegistration<C>` | Struct | Builder for component options |
| `AppComponentExt` | `Trait` | `register_component::<C>()`, `register_component_custom_serde::<C>()` |
| `ComponentReplicationConfig` | `Struct` | Per-component replication configuration |
| `ComponentReplicationOverrides<C>` | `Component` | Per-entity overrides for component replication |
| `ComponentReplicationOverride` | `Struct` | Individual override: `send`, `receive`, prediction/interpolation flags |
| `DeltaManager` | `Component` | Manages old state for diffable components |
| `Diffable` | `Trait` | For components that can compute deltas between old/new state |
| `Confirmed<C>` | `Component` | Wraps the last confirmed value of `C` from the server (on predicted entities) |
| `ConfirmedTick` | `Component` | Tick at which the entity was last confirmed |
| `InitialReplicated` | `Component` | Marker: entity was initially spawned via replication |
| `Persistent` | `Component` | Marker: entity survives `ReplicationReceiver` disconnect |
| `Cached` | `Component` | Tracks last known component state for delta computation |
| `TransformLinearInterpolation` | `Component` | Tag for transform linear interpolation |
| `SpawnAction` | Internal | Spawn message in replication actions |
| `ActionsMessage` | Internal | Batched entity actions (spawn, despawn, insert, remove) |
| `UpdatesMessage` | Internal | Batched component update messages |

### 5.6 Control & Authority

| Type | Kind | Description |
|------|------|-------------|
| `Controlled` | `Component` | On receiver: entity is controlled by local peer |
| `ControlledBy` | `Component` | On sender: associates entity with a controlling `ReplicationSender`. Fields: `owner: Entity`, `lifetime: Lifetime` |
| `ControlledByRemote` | `Component` | On sender: lists entities controlled by remote. Relationship target of `ControlledBy` |
| `Lifetime` | `Enum` | `SessionBased` (despawn on disconnect) or `Persistent` (keep) |
| `HasAuthority` | `Component` | Current peer has authority over this entity |
| `AuthorityBroker` | `Component` | Tracks entity ownership per peer |
| `AuthorityTransfer` | `Component` | How entity behaves on authority request |
| `GiveAuthority` | `Event` | Emit to give authority to a remote peer |
| `RequestAuthority` | `Event` | Emit to request authority from current owner |
| `NetworkVisibility` | `Component` | Marker: interest management active |
| `NetworkVisibilityPlugin` | `Plugin` | Visibility-based replication culling |
| `Room` | `Resource` | Room data structure for interest management |
| `RoomEvent` | `Event` | Modify room membership |
| `RoomTarget` | `Enum` | Target entity or peer for room operations |
| `ReplicateLike` | `Component` | Entity should replicate like another entity |
| `ReplicateLikeChildren` | `Component` | Relationship target for `ReplicateLike` |
| `DisableReplicateHierarchy` | `Component` | Stop replicating children |

### 5.7 Prediction

| Type | Kind | Description |
|------|------|-------------|
| `Predicted` | `Component` | Marker: entity is client-predicted. Stores component history for rollback. |
| `PredictionManager` | `Component` | On client link: manages prediction state |
| `PredictionPlugin` | `Plugin` | Enables prediction + rollback systems |
| `PredictionRegistry` | `Resource` | Registry of predicted components |
| `PredictionHistory<C>` | Type alias | `HistoryBuffer` for component `C` (stores past states) |
| `PredictionRegistrationExt` | `Trait` | `add_prediction()` builder trait |
| `PredictionAppRegistrationExt` | `Trait` | `add_rollback::<C>()` for marking components for rollback |
| `RollbackPolicy` | `Component` | Controls rollback check/trigger behavior |
| `Rollback` | `Component` (Enum) | Marker inserted during rollback: `FromState` or `FromInputs` |
| `RollbackMode` | `Enum` | Rollback behavior mode |
| `is_in_rollback` | Function | Run condition: true if currently rolling back |
| `VisualCorrection` | `Component` | Smoothing configuration for rollback visual correction |
| `DisableRollback` | `Component` | Exclude entity from all rollback operations |
| `DisabledDuringRollback` | `Component` | `Disabled` variant inserted on `DisableRollback` entities during rollback |
| `DeterministicPredicted` | `Component` | Predicted but no rollback-from-state (deterministic mode) |
| `PredictionDisable` | `Component` | Disable prediction for an entity |
| `PredictionMetrics` | `Resource` | Client prediction metrics (flushed to diagnostics) |
| `LastConfirmedInput` | `Component` | Most recent confirmed input across all remote clients |
| `PredictionSystems` | `SystemSet` | System ordering sets for prediction |
| `RollbackSystems` | `SystemSet` | System ordering sets for rollback |
| `SyncComponent` | `Trait` | For components sync-able between confirmed and predicted/interpolated |

### 5.8 Interpolation

| Type | Kind | Description |
|------|------|-------------|
| `Interpolated` | `Component` | Marker: entity is visually interpolated |
| `InterpolationPlugin` | `Plugin` | Enables interpolation systems |
| `InterpolationRegistry` | `Resource` | Registry of interpolated components |
| `ConfirmedHistory<C>` | `Component` | Buffer of past confirmed values for interpolation |
| `InterpolationTimeline` | `Resource` | Timeline for interpolation |
| `InterpolationDelay` | `Struct` | Interpolation delay value (wraps `PositiveTickDelta`) |
| `InterpolationRegistrationExt` | `Trait` | `add_linear_interpolation()` builder |
| `interpolation_fraction` | Function | Compute the current interpolation `t` value (0..1) |
| `SyncComponent` | `Trait` | For components that can be interpolated |

### 5.9 PreSpawned

| Type | Kind | Description |
|------|------|-------------|
| `PreSpawned` | `Component` | Marks client-prespawned entity. Fields: `hash`, `user_salt`, `receiver`. Methods: `new(hash)`, `default()`, `default_with_salt(salt)`, `for_receiver(entity)` |
| `PreSpawnedReceiver` | `Component` | On client link: tracks prespawned entity hashes for matching |
| `PreSpawnedSystems` | `SystemSet` | `CleanUp` — cleanup unmatched prespawned entities |

### 5.10 Timeline & Tick

| Type | Kind | Description |
|------|------|-------------|
| `Tick` | `Struct` | Wraps `u16` with wrapping arithmetic |
| `TickDuration` | `Resource` | Duration of one tick |
| `TickInstant` | `Struct` | A tick with fractional overstep |
| `TickDelta` | `Struct` | Signed difference between ticks (can be negative) |
| `PositiveTickDelta` | `Struct` | Non-negative tick difference |
| `Overstep` | `Struct` | Fractional progress within the current tick (0..1, serialized as `U0F16`) |
| `LocalTimeline` | `Resource` | Incremented every `FixedUpdate`. Method: `tick() -> Tick`, `apply_delta(delta)` |
| `Timeline<T>` | `Struct` | Generic timeline with `now: TickInstant` and config context |
| `TimelineConfig` | `Trait` | Configuration for a timeline |
| `NetworkTimeline` | `Trait` | Methods: `now()`, `tick()`, `overstep()`, `set_now()`, `apply_delta()` |
| `SyncEvent<T>` | `Event` | Sync trigger: applies `tick_delta` to a target timeline |
| `IsSynced` | `Component` | Marker: timeline has been synced |
| `InputTimeline` | `Component` | Client input buffer timeline |
| `InputTimelineConfig` | `Component` | Input timeline configuration |
| `SetTickDuration` | `Event` | Update tick duration at runtime |
| `SyncConfig` | `Component` | Configuration for timeline sync |

### 5.11 Input (Native / Leafwing / BEI)

| Type | Feature | Description |
|------|---------|-------------|
| `InputPlugin<A>` | any input | Plugin to register input type `A` |
| `InputTimeline` | any input | Tracks input buffer timing |
| `InputTimelineConfig` | any input | Input timing configuration |
| `InputDelayConfig` | `client` | Input delay configuration |
| `InputRegistryExt` | `input_bei` | Register BEI input types |
| `InputMap<A>` | `leafwing` | Leafwing bindings-to-actions map on controlled entities |
| `InputMessage` | each backend | Wraps input for network transmission |

### 5.12 Utility & Types

| Type | Kind | Description |
|------|------|-------------|
| `PeerId` | `Enum` | `Local(u16)`, `Server`, `Netcode(u64)`, `Steam(u64)` |
| `NetworkDirection` | `Enum` | `ClientToServer`, `ServerToClient`, `Bidirectional` |
| `NetworkTarget` | Type alias | `Target<PeerId>` |
| `EntityTarget` | Type alias | `Target<Entity>` |
| `Target<T>` | `Enum` | `None`, `All`, `Single(T)`, `AllExceptSingle(T)`, `AllExcept(Vec<T>)`, `Only(Vec<T>)` |
| `RemoteEntityMap` | `Struct` | Maps remote entity IDs to local entities |
| `SendEntityMap` | `Struct` | Maps local entities to remote IDs |
| `ReceiveEntityMap` | `Struct` | Maps remote IDs to local entities |
| `SendEntityMap` | `Component` | On link: maps sent entities |
| `PriorityConfig` | `Struct` | Bandwidth priority configuration |
| `PriorityManager` | `Struct` | Bandwidth manager for message sending |
| `PingConfig` | `Component` | Ping/RTT measurement configuration |
| `PingManager` | `Component` | Sends pings, measures RTT and jitter |
| `Ping` | Message | Ping request |
| `Pong` | Message | Ping response |
| `DrivingTimeline` | `Component` | Marker: timeline drives the Bevy app |
| `Seek` / `SeekFrom` | Trait/Enum | Byte stream seeking for serialization |

---

## 6. Common Patterns

### 6.1 Client Setup (Crossbeam / In-Process)

```rust
let (client_io, server_io) = CrossbeamIo::new_pair();

commands.spawn((
    Client::default(),
    LocalId(PeerId::Local(1)),
    RemoteId(PeerId::Server),
    Connected,
    Link::default(),
    Linked,
    client_io,
    Transport::default(),
    MessageManager::default(),
    ReplicationReceiver::default(),
    PredictionManager::default(),
));
```

Two `client.update()` calls after spawning before first schedule run (seen in every example).

### 6.2 Server Setup (Crossbeam)

```rust
commands.spawn((Server::default(), Started));

// Handle new client connection:
fn on_new_client(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands.entity(trigger.entity).insert((
        ReplicationSender::new(
            Duration::from_millis(100),
            SendUpdatesMode::SinceLastAck,
            false,
        ),
        ClientOf,
        MessageReceiver::<GameplayCommand>::default(),
        MessageSender::<ServerMessage>::default(),
    ));
}
```

### 6.3 Message Send/Receive

**Send (client or server per-link)**:
```rust
fn send_system(mut sender: Single<&mut MessageSender<Msg1>>) {
    sender.send::<MyChannel>(Msg1(42));
}
```

**Server broadcast**:
```rust
fn server_broadcast(mut sender: ServerMultiMessageSender, server: Single<&Server>) {
    sender.send::<Msg1, MyChannel>(&Msg1(42), &server, &NetworkTarget::All);
}
```

**Receive**:
```rust
fn recv_system(mut receiver: Single<&mut MessageReceiver<Msg1>>) {
    for msg in receiver.receive() {
        info!("Got: {:?}", msg);
    }
}
```

### 6.4 Component Replication

```rust
// Server spawns:
commands.spawn((
    PlayerPosition(Vec2::ZERO),
    PlayerColor(Color::RED),
    Replicate::to_clients(NetworkTarget::All),
    PredictionTarget::to_clients(NetworkTarget::Single(client_id)),
    ControlledBy { owner: trigger.entity, lifetime: Lifetime::SessionBased },
));

// Client observer adds local-only components:
fn on_predicted(trigger: On<Add, Predicted>, mut commands: Commands) {
    commands.entity(trigger.entity).insert((
        PhysicsBundle::default(),
        InputMap::<PlayerInputs>::default(),
    ));
}

// Client prediction system:
fn predicted_movement(
    mut query: Query<(&mut PlayerPosition, &ActionState<PlayerInputs>), With<Predicted>>,
) {
    for (mut pos, input) in query.iter_mut() {
        if input.pressed(GameAction::MoveRight) {
            pos.0.x += 1.0;
        }
    }
}
```

### 6.5 PreSpawned Lifecycle

```rust
// CLIENT: spawn predicted entity immediately
let hash = calculate_hash(player_id, sequence);
commands.spawn((
    Projectile { speed: 100.0 },
    Position(Vec2::ZERO),
    PreSpawned::new(hash).for_receiver(client_link),
));

// SERVER: spawn with same hash
commands.spawn((
    Projectile { speed: 100.0 },
    Position(Vec2::ZERO),
    PreSpawned::new(hash),
    Replicate::to_clients(NetworkTarget::All),
    PredictionTarget::to_clients(NetworkTarget::All),
));
```

When the server's replicated entity arrives, Lightyear auto-matches by hash:
- Removes `PreSpawned` on client entity
- Merges server-authoritative component data
- If no match found within `PreSpawnedReceiver` timeout (~60-70 ticks), client entity is despawned

**Important (from source)**: `PreSpawned` must be spawned in `FixedMain`. The hash is computed by a component hook (`on_add`) on the insertion tick, based on the entity's archetype (list of components) at that time.

### 6.6 Avian Physics Integration

```rust
app.add_plugins(LightyearAvianPlugin {
    replication_mode: AvianReplicationMode::Position,
    ..default()
});
app.add_plugins(
    PhysicsPlugins::default()
        .build()
        .disable::<PhysicsTransformPlugin>()
        .disable::<PhysicsInterpolationPlugin>(),
);
```

`AvianReplicationMode`:
- `Position` — replicate `Position`, prediction/correction on `Position`
- `PositionButInterpolateTransform` — replicate `Position`, interpolate on `Transform`
- `Transform` — replicate `Transform` directly

### 6.7 Lag Compensation (lightyear_avian)

```rust
app.add_plugins(LagCompensationPlugin);

fn raycast(
    lag_query: LagCompensationSpatialQuery,
    mut query: Query<&mut Health>,
) {
    let hit = lag_query.cast_ray(
        InterpolationDelay { delay: PositiveTickDelta::lit("3") },
        origin, direction, max_dist, true, &mut filter,
    );
}
```

Entities need `LagCompensationHistory::default()` to participate. The `InterpolationDelay` specifies how far back in time to query (in ticks).

---

## 7. Test-Backed Usage

### From lightyear_replication tests

The `lightyear_replication::registry::registry::AppComponentExt` test shows registration:
```rust
app.register_component::<MyComponent>();
```

The `lightyear_connection::network_target::tests` unit tests cover `Target` intersection, union, exclusion, and serde round-trips for all `NetworkTarget` variants.

The `lightyear_core::tick::tests::test_shared_atomic_tick_minimum` tests `AtomicTick::set_if_lower` for concurrent minimum tracking.

### From lightyear_replication `prespawn.rs`

The `PreSpawned` hook (`register_prespawn_hashes` observer on `On<Add, PreSpawned>`) is tested through the prespawn flow:
1. Observer fires when `PreSpawned` is added to a non-`Replicated` entity
2. Computes hash from archetype + tick if not already set
3. Registers in `PreSpawnedReceiver`
4. On replication receive, matches by hash

### From Afterglow's own tests

- `lightyear.rs`: Manual replication with `CrossbeamIo`, manual `Transport` channel registration, component sync via `sync_component_set`
- `physics_grab.rs`: Full PreSpawned + prediction + avian pipeline with crossbeam transport
- `prototype-physics-lightyear`: Avian rollback mechanics test

---

## 8. Gotchas & Footguns

1. **Protocol registration ordering**: Protocol (channels, messages, components) MUST be added AFTER `ClientPlugins`/`ServerPlugins` but BEFORE spawning any `Client` or `Server` entity. Violating this causes desync between client and server because the registries are finalized during entity construction.

2. **Crossbeam Transport setup**: When using `CrossbeamIo` (netcode disabled), each channel that will be used must manually register senders/receivers:
   ```rust
   let mut transport = Transport::default();
   transport.add_sender_from_registry::<MyChannel>(&registry);
   transport.add_receiver_from_registry::<MyChannel>(&registry);
   ```
   This is required because netcode's automatic channel wiring is skipped.

3. **Two `client.update()` calls**: After spawning the client entity, call `client.update()` twice before running any schedule. This is a consequence of Lightyear's deferred initialization and appears in every crossbeam example.

4. **`InputMap<T>` for leafwing inputs**: The Leafwing `InputMap<T>` component must be added to client-controlled entities via an `On<Add, Predicted>` or equivalent observer to activate the Leafwing input buffer.

5. **No `RigidBody` on interpolated entities**: Adding `RigidBody` (avian) to interpolated entities causes `Transform::default()` to spawn before the first interpolation value. Add physics bundles in observers that check `With<Predicted>` (not interpolated).

6. **`add_linear_interpolation()` requires `Ease` trait**: For components to use built-in linear interpolation, they must implement `Ease`. For custom interpolation without `Ease`, use `add_linear_correction_fn()` instead.

7. **`add_should_rollback(fn)` for threshold-based rollback**: Without this, every server update triggers a rollback even for tiny floating-point differences. For position components, a threshold of ~0.01 is recommended.

8. **`Confirmed<T>` is NOT the component**: On predicted entities, the server-received authoritative value lives in `Confirmed<T>`, NOT in the raw component slot. Systems reading the "live" value read the predicted component directly. Systems that reconcile should read `Confirmed<T>` and copy to the predicted component.

9. **PreSpawned timeout**: Defaults to ~60-70 ticks (source: `PreSpawnedReceiver`). Unmatched prespawned entities are cleaned up. To force expiration (e.g. in tests), advance the local timeline past the timeout.

10. **`#[require(Component)]` generates RequiredComponents**: In Bevy 0.18+, `#[require(Replicating)]` on `Replicate` means inserting `Replicate` automatically inserts `Replicating`. Similarly `#[require(MessageManager)]` on `MessageSender<M>`. This is automatic but can cause unexpected component dependencies.

11. **HostServer mode**: When both client and server run in the same process, server-side systems MUST check for `Has<Predicted>` to avoid double-applying inputs to predicted entities. The `HostServerPlugin` handles some of this, but custom server systems also need this guard.

12. **`Transport` is required by `ReplicationSender`**: The `#[require(Transport)]` on `ReplicationSender` means a `Transport` component is auto-inserted when `ReplicationSender` is added. If using crossbeam, this auto-inserted `Transport` may not have the right channel senders/receivers configured.

13. **`Tick` wraps `u16`**: Ticks use wrapping arithmetic. At 60 ticks/sec, the tick wraps around every ~18 minutes. All tick comparison logic must handle wrapping (Lightyear does this internally via `wrapping_diff`).

14. **`MessageReceiver<M>` is auto-cleared**: Messages are drained from the `recv` buffer every frame in the `Last` schedule. If you don't call `receive()` in your system, messages are silently dropped.

15. **`ProtocolCheckPlugin` verifies client/server protocol match**: The server sends a checksum of the registries when a client connects. If they don't match, the client gets an error. The relevant observers are added in `finish()` but currently commented out in the default plugin build (source: `lightyear/src/protocol.rs:83-84` — they are theoretically added but the observers are `add_observer` called in `finish`).

16. **`Replicate` automatically requires `ReplicationState`**: The `Replicate` component has `#[require(ReplicationState)]`, so a `ReplicationState` is auto-inserted. This `ReplicationState` tracks per-sender metadata and is critical for correct replication behavior.

17. **`PreSpawned` hash computation**: The hash is computed via a component hook (`on_add`) on the same tick `PreSpawned` is inserted. The default hasher uses the entity's archetype (list of components at that tick) AND the spawn tick. If you manually set a hash via `PreSpawned::new(hash)`, the default hasher is bypassed. Custom hashes are strongly recommended for deterministic matching.

18. **`.for_receiver(entity)` scoping**: If `receiver` is `None`, Lightyear uses the single entity with `PreSpawnedReceiver` (typically the client link). Setting a specific receiver allows multiple client links to independently prespawn entities.

---

## 9. Integration with Afterglow

### Current usage mapping

| Afterglow | Lightyear API | Notes |
|-----------|---------------|-------|
| `CrossbeamIo` for in-process transport | `lightyear_crossbeam` | Standard for testing |
| `register_protocol` with channels + messages + components | `AppChannelExt`, `AppMessageExt`, `AppComponentExt` | Matches simple_box/simple_setup patterns |
| `Replicate` + `PredictionTarget` on server-spawned entities | `Replicate::to_clients()`, `PredictionTarget::to_clients()` | Standard pattern |
| `PreSpawned::new(hash).for_receiver(receiver)` | `PreSpawned` | Matches physics_grab pattern |
| Observers on `On<Add, Predicted>` / `On<Add, Controlled>` | Observer pattern | Standard Lightyear 0.26 |
| `Confirmed<T>` for reading authoritative state | `Confirmed<Component>` | Standard pattern |
| `reconcile_confirmed_constraints` | Read `Confirmed<T>`, write to predicted | Standard reconciliation |
| `LagCompensationHistory` + `LagCompensationSpatialQuery` | `lightyear_avian` | Lag compensation |
| Manual link construction with `MessageSender`/`MessageReceiver` | `simple_setup` pattern | Works without ClientPlugins auto-setup |
| Manual timeline advancing in tests | `LocalTimeline::apply_delta()` | Standard in-memory testing |

### Key differences from canonical patterns

1. **Native Leafwing input is the primary player-input path**: Afterglow installs `lightyear_inputs_leafwing::InputPlugin::<AfterglowAction>`, writes local desired input in `FixedPreUpdate` / `InputSystems::WriteClientInputs`, and reads `ActionState<AfterglowAction>` in fixed gameplay. Do not register `ActionState` as a normal replicated component for player commands. The older manual `MessageSender<ActionState<AfterglowAction>>` path is retained only as a regression comparison.

2. **Player actions do not get parallel gameplay intents**: Interactions such as rope attach/detach are derived from `ActionState` in fixed simulation on both the server and owning predicted client. Target ids, rope ids, and hit results are outcomes in replicated gameplay components, not client-sent command payloads and not fields inside `ActionState`.

3. **Netcode peer ids are handshake-owned**: Direct UDP/netcode client entities are spawned without `LocalId`, `RemoteId`, or `Connected`; Lightyear inserts those after a successful netcode handshake. Crossbeam/in-process test links may still use explicit ids because there is no handshake.

4. **Manual `Transport` channel setup**: Afterglow correctly calls `transport.add_sender_from_registry()` and `transport.add_receiver_from_registry()` for each game channel. For native input, the Leafwing input plugin registers `InputChannel`; transports only add it from the registry if present.
