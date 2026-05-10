# Steam Multiplayer Research

## TLDR

Steam should be treated as a platform backend, not as the engine's multiplayer
architecture. It can provide Steam identity, ownership checks, friends/invites,
lobbies, NAT traversal, encrypted packet transport, and Steam Datagram Relay
(SDR). Afterglow should still own the gameplay protocol: player commands,
authoritative simulation, prediction, snapshots, interest management, stable
entity IDs, and save/load semantics.

The practical path is:

1. Keep a transport-agnostic networking layer in the engine.
2. Build and test replication against loopback and fake transports first.
3. Add Steam as an optional native feature/backend for lobby discovery,
   platform identity, auth, and `ISteamNetworkingSockets` transport.
4. Keep Iroh as a separate non-Steam transport option, because Steam is not a
   universal runtime and the Steamworks API is tied to Steam platform setup.

For the open-world immersive sim target, prefer authoritative server plus
client prediction. Full rollback via GGRS is useful research and may fit small
deterministic subsystems, but it should not become the baseline model for the
whole streaming RPG world.

## Scope

"SteamMultiplayer" can mean different things in different engines. For this
engine, the relevant Steamworks pieces are:

- `ISteamMatchmaking`: lobby creation, search, membership, invites, lobby
  metadata, and lobby owner state.
- `ISteamNetworkingSockets`: connection-oriented game transport with reliable
  and unreliable messages.
- `ISteamNetworkingMessages`: more UDP-like peer messaging without explicit
  connection handles.
- Steam Datagram Relay: Valve's relay/backbone service for P2P and dedicated
  server traffic.
- Steam authentication: Steam IDs, session tickets, Web API validation, and
  encrypted app tickets.
- Rust integration crates: `steamworks` and `bevy_steamworks`.

This note does not design Steam inventory, achievements, workshop, cloud saves,
or Steam Audio.

## What Steam Provides

Steam's multiplayer stack is best understood as four separate services.

### Identity And Ownership

Steam users have stable 64-bit Steam IDs. Steamworks authentication APIs can
prove that a connecting user owns the app and is who they claim to be. For
client-to-client or client-to-game-server flows, Steam session tickets are the
standard path. For a trusted backend, the client can request a Web API ticket
and the server can validate it against Steam's Web API. Encrypted application
tickets are another backend option when the secure server should not depend on
Steam Web API availability.

Engine implication: map Steam ID to an engine-level `NetworkPlayerId` or account
record, then map that to stable player entity IDs. Do not make Steam ID the ECS
entity ID or save-game entity ID.

### Lobbies And Session Discovery

Steam lobbies are shared rooms with membership, owner, metadata, chat/messages,
invites, and friend join support. They are not a replication layer and they do
not replace a game server. A lobby can advertise enough metadata to decide
whether a client should join a listen server, P2P host, or dedicated server.

Engine implication: use lobbies for discovery and launch handoff only:

- session name/version/build hash
- max players/current players
- world seed or world shard id
- host Steam ID or dedicated server routing payload
- network protocol version
- mod set or content hash
- privacy state

The simulation should continue to work without lobbies via LAN, direct dev
connection, Iroh tickets, or local loopback.

### Packet Transport

`ISteamNetworkingSockets` is the main modern transport API. It is connection
oriented like TCP but message oriented like UDP. It supports reliable and
unreliable message delivery, fragmentation/reassembly, encryption, connection
status callbacks, and P2P/dedicated-server connection flows. Steam recommends
initializing relay access early when using P2P/relay paths.

`ISteamNetworkingMessages` is a higher-level API that looks more like UDP
`sendto`: each message names a recipient rather than a connection handle. It is
easier to port simple UDP code to it, but the connection-oriented sockets API is
the better fit for a serious engine backend because connection state, accept,
close, and polling groups map cleanly to engine sessions.

Engine implication: model Steam as a `TransportBackend` implementation that
emits connection events and packet events. Gameplay code should see only engine
packets, channels, and peer IDs.

### Steam Datagram Relay

SDR routes traffic through Valve's private gaming network when appropriate. It
can protect player/server IP addresses, provide authentication/encryption/rate
limiting at the relay path, and sometimes improve routing. Steam's P2P socket
APIs can use SDR automatically for Steam users. Dedicated servers can also use
SDR, but the more robust hosted-server/ticket flow needs game coordinator style
backend work.

Engine implication: for a first Steam backend, P2P/listen-server Steam sockets
are simpler than dedicated server SDR. Dedicated servers should be designed, but
not implemented as the first milestone unless we also build the backend service
that issues and validates server access.

## Rust And Bevy Integration

### `steamworks`

The `steamworks` crate is the low-level Rust binding. Current docs list
`steamworks = "0.13.0"` as the dependency line and map that release to Steamworks
SDK 1.64. It exposes matchmaking, user/auth, networking messages, networking
sockets, networking utils, game server APIs, callbacks, and related types.

The crate dynamically loads Steamworks redistributable libraries, so runtime
packaging matters. Missing platform libraries near the game binary will fail at
startup even if Rust compilation succeeds.

### `bevy_steamworks`

`bevy_steamworks` provides a Bevy plugin around `steamworks`. Current docs show
version `0.16.0`, initialization via `SteamworksPlugin::init_app(app_id)`, and a
Bevy `Client` resource. It automatically runs Steam callbacks in Bevy's `First`
schedule and forwards callbacks as Bevy events.

Useful property: we do not need to hand-roll callback pumping if this plugin
works with the Bevy version we are on.

Risk: the plugin must be version-compatible with our Bevy dependency. If it
lags, the fallback is to use `steamworks` directly and run callbacks in our own
early schedule.

## How This Ties Into Afterglow

Afterglow already has the right foundations forming:

- stable entity IDs in `core::identity`
- command collection in the input layer
- explicit schedule sets via `AfterglowSet`
- world/chunk identity and streaming direction
- existing Iroh and GGRS research notes

Steam should slot in below the engine protocol:

```text
Steam lobby / invite / friend join
        |
Steam auth -> platform player identity
        |
SteamNetworkingSockets transport backend
        |
Afterglow network protocol
        |
commands, snapshots, deltas, replication, interest management
        |
authoritative world/chunk simulation
```

The engine API should look more like this:

```rust
pub trait NetworkTransport {
    fn poll_events(&mut self, out: &mut Vec<TransportEvent>);
    fn send(&mut self, peer: PeerId, channel: NetChannel, payload: &[u8]);
    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason);
}

pub enum TransportEvent {
    Connected(PeerId),
    Disconnected(PeerId, DisconnectReason),
    Packet { peer: PeerId, channel: NetChannel, bytes: Vec<u8> },
}
```

Steam-specific types should not leak into gameplay systems. A Steam backend can
store the mapping:

```text
SteamID64 <-> PeerId <-> NetworkPlayerId <-> StableEntityId
```

## Recommended Multiplayer Model

For Afterglow's target games, use an authoritative host/server model:

- clients send compact `PlayerCommand`s tagged by tick
- server validates, simulates, and assigns authoritative state
- clients predict local movement/interactions where useful
- clients reconcile from snapshots and correction packets
- chunk visibility controls replication scope
- persistent `StableEntityId`s identify long-lived objects
- ephemeral network IDs can be per-session and per-replication stream

This aligns with open-world streaming and multiplayer-first RPG goals. It also
keeps save/load, chunk migration, and future dedicated server support tractable.

Rollback/GGRS should not be the default world model. It requires deterministic
full-state snapshots, stable ordering, rollback-safe spawn/despawn, and strict
fixed-step simulation. That is realistic for a fighting game or small arena
simulation. It is hostile to a large immersive sim world with streaming, AI,
physics, skeletal animation, audio, and persistent authored state.

## Steam Backend Design

### Feature Gate

Use a native-only Cargo feature:

```toml
[features]
steam = ["dep:bevy_steamworks"]
```

Do not include Steam in default engine builds, wasm builds, or headless CI.

### Plugin Shape

```text
AfterglowSteamPlugin
├── initializes Steam client or reports unavailable
├── inserts SteamPlatform resource
├── maps Steam callbacks to engine events
├── owns lobby lifecycle systems
├── owns SteamNetworkingSockets transport adapter
└── registers diagnostics for relay state and connection quality
```

Schedule placement:

- `First`: Steam callbacks, if not delegated to `bevy_steamworks`
- `PreUpdate`: collect Steam lobby/network events into engine events
- `AfterglowSet::ReadInput`: local commands already exist
- `AfterglowSet::BuildCommands`: serialize commands into network packets
- `AfterglowSet::Simulate`: authoritative server-side apply
- `PostUpdate`: snapshot/delta packet emission

### Lobby Flow

Host:

1. Initialize Steam.
2. Create lobby with target max players.
3. Set lobby metadata: protocol version, build hash, map/world id, mode,
   host identity, privacy.
4. Start a Steam P2P listen socket.
5. On lobby member join, wait for transport connection/auth before spawning a
   replicated player.

Client:

1. Find lobby by friend invite or lobby query.
2. Join lobby.
3. Read metadata and validate protocol/build/content compatibility.
4. Connect to host via SteamNetworkingSockets.
5. Send auth/session hello packet.
6. Wait for server approval and snapshot baseline.

### Dedicated Server Flow

Minimal first-party dedicated server without SDR can use normal UDP/Iroh/local
transport first. Steam dedicated server integration is a later backend:

- Steam Game Server API for server identity/listing
- optional lobby handoff to dedicated server
- server auth/session ticket validation
- SDR hosted dedicated server flow when a game coordinator exists

Do not begin with hosted dedicated server SDR unless we also commit to a backend
service. Steam's robust ticket-based SDR flow needs a coordinator capable of
issuing signed relay auth tickets and managing server assignment.

## Channels

Use engine channels independent of transport:

| Channel | Reliability | Contents |
|---|---:|---|
| Control | reliable | hello, auth, version, disconnect reasons |
| Commands | unreliable sequenced | input commands by tick |
| Snapshots | unreliable sequenced | state snapshots and correction frames |
| Events | reliable | inventory, doors, scripted events, chat |
| Bulk | reliable | optional chunk/prefab deltas, mod handshakes |

Steam supports reliable and unreliable message flags. The engine should define
channel semantics once and map those semantics to Steam, Iroh, loopback, and
future transports.

## Testing Strategy

Normal unit tests should not require Steam or a logged-in client.

Use three layers:

1. Pure protocol tests: serialization, versioning, command ordering, snapshot
   application, disconnect reasons.
2. Fake/loopback transport tests: packet loss, duplication, reorder, latency,
   connection lifecycle.
3. Steam integration tests: gated behind feature and environment variables,
   manual/CI machine with Steam client or dedicated server setup.

Rendering-style "real hardware only" discipline does not directly transfer to
networking. For networking, fake transports are valuable because they make
packet reordering and fault injection deterministic. Steam backend tests should
exist, but they should not be the only tests for multiplayer correctness.

## Risks

- Platform lock-in: Steam is unavailable for non-Steam builds and wasm.
- Runtime packaging: Steamworks redistributable libraries must ship beside the
  game/server binaries.
- Callback discipline: Steam callbacks must be pumped every frame.
- Version compatibility: `bevy_steamworks` can lag Bevy.
- Dedicated server complexity: serious SDR dedicated server support wants a
  game coordinator/backend.
- Auth confusion: lobbies prove session membership, not gameplay authority.
- Host migration: Steam lobby ownership can change automatically, but that does
  not migrate our authoritative simulation state.
- Testing friction: real Steam integration is harder to automate than loopback.

## Recommendation

Add Steam only after the engine has a minimal transport-independent protocol:

1. Define `PeerId`, `NetworkPlayerId`, packet channels, and a transport trait.
2. Implement loopback/fake transport with deterministic fault injection.
3. Serialize current `PlayerCommand` into a command packet.
4. Add authoritative host state for player spawn/despawn and snapshot baseline.
5. Add optional `steam` feature with `bevy_steamworks` if version-compatible.
6. Implement Steam lobby create/join metadata.
7. Implement SteamNetworkingSockets transport adapter.
8. Add Steam auth handshake and SteamID-to-player mapping.
9. Add manual gated Steam integration tests.
10. Revisit dedicated server Steam Game Server API and SDR after the basic
    hosted/listen-server path works.

## References

- Steam Datagram Relay:
  https://partner.steamgames.com/doc/features/multiplayer/steamdatagramrelay
- Steam Networking overview:
  https://partner.steamgames.com/doc/features/multiplayer/networking
- `ISteamNetworkingSockets`:
  https://partner.steamgames.com/doc/api/ISteamnetworkingSockets
- `ISteamMatchmaking`:
  https://partner.steamgames.com/doc/api/isteammatchmaking
- Steam authentication and ownership:
  https://partner.steamgames.com/doc/features/auth
- `steamworks` Rust crate:
  https://docs.rs/steamworks/latest/steamworks/
- `bevy_steamworks` Rust crate:
  https://docs.rs/bevy-steamworks/latest/bevy_steamworks/
- Valve GameNetworkingSockets:
  https://github.com/ValveSoftware/GameNetworkingSockets
- Existing Afterglow Iroh note:
  `docs/research/iroh-networking.md`
- Existing Afterglow rollback note:
  `docs/research/bevy-ggrs-rollback.md`
