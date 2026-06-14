# Session Transport and Connection Design

## Status

Research and design note for how Afterglow session join codes / transports should
resolve into real Lightyear network links. The current implementation writes
`NetcodeClientParams`/`NetcodeServerParams` into `PendingNetcodeStartup` and
expects an unspecified "consumer" to spawn UDP/netcode links. The open question
is: what should that consumer look like, and how do players connect by join code
without exposing the host's IP?

## Research Findings

### 1. Lightyear transport options

Lightyear is transport-agnostic. From the project docs, examples, and crate
features, it supports:

| Transport | Use case | Notes |
|---|---|---|
| **UDP + netcode.io** | Native client/server over the internet | The default for real multiplayer. Uses `NetcodeClient`/`NetcodeServer` with `UdpIo` / `ServerUdpIo`. Supports manual auth (`server_addr`, `client_id`, `protocol_id`, `private_key`) or `ConnectToken` auth. |
| **Crossbeam channels** | In-process / local / tests | Used by `SessionTransport::Local` today. Spawns link entities with `CrossbeamIo`. |
| **WebTransport (QUIC)** | Browser wasm + native | Requires certificates for wasm. One path for cross-platform web games. |
| **WebSocket** | Browser fallback | Simpler than WebTransport but no unreliable channel semantics at the socket layer. |
| **Steam** | Steam overlay / P2P / relay | Uses Steamworks networking / SDR. Lightyear exposes this as a backend. |

Sources:

- Lightyear README and feature list (`lightyear_udp`, `lightyear_netcode`,
  `lightyear_webtransport`, `lightyear_websocket`, `lightyear_steam`) in
  `Cargo.toml`.
- Lightyear book, "Building the client and server": netcode manual auth + UDP
  IO components.
- Search result: "The server supports running multiple transports at the same
  time (WebTransport, WebSockets, UDP, channels, Steam)."

### 2. Lightyear's own lobby example

Lightyear ships a `lobby` example that already models the problem we are trying
to solve:

- A **dedicated server** maintains a replicated `Lobbies` resource.
- Clients connect to the dedicated server and **join a lobby**.
- Inside a lobby, a client can click `StartGame` and choose who hosts:
  - the dedicated server (uses Lightyear `Rooms` per lobby), or
  - one of the clients in `HostServer` mode (the client app also runs a server
    in the same process).

This shows two valid models: **dedicated server** and **host-client / listen
server**.

Source: `cBournhonesque/lightyear/examples/lobby/README.md` via GitHub API.

### 3. Steam SDR / Steam lobbies

Steam Datagram Relay (SDR) is Valve's relay network. Key properties:

- Players connect via **SteamID / Steam networking identity**, not raw IPs.
- Steam handles NAT traversal and often routes traffic over the Valve backbone.
- IP addresses are hidden from peers.
- `ISteamNetworkingSockets::ConnectP2P` connects by peer identity.
- R.E.P.O.'s "join code" UX is actually implemented on top of **Steam lobbies**
  (Steam overlay invite links), not raw UDP addresses.

Sources:

- Steamworks `ISteamNetworkingSockets` documentation.
- Steam Datagram Relay documentation on Steamworks partner site.
- Reddit / ProGameGuides: R.E.P.O. hosts create Steam lobbies and share invite
  links.

### 4. Bevygap / Edgegap pattern

`bevygap` (Richard Jones) demonstrates how a Bevy + Lightyear game can avoid
exposing IPs:

- A **matchmaker** runs as a service.
- The matchmaker generates a **Lightyear `ConnectToken`** and stores the mapping
  from token ID to Edgegap session.
- The client receives the token (via HTTP/WebSocket) and uses it to connect to
  the allocated gameserver.
- The gameserver is headless, deployed close to players, and maintains a list
  of active connect tokens.

This is the **dedicated server + matchmaker** model, and it uses Lightyear's
native `ConnectToken` authentication.

Sources:

- `metabrew.com/article/bevygap-bevy-multiplayer-with-edgegap-and-lightyear`
- `bevygap` GitHub repository description.

### 5. NAT, P2P, and relay realities

- Glenn Fiedler / Gaffer on Games and Valve's GameNetworkingSockets docs
  emphasize that raw UDP sockets are not enough for internet P2P.
- Practical internet multiplayer needs one of:
  1. dedicated server with a public IP,
  2. a relay service (Steam SDR, Edgegap, custom), or
  3. a STUN/TURN-style NAT traversal layer, which is fragile and usually worse
     than a relay.

Sources:

- ValveSoftware/GameNetworkingSockets README: P2P networking / NAT traversal.
- `multiplayernetworking.com` curated resources.

## Design Principles

1. **Separation of concerns.** Matchmaking/session identity is not transport.
   The session layer decides *what game to join*. A transport resolver decides
   *how to reach it*.
2. **Backend-specific resolution.** Native and Steam should resolve addresses
   differently. Do not force a single `host: String` field to serve both.
3. **No IP exposure by default.** Join codes should resolve through a backend
   service, Steam lobby, or LAN discovery — not by embedding the host's public
   IP in `SessionInfo`.
4. **Lightyear owns the wire.** Use Lightyear's transports directly (UDP,
   Steam, WebTransport). Afterglow only decides *when* to spawn them.
5. **Consumer is engine-owned, but backend-aware.** The missing system that
   drains pending params and spawns Lightyear links should live in the engine,
   not in every game.

## Proposed Design

### Split session transport from resolved address

Replace `SessionTransport::DirectUdp { host: String }` with a transport
preference enum and a separate connection-resolution type:

```rust
/// What kind of transport the session prefers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionTransport {
    /// In-process Crossbeam links for local tests or split-screen logic.
    Local,
    /// Native UDP + netcode.io (or WebTransport/WebSocket for wasm).
    /// The actual address comes from the session backend.
    Netcode,
    /// Steam networking / Steam Datagram Relay.
    SteamSdr,
}

/// How to reach the session once it has been accepted.
/// Populated by the backend and consumed by the transport startup system.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionConnectionTarget {
    /// Direct socket address (for LAN or dedicated server with known IP).
    Direct(SocketAddr),
    /// Lightyear netcode connect token issued by a matchmaker.
    /// Hides the real server address from the client.
    NetcodeToken(Vec<u8>),
    /// Steam networking identity (SteamID + virtual port).
    SteamPeer(u64), // or a real SteamNetworkingIdentity wrapper
    /// No resolved target yet; waiting for matchmaker / discovery.
    Resolving,
}

/// Connection metadata attached to a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionConnection {
    pub transport: SessionTransport,
    pub target: SessionConnectionTarget,
}
```

`SessionConfig` would then look like:

```rust
pub struct SessionConfig {
    pub name: String,
    pub backend: SessionBackend,
    pub max_members: u32,
    pub visibility: SessionVisibility,
    pub metadata: HashMap<String, String>,
    pub transport: SessionTransport, // preference only
}
```

And `SessionInfo` would expose:

```rust
pub struct SessionInfo {
    pub id: SessionId,
    pub code: SessionCode,
    pub backend: SessionBackend,
    pub name: String,
    pub owner: SessionMemberId,
    pub owner_identity: PlayerIdentity,
    pub member_count: u32,
    pub max_members: u32,
    pub visibility: SessionVisibility,
    pub metadata: HashMap<String, String>,
    pub transport: SessionTransport,
    pub connection: SessionConnection, // only meaningful fields populated by backend
}
```

### Per-backend resolution

#### NonSteam backend

Support at least two modes:

1. **Direct LAN / direct IP** — for local play or debug.
   - Host picks a port; session carries `SessionConnectionTarget::Direct(addr)`.
   - This exposes the IP, but only because the user explicitly chose direct IP.
2. **Matchmaker + dedicated server / relay** — for internet play.
   - Create session → backend contacts a matchmaker service.
   - Matchmaker returns a session handle and a future `ConnectToken`.
   - Session code maps to the matchmaker session handle.
   - Join by code → client asks matchmaker for the `ConnectToken` (and server
     address if not token-based).
   - `SessionConnectionTarget` becomes `NetcodeToken(token)` or `Direct(server_addr)`.

The current `DirectUdp { host }` becomes the **direct IP debug path** only.

#### Steam backend

- Use Steam lobbies as the matchmaking layer.
- The Steam session provider creates a Steam lobby and maps `SessionCode` to the
  lobby ID internally.
- Join by code → resolve to Steam lobby → invite/join via Steamworks API.
- `SessionConnectionTarget::SteamPeer(steam_id)` feeds Lightyear's Steam
  transport.

### Consumer system

Introduce an engine system `start_session_transport` that runs in
`AfterglowSessionSet::ApplyEffects` after the bridge:

```rust
fn start_session_transport(
    mut commands: Commands,
    mut pending: ResMut<PendingNetcodeStartup>,
    mut intents: EventReader<SessionTransportIntent>,
    // Lightyear registry/config resources
) {
    // For DirectUdp pending params: spawn NetcodeClient/NetcodeServer entities.
    // For Steam peer identity: spawn Lightyear Steam link entities.
    // For Local: already handled by the bridge itself.
}
```

`PendingNetcodeStartup` is general enough to keep, but its data should come from
`SessionConnectionTarget`, not from a `host` string baked into session config.
Alternatively, rename `PendingNetcodeStartup` to `PendingSessionTransportStartup`
and make it backend-aware.

### Host migration

- For dedicated-server sessions, the server entity is independent of any player.
  Host migration is not needed.
- For listen-server / host-client sessions, if the host leaves, the session ends
  (current behavior). Long-term, Afterglow could support migration by electing a
  new host and re-resolving the connection target, but that is out of scope for
  the first slice.

### Join-by-code flow

```
host:
  CreateSession(NonSteam, transport=Netcode)
    -> NonSteamSessionCatalog allocates code
    -> (future) matchmaker creates server / returns token
    -> SessionInfo contains transport=Netcode, target=NetcodeToken(...) or Direct(...)

client:
  JoinByCode(NonSteam, code)
    -> backend resolves code -> session
    -> backend fetches connection target (token / address)
    -> SessionEvent::Joined(info) triggers bridge
    -> bridge produces PendingNetcodeStartup
    -> start_session_transport spawns UDP/netcode links
    -> gameplay begins
```

## Trade-offs

| Approach | Pro | Con |
|---|---|---|
| Dedicated server + matchmaker | No IP exposure, works through NAT, host migration unnecessary | Requires hosted service, costs money/complexity |
| Steam lobbies + SDR | Free for Steam players, handles NAT/relay, proven by R.E.P.O. | Steam-only, not cross-platform |
| Direct IP / LAN | Simple, zero infra | Exposes IP, fails across most NATs |
| Host-client with custom relay | Works without dedicated server | Relay infra still required; host advantage physics |

## Recommended Next Slice

1. Remove `DirectUdp { host: String }` from `SessionTransport`.
2. Add `SessionTransport::Netcode` and `SessionConnectionTarget`.
3. Keep the existing bridge for `Local`, and extend it to consume
   `SessionConnectionTarget` instead of parsing `host`.
4. Add an engine consumer system that drains `PendingNetcodeStartup` and spawns
   Lightyear `NetcodeClient`/`NetcodeServer` entities for the direct-IP case.
5. Add a minimal LAN / direct-IP test proving a host and a join-by-code client
   establish real UDP links.
6. Defer full matchmaker integration to a follow-up slice; document the
   `NetcodeToken` target as the intended hook.

## References

- `docs/research/network-backend-abstraction.md`
- `docs/research/player-identity-authentication.md`
- `docs/research/steam-multiplayer.md`
- `docs/api/network.md`
- `crates/afterglow-engine/src/network/session/mod.rs`
- `crates/afterglow-engine/src/network/lightyear/link/mod.rs`
- `crates/engine-rpg-harness/src/rig/setup.rs`
- `cBournhonesque/lightyear/examples/lobby/README.md`
- `metabrew.com/article/bevygap-bevy-multiplayer-with-edgegap-and-lightyear`
- Steamworks `ISteamNetworkingSockets` documentation
- `ValveSoftware/GameNetworkingSockets`
