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

### Two planes: control vs data

Matchmaking and actual gameplay traffic should be separated into two planes:

| Plane | Purpose | Examples |
|---|---|---|
| **Control plane** | Create, search, join, leave sessions | `SessionRequest` / `SessionEvent` |
| **Data plane** | Gameplay replication, input, messages | Lightyear links |

For Steam, the control plane is Steamworks lobbies / messages.
For NonSteam, the control plane is a **session provider** — a listener exposed
by the host that accepts `SessionRequest`-like messages and replies with
`SessionEvent`s.

### Simple external API

The engine should expose a small helper API so games do not need to write raw
`SessionRequest`s for the common cases. See
[`docs/api/session-api.md`](../api/session-api.md) for the proposed surface.
The short version:

```rust
app.session().host(config, identity);
app.session().host_with_endpoint(config, identity, "0.0.0.0:7777".parse()?);
app.session().join_non_steam(code, "203.0.113.42:7777".parse()?, identity);
app.session().join_steam(code, identity);
app.session().join_local(code, identity);
app.session().leave();
```

These helpers internally produce the same `SessionRequest` messages documented
below.

### Session provider endpoint

For NonSteam, the client must know **where** to send session requests. The
provider address is part of the request, not hidden inside the session config:

```rust
/// Where to send session control-plane requests for a given backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProviderEndpoint {
    /// In-process catalog (tests / title screen).
    InProcess,
    /// NonSteam UDP/netcode session listener, e.g. the host's address.
    Udp(SocketAddr),
    /// Steamworks resolves the endpoint implicitly via Steam lobbies.
    Steam,
}
```

`JoinByCode` and `Search` carry the provider the client wants to talk to:

```rust
pub enum SessionRequest {
    Create(SessionConfig, PlayerIdentity),
    Search(SessionSearch),
    Join {
        backend: SessionBackend,
        session: SessionId,
        identity: PlayerIdentity,
        provider: ProviderEndpoint,
    },
    JoinByCode {
        backend: SessionBackend,
        code: SessionCode,
        identity: PlayerIdentity,
        provider: ProviderEndpoint,
    },
    Leave,
}

pub struct SessionSearch {
    pub backend: SessionBackend,
    pub provider: ProviderEndpoint,
    pub metadata: HashMap<String, String>,
    pub require_open_slot: bool,
    pub max_results: u32,
}
```

This matches the expected player flow:

- A friend shares a code **plus** the host/provider address over Discord; the
  join message includes both.
- Steam does not need an explicit provider because Steamworks is the provider.
- Local tests use `InProcess`.

### NonSteam session provider / listener

The current `NonSteamSessionCatalog` is an in-process state machine. A real
NonSteam backend needs two pieces:

1. **`NonSteamSessionProvider`** — the engine/server side that hosts the catalog
   and listens for control-plane messages on a UDP/netcode socket.
2. **`NonSteamSessionClient`** — the engine/client side that serializes
   `SessionRequest`s and sends them to a `ProviderEndpoint`, then receives
   `SessionEvent`s.

On the host, creating a session also starts the provider listener on the chosen
address. On the client, joining a session contacts that provider.

The provider listener and the gameplay server may run on the same address and
even the same process, but they are logically separate:

- Control: "here is my join code, can I join?"
- Data: Lightyear replication + input.

### Split gameplay transport from resolved address

Once the control-plane join succeeds, `SessionInfo` carries the data-plane
target:

```rust
/// What kind of transport the session prefers for gameplay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionTransport {
    /// In-process Crossbeam links for local tests.
    Local,
    /// Native UDP + netcode.io (or WebTransport/WebSocket for wasm).
    Netcode,
    /// Steam networking / Steam Datagram Relay.
    SteamSdr,
}

/// How to reach the gameplay session once accepted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionConnectionTarget {
    /// Direct socket address (LAN or host-as-server with known IP).
    Direct(SocketAddr),
    /// Steam networking identity (SteamID + virtual port).
    SteamPeer(u64),
    /// Not resolved yet.
    Resolving,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionConnection {
    pub transport: SessionTransport,
    pub target: SessionConnectionTarget,
}
```

`SessionConfig` keeps only the transport preference:

```rust
pub struct SessionConfig {
    pub name: String,
    pub backend: SessionBackend,
    pub max_members: u32,
    pub visibility: SessionVisibility,
    pub metadata: HashMap<String, String>,
    pub transport: SessionTransport,
}
```

### Per-backend control + data flow

#### NonSteam listen-server / direct-IP model

1. Host creates session. Its `NonSteamSessionProvider` starts listening on a
   UDP/netcode control socket at `provider_addr`.
2. Host shares code + `provider_addr` with friends.
3. Client sends `JoinByCode { provider: Udp(provider_addr), code, ... }`.
4. Provider accepts, returns `SessionInfo` with
   `connection: Netcode, target: Direct(gameplay_addr)`.
5. Engine transport consumer spawns Lightyear netcode links to
   `gameplay_addr`.

`provider_addr` and `gameplay_addr` are often the same address in the simplest
host-as-server case.

#### Steam backend

1. Host creates a Steam lobby.
2. Steam session provider maps `SessionCode` to the Steam lobby ID.
3. Client joins via Steamworks lobby invite or `JoinByCode { provider: Steam, ... }`.
4. Steam SDR resolves the data-plane path; `SessionConnectionTarget::SteamPeer`
   drives Lightyear's Steam transport.

### Transport consumer system

Introduce an engine system `start_session_transport` that runs in
`AfterglowSessionSet::ApplyEffects` after the control-plane join has produced a
`SessionInfo`:

```rust
fn start_session_transport(
    mut commands: Commands,
    mut pending: ResMut<PendingNetcodeStartup>,
    // Lightyear registry/config resources
) {
    // Drain pending params and spawn NetcodeClient / NetcodeServer entities.
    // For Steam: spawn Lightyear Steam link entities.
    // For Local: already handled by the bridge itself.
}
```

`PendingNetcodeStartup` now derives from `SessionConnectionTarget` and the local
`SessionMemberId`, not from a `host` string baked into session config.

### Host migration

For listen-server / host-client sessions, if the host leaves, the session ends
(current behavior). Long-term, Afterglow could support migration by electing a
new host and re-resolving the connection target, but that is out of scope for
the first slice.

### Join-by-code flow

```
host (NonSteam listen-server):
  CreateSession(NonSteam, transport=Netcode)
    -> NonSteamSessionCatalog allocates code
    -> NonSteamSessionProvider starts listening

friend (Discord):
  "code is XFQ-KRB, provider is 203.0.113.42:7777"

client:
  JoinByCode {
    backend: NonSteam,
    provider: Udp(203.0.113.42:7777),
    code: SessionCode::new("XFQ-KRB"),
    identity: my_identity,
  }
    -> provider validates, returns SessionInfo
    -> SessionEvent::Joined(info) triggers bridge
    -> bridge produces PendingNetcodeStartup from info.connection
    -> start_session_transport spawns UDP/netcode links
    -> gameplay begins
```

## Trade-offs (in scope)

| Approach | Pro | Con |
|---|---|---|
| Local / Crossbeam | Zero setup, deterministic tests | Single process only |
| NonSteam listen-server + direct IP | No extra infrastructure; friend shares code + address | Exposes host IP, fails across many NATs unless both players forward ports |
| Steam lobbies + SDR | Free for Steam players, handles NAT/relay, proven by R.E.P.O. | Steam-only, not cross-platform |

## Future extensions (out of scope for now)

| Approach | Pro | Con |
|---|---|---|
| Dedicated server + matchmaker | No IP exposure, works through NAT, host migration unnecessary | Requires hosted service, costs money/complexity |
| Custom relay / NAT traversal | Works without Steam or dedicated server | Relay infrastructure and operation cost |
| WebTransport / WebSocket | Cross-platform browser play | Certificate and hosting complexity |
| Direct IP / LAN | Simple, zero infra | Exposes IP, fails across most NATs |
| Host-client with custom relay | Works without dedicated server | Relay infra still required; host advantage physics |

## Recommended Next Slice

1. Add `ProviderEndpoint` and update:
   - `SessionRequest::Join { provider }`
   - `SessionRequest::JoinByCode { provider }`
   - `SessionSearch { provider }`
2. Add the simple external API helpers in `docs/api/session-api.md` (e.g.
   `app.session().host_with_endpoint`, `app.session().join_non_steam`).
3. Introduce `NonSteamSessionProvider` (server/listener) and
   `NonSteamSessionClient` (networked request sender) abstractions.
4. **Decision:** keep `DirectUdp { host: String }` on `SessionTransport`. The
   game shares its gameplay address out-of-band (e.g. Discord) and the client
   passes it explicitly to `join_non_steam`. Automatic address exchange was
   deferred per product call.
6. Keep the existing bridge for `Local`, and extend it to consume
   `SessionConnectionTarget` instead of parsing `host`.
7. Add an engine consumer system that drains `PendingNetcodeStartup` and spawns
   Lightyear `NetcodeClient`/`NetcodeServer` entities for the direct-IP case.
8. Add a minimal LAN / direct-IP test proving a host and a join-by-code client
   can find the provider, exchange session messages, and establish real UDP
   links.

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
