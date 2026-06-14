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
| `AfterglowNetworkPlugin` | Adds `AfterglowLightyearPlugin` and `AfterglowSessionPlugin`. |
| `AfterglowLightyearPlugin` | Initializes `AfterglowLightyearConfig`; with the `lightyear` feature, adds Lightyear client/server plugin groups and Leafwing input networking. Concrete link/transport entity setup is still test/demo-owned. |
| `AfterglowLightyearConfig` | Engine-facing Lightyear config: role, server/remote addresses, tick rate, prediction window, protocol id, optional connect token, link-conditioner settings, and `netcode_private_key`. The private key defaults to `[0u8; 32]` — a development placeholder that **must** be replaced before any real network deployment. |
| `register_afterglow_lightyear_protocol` | Opt-in helper that initializes `HistoryTick`, registers reflection for `HistoryTick` and `StableEntityId`, and — under the `lightyear` feature — registers `StableEntityId` as a replicated Lightyear component. Call it after Lightyear client/server plugins are present. |
| `HistoryTick` | Plain `u32` resource used by deterministic fixed-step tests and scenario systems. It is not rewind history. |
| `AfterglowSessionPlugin` | Platform-neutral session/matchmaking API layer. Registers `AfterglowSessionState`, `SessionStatus`, `NonSteamSessionCatalog`, `SessionRequest` and `SessionEvent` Bevy `Message` types, and systems that process session requests and maintain a session status snapshot. Future Steam lobby support will be added as an alternative backend. |
| `AfterglowSessionState` | Resource tracking the local `SessionMemberId`, current `SessionId`, and current `SessionBackend` (if any). |
| `SessionStatus` | Game-friendly snapshot derived from `SessionEvent`s: current `SessionInfo`, observed member list, and coarse connection state. |
| `SessionConnectionState` | `Idle`, `Joining`, `Connected`, `Error(SessionError)`. |
| `AfterglowSessionExt` | Fluent extension trait on `App` (`app.session()`) providing the high-level API in [`session-api.md`](session-api.md). |
| `AfterglowSessionLightyearBridgePlugin` | Opt-in bridge that maps `SessionEvent`s to Lightyear link lifecycle. |
| `AfterglowNetcodeConsumerPlugin` | Opt-in consumer that drains `PendingNetcodeStartup` and spawns real UDP/netcode link entities. |

## Universal Identity

`StableEntityId` is the only engine-level durable entity ID source. It is used
for persistence, durable identity on replicated entities, and cross-peer gameplay
references. Lightyear still owns live network entity remapping for the current
session.
Raw Bevy `Entity` values are local handles only and must not appear in network
payloads, save data, or cross-peer gameplay references.

Systems that need durable identity should require or insert `StableEntityId`.
Entities marked `RuntimeOnly` are excluded from automatic stable-ID assignment in
helpers that perform that pass. The allocator skips IDs already authored in the
world, so generated IDs do not collide with scene, persistence, or network IDs.

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

Use Lightyear's existing `ControlledBy` / `Controlled` relationship to bind an
entity to the link that controls it, and Leafwing `InputMap<AfterglowAction>` /
`ActionState<AfterglowAction>` on the controlled entity for gameplay input.
Afterglow does not add a separate player/avatar taxonomy.

The missing reusable layer is controlled-entity lifecycle orchestration: assign,
revoke, and rebind controlled entities for join, disconnect, respawn,
possession, and reconnect. It is not a custom action transport or entity-routing
protocol.

## Input Pattern

Networked input is entity-scoped Leafwing state:

```rust
app.add_plugins(lightyear_inputs_leafwing::InputPlugin::<AfterglowAction>::default());
```

Gameplay reads `ActionState<AfterglowAction>` in fixed schedules. Afterglow no
longer serializes custom command DTOs for movement/combat input.

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

## Session / Matchmaking API

The session layer is a platform-neutral wrapper for matchmaking operations
(create, search, join, leave) backed at runtime by a chosen provider. Both
`SessionRequest` and `SessionEvent` are Bevy `Message` types (not Lightyear
messages).

| Type | Purpose |
|---|---|
| `SessionId` / `SessionMemberId` | `u128` wrapper IDs with `INVALID`/`new`/`from_raw`/`as_raw`/`is_valid`, following the `StableEntityId` pattern. `SessionId` is internal/durable; players do not type it. |
| `SessionCode` | Player-facing short join token (e.g., `XFQ-KRB`). Six uppercase characters in two hyphenated groups, excluding I/L/O. Generated by the non-Steam provider and unique among active sessions. |
| `PlayerIdentity` | Durable anti-spoofing identity: `Native(NativeIdentityProof)` with Ed25519 public key + signature, or `Steam { steam_id, ticket }` for Steam ticket passthrough. |
| `NativeIdentityProof` | Ed25519 public key and signature over a canonical challenge (`"afterglow-session:" + backend + target + nonce`). The client holds the private key locally; the server never sees it. |
| `SessionIdentityNonce` | Server-side nonce resource used when verifying native proofs. Initialized from the OS CSPRNG. |
| `IdentityError` | `InvalidPublicKey`, `InvalidSignature`, `UnsupportedBackend`. |
| `SessionBackend` | `NonSteam` or `Steam` — identifies which provider the session came from. |
| `SessionVisibility` | `Private` / `FriendsOnly` / `Public` / `Invisible` — maps to Steam lobby visibility. |
| `SessionTransport` | `Local` or `Netcode` — describes the expected transport without owning a Lightyear link. `DirectUdp` was removed in favor of `Netcode` plus an explicit provider endpoint. |
| `ProviderEndpoint` | `InProcess`, `Udp(SocketAddr)`, or `Steam`. A remote client must supply a `Udp` endpoint to find/join a NonSteam listen-server because there is no central registry. `Steam` resolves through Steamworks. `InProcess` routes requests through the in-process catalog for local co-op/tests. |
| `SessionConfig` | Name, `backend`, capacity, visibility, metadata (key/value), and transport preference. `SessionConfig::default()` targets `NonSteam`. |
| `SessionSearch` | `backend`, `provider`, metadata filter (exact AND semantics), `require_open_slot`, and `max_results` bound (0 returns empty). |
| `SessionInfo` | Full read-only snapshot: id, code, backend, name, owner, owner identity, member count/capacity, visibility, metadata, transport. |
| `SessionRequest` | Bevy `Message` enum: `Create(SessionConfig, PlayerIdentity)`, `Search(SessionSearch)`, `Join { backend, session, identity, provider }`, `JoinByCode { backend, provider, code, identity }`, `Leave`. `Join` and `JoinByCode` carry a `provider` so remote joiners can reach the host. `Search` carries a `provider` for the same reason. |
| `SessionEvent` | Bevy `Message` enum carrying session lifecycle outcomes: `Created`, `SearchResults`, `Joined`, `Left`, `MemberJoined`, `MemberLeft`, `SessionEnded`, `Error`. |
| `SessionError` | `AlreadyInSession`, `NotInSession`, `SessionNotFound`, `SessionFull`, `InvalidConfig`, `PermissionDenied`, `BackendUnavailable`. |
| `SessionLeaveReason` | `Left`, `Disconnected`, `Kicked`, `Banned`, `HostEnded`. |
| `AfterglowSessionState` | Resource with `local_member_id`, `identity`, `current_session`, `current_backend`. `local_member_id` defaults to `INVALID` and is lazily allocated by the catalog on first use. |

Each `SessionRequest` carries a `backend` field (directly in `SessionConfig.backend`,
`SessionSearch.backend`, or `Join.backend`). The non-Steam provider processes
only `NonSteam` requests; `Steam` requests emit `BackendUnavailable`. This
design avoids duplicate provider state — a future Steam provider will handle
Steam-targeted requests without shadowing the non-Steam catalog.

The **in-process non-Steam provider** (`NonSteamSessionCatalog` + `process_non_steam_session_requests`) is the
default in-memory implementation. It validates all operations, enforces capacity
and ownership, generates a unique [`SessionCode`] for each created session, and
emits the outcome protocol locally.

The **networked non-Steam provider** (`NonSteamSessionProvider` + `NonSteamSessionClient`) lets a
host listen on a TCP address and exposes the same catalog operations to remote
clients over a length-prefixed postcard protocol. The provider shares the
`NonSteamSessionCatalog` resource with the in-process system: a listen-server
host can create a session via the normal in-process path and remote clients
querying the provider will see it. Provider responses are sent back to the
requester over TCP, and `MemberJoined`/`MemberLeft`/`SessionEnded` events are
also emitted locally so the hosting app observes membership changes.

Games usually send remote `Search`/`JoinByCode` requests through the
`NonSteamSessionClient` resource (or via the [`AfterglowSessionExt`](session-api.md)
high-level API), and then read `SessionStatus` or `AfterglowSessionState` for the
result. Neither provider creates Lightyear transport links, Netcode connections,
or lobby network traffic by itself.

### Joining by Code

To join a friend's session, use the player-facing code emitted in
[`SessionEvent::Created`] / [`SessionEvent::Joined`]:

```rust
app.world_mut().write_message(SessionRequest::JoinByCode {
    backend: SessionBackend::NonSteam,
    provider: ProviderEndpoint::Udp("203.0.113.42:7777".parse().unwrap()),
    code: SessionCode::new("XFQ-KRB"),
    identity: my_identity(),
});
```

Joining by internal [`SessionId`] remains available via
`SessionRequest::Join { backend, session, identity }` for systems that already
track the durable ID.

### Player Identity

Every create/join request must carry a [`PlayerIdentity`]:

```rust
let identity = PlayerIdentity::Native(NativeIdentityProof {
    public_key: my_public_key.to_vec(),
    signature: my_signing_key.sign(&NativeIdentityProof::challenge(
        SessionBackend::NonSteam,
        "create",
        &nonce.0,
    )).to_vec(),
});

app.world_mut().write_message(SessionRequest::Create(
    SessionConfig::default(),
    identity,
));
```

The non-Steam provider:

- verifies the Ed25519 signature against the canonical challenge for the
  request target (`"create"`, the session id for `Join`, or the session code for
  `JoinByCode`);
- rejects requests that fail verification with `SessionError::PermissionDenied`;
- binds the public key to a `SessionMemberId` so the same native key rejoining a
  session gets the same member slot;
- accepts `PlayerIdentity::Steam` as a passthrough; the future Steam backend will
  validate the Steam ticket.

`SessionMemberId` remains a per-session handle. `PlayerIdentity` is the trusted
anti-spoofing/persistence boundary.

**Future Steam mapping**: `SessionId` / `SessionMemberId` will wrap
`steamworks::LobbyId` / `steamworks::SteamId`. `SessionRequest` messages
will map to `ISteamMatchmaking` calls, and `SessionEvent` messages will deliver their
completion callbacks. `SessionVisibility` maps directly to Steam lobby type.
Metadata parity with Steam key/value lobby data is already present.

The session layer does **not** own Lightyear transport, link spawning, or
`StableEntityId` gameplay identity. Session member IDs are platform/session
identity only. Clients send input through Leafwing/Lightyear regardless of
session membership.

### Provider Types in the Plugin

| Type | Purpose |
|---|---|
| `NonSteamSessionProvider` | Optional resource. When inserted with a TCP listen address, it accepts remote clients and runs `NonSteamSessionCatalog` operations on their behalf. Defaults disabled. |
| `NonSteamSessionClient` | Always-registered resource. Used by games to send `SessionRequest`s to a remote `ProviderEndpoint::Udp(addr)`. Manages a single outbound TCP connection at a time and writes remote responses as local `SessionEvent` messages. |
| `RemoteClient` | Internal per-connection state stored in `NonSteamSessionProvider::clients`. |

## Querying Session Status

`AfterglowSessionPlugin` registers a [`SessionStatus`] resource that is kept in
sync with session events. Query it instead of draining [`SessionEvent`]
messages when you only need the current snapshot:

```rust
let state = app.world().resource::<AfterglowSessionState>();
if state.is_in_session() {
    let status = app.world().resource::<SessionStatus>();
    println!("session: {:?}", status.info);
    println!("members: {}", status.member_count());
    match status.state {
        SessionConnectionState::Idle => {}
        SessionConnectionState::Joining => {}
        SessionConnectionState::Connected => {}
        SessionConnectionState::Error(err) => eprintln!("session error: {:?}", err),
    }
}
```

`SessionStatus` is also the natural hook for higher-level helpers such as the
API proposed in [`session-api.md`](session-api.md) (e.g. `app.session().status()`).

## Session-to-Lightyear Bridge

[`AfterglowSessionLightyearBridgePlugin`] maps [`SessionEvent`] lifecycle
events to Lightyear link management. It is opt-in and not included in
[`AfterglowNetworkPlugin`]. Add it explicitly after Lightyear plugins and the
session plugin:

```rust
app.add_plugins((
    AfterglowLightyearPlugin,
    AfterglowSessionPlugin,
    AfterglowSessionLightyearBridgePlugin, // opt-in
));
```

| Plugin / Type | Purpose |
|---|---|
| `AfterglowSessionLightyearBridgePlugin` | Initializes `SessionLightyearLinks` and `PendingNetcodeStartup` resources; runs its private bridge system in [`AfterglowSessionSet::ApplyEffects`]. Lightyear may observe newly spawned link entities on the following frame depending on its own internal `PreUpdate` ordering. |
| `SessionLightyearLinks` | Resource tracking the client link, server link, and server entity spawned for a local session. Cleared on leave/session-end. |
| `PendingNetcodeStartup` | Resource carrying optional `NetcodeClientParams` and `NetcodeServerParams` produced by `DirectUdp` session events. [`AfterglowNetcodeConsumerPlugin`] drains this and spawns real UDP/netcode link entities. |
| `NetcodeClientParams` | Parameters for starting a Netcode client connection: `server_addr`, `client_id`, `protocol_id`, `private_key`. The `private_key` is populated from `AfterglowLightyearConfig.netcode_private_key`. |
| `NetcodeServerParams` | Parameters for starting a Netcode server: `bind_addr`, `protocol_id`, `private_key`. The `private_key` is populated from `AfterglowLightyearConfig.netcode_private_key`. |

### Netcode Link Consumer

`AfterglowNetcodeConsumerPlugin` is an opt-in plugin that drains
`PendingNetcodeStartup` each frame and spawns Lightyear `NetcodeClient` /
`NetcodeServer` link entities with UDP transport. Add it when you want
`SessionTransport::DirectUdp` sessions to open real sockets:

```rust
app.add_plugins((
    AfterglowLightyearPlugin,
    AfterglowSessionPlugin,
    AfterglowSessionLightyearBridgePlugin,
    AfterglowNetcodeConsumerPlugin,
));
```

The consumer is separate so tests and headless scenarios can inspect
`PendingNetcodeStartup` without opening sockets, and so games can replace it
with custom transport logic if needed.

### Transport Behaviour

| `SessionTransport` | Bridge Action |
|---|---|
| `Local` | Despawns any previously tracked entities, then spawns a server entity (`Server::default()`, `Started`), a client link entity with Crossbeam transport, and a server link entity with Crossbeam transport. The client link carries `Client`, `LocalId`, `RemoteId(PeerId::Server)`, `Connected`, `Link`, `Linked`, `CrossbeamIo`, `Transport`, `MessageManager`, `ReplicationReceiver`, and `PredictionManager`. The server link carries `LinkOf { server }`, `ClientOf`, `LocalId(PeerId::Server)`, `RemoteId`, `Connected`, `Link`, `Linked`, `CrossbeamIo`, `Transport`, `MessageManager`, and `ReplicationSender::new(Duration::ZERO, SendUpdatesMode::SinceLastAck, false)`. |
| `Netcode` | Looks up the remote provider endpoint from `SessionInfo` (currently encoded in metadata). Writes `NetcodeClientParams` to `PendingNetcodeStartup` when the local `SessionMemberId` is a valid nonzero `u64`. If `AfterglowLightyearConfig.role` is `Host` or `Server`, also writes `NetcodeServerParams`. The `private_key` field in both param structs is populated from `AfterglowLightyearConfig.netcode_private_key` (default `[0u8; 32]` — a development placeholder that must be replaced before any real network deployment). If the provider endpoint cannot be resolved, pending state is cleared and no panic occurs. No link entities are spawned — a separate consumer should drain `PendingNetcodeStartup`. |

### Session Event Handling

| Event | Action |
|---|---|
| `Created` / `Joined` (Local) | Spawn tracked Crossbeam link entities (idempotent). |
| `Created` / `Joined` (Netcode) | Write pending netcode startup parameters. |
| `Left` / `SessionEnded` | Despawn tracked entities, clear pending startup. |
| `SearchResults`, `MemberJoined`, `MemberLeft`, `Error` | Ignored. |

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

## See Also

- [`session-api.md`](session-api.md) — proposed simple public API for hosting
  and joining sessions (Local, NonSteam listen-server, Steam).
- [`session-workflows.md`](session-workflows.md) — end-to-end Steam, NonSteam
  listen-server, and Local session workflows, plus identity and rejoin behavior.
