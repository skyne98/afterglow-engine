# Session Workflows

End-to-end workflows for joining an Afterglow multiplayer session and
maintaining player identity across the session lifecycle.

## Layers

```text
Player (identity)
    |
    v
Control plane  -- SessionRequest / SessionEvent -- decides *which* game to join
    |
    v
Data plane     -- Lightyear links -- actual gameplay replication and input
    |
    v
Gameplay       -- authoritative simulation, prediction, presentation
```

- **Control plane**: matchmaking, join codes, lobby state, member lifecycle.
- **Data plane**: Lightyear transport (UDP/netcode, Steam SDR, WebTransport,
  Crossbeam).
- **Identity layer**: durable player identity that survives disconnect/rejoin.

## Glossary

| Type | Meaning | Lifetime |
|---|---|---|
| `PlayerIdentity` | Durable identity: native Ed25519 keypair proof or Steam ticket. | Persistent across sessions. |
| `SessionMemberId` | Per-session handle for a player. | Tied to one `SessionId`. |
| `SessionId` | Internal durable session identifier. | Tied to one session instance. |
| `SessionCode` | Short player-facing join token, e.g. `XFQ-KRB`. | Valid while session is active. |
| `ProviderEndpoint` | Where control-plane session requests are sent. | Known before join. |
| `SessionConnectionTarget` | How to reach the gameplay data plane after join. | Returned in `SessionInfo`. |

## 1. NonSteam: Listen Server / Direct IP

Use this for LAN parties, local tests, or any scenario where the host's address
is known or shared explicitly. The host runs both the session provider listener
and the gameplay server.

### Preconditions

- Host and client both run the Afterglow client with the `lightyear` feature.
- Host has a reachable UDP socket address `host_addr`.
- Client has a native Ed25519 keypair (or a Steam identity, which the non-Steam
  provider accepts as a passthrough).

### Sequence

```text
host_app
    | world.write_message(SessionRequest::Create(
    |     SessionConfig { backend: NonSteam, transport: Netcode, ... },
    |     PlayerIdentity::Native(...)))
    v
NonSteamSessionProvider starts listening for control messages on host_addr
NonSteamSessionCatalog allocates SessionId and SessionCode XFQ-KRB
    |
host shares "XFQ-KRB @ host_addr" with friend (Discord)
    |
client_app
    | world.write_message(SessionRequest::JoinByCode(
    |     backend: NonSteam,
    |     provider: ProviderEndpoint::Udp(host_addr),
    |     code: SessionCode::new("XFQ-KRB"),
    |     identity: PlayerIdentity::Native(...)))
    v
NonSteamSessionClient serializes the request and sends it to host_addr
    |
    v
NonSteamSessionProvider receives JoinByCode request
    - validates identity proof (Ed25519 signature over nonce + "XFQ-KRB")
    - looks up code -> SessionId
    - checks capacity / already joined
    - allocates a SessionMemberId
    - stores (public_key -> member_id) for rejoin detection
    - emits SessionInfo { id, code, owner, owner_identity, connection, ... }
    |
    v
client_app receives SessionEvent::Joined(SessionInfo)
    |
    v
AfterglowSessionLightyearBridge reads SessionInfo.connection
    - SessionConnection { transport: Netcode, target: Direct(gameplay_addr) }
    - writes PendingNetcodeStartup {
          client: Some(NetcodeClientParams {
              server_addr: gameplay_addr,
              client_id: member_id.as_raw() as u64,
              protocol_id, private_key
          })
      }
    |
    v
start_session_transport system drains PendingNetcodeStartup
    - spawns Lightyear NetcodeClient + UdpIo entity
    - Lightyear opens the UDP socket and completes the netcode handshake
    |
    v
Gameplay begins: client input -> Leafwing -> Lightyear -> server simulation
```

### Identity during the session

- The client's `PlayerIdentity::Native` contains the Ed25519 public key.
- On first join, the provider allocates a fresh `SessionMemberId` and stores
  the mapping `public_key -> member_id`.
- If the client disconnects and rejoins with the same public key, the provider
  returns the existing `SessionMemberId`.
- The server simulation uses `SessionMemberId` (via Lightyear's client id) to
  identify the player's controlled avatar.
- `PlayerIdentity` itself is not used as an ECS id; it is only the admission
  and persistence boundary.

### Leave / disconnect / reconnect

```text
client_app writes SessionRequest::Leave
    |
NonSteamSessionClient sends leave to provider
    |
provider removes member from session, emits SessionEvent::MemberLeft + Left
    |
AfterglowSessionLightyearBridge despawns Lightyear link entities
    |
if owner leaves, provider emits SessionEvent::SessionEnded and frees code
```

If the client crashes and reconnects:

```text
client_app restarts, loads the same private key
    |
client_app writes SessionRequest::JoinByCode(code = XFQ-KRB, provider = host_addr)
    |
provider validates native proof with the same public key
    |
provider finds existing public_key -> SessionMemberId mapping
    |
provider returns SessionInfo with the SAME SessionMemberId
    |
client joins data plane; server can remap the member to a new Lightyear client id
```

## 2. NonSteam: Matchmaker + Dedicated Server

Use this for internet play without exposing a host's home IP. A matchmaking
service owns the control plane and may also allocate a dedicated game server.

### Preconditions

- A matchmaker service runs at `https://matchmaker.example.com`.
- The matchmaker can spin up dedicated game servers (or uses a long-lived pool).
- Client and server trust the matchmaker to issue Lightyear `ConnectToken`s.

### Sequence

```text
host_app (or headless server)
    | HTTP POST /create {
    |     config: SessionConfig,
    |     identity: PlayerIdentity::Native(...)
    | }
    v
matchmaker creates session record, allocates SessionCode XFQ-KRB
    - may deploy a dedicated server to gameplay_addr
    - generates a Lightyear ConnectToken bound to gameplay_addr
    |
matchmaker returns { code: "XFQ-KRB", gameplay_token, gameplay_addr }
    |
host shares "XFQ-KRB" publicly or privately
    |
client_app
    | lists public lobbies:
    |     SessionRequest::Search(
    |         backend: NonSteam,
    |         provider: ProviderEndpoint::Http("https://matchmaker.example.com"),
    |         ...
    |     )
    |   OR receives code from friend
    |
client_app
    | HTTP POST /join { code, identity }
    v
matchmaker validates identity proof, checks capacity/already joined
    |
matchmaker returns SessionInfo {
    code: "XFQ-KRB",
    connection: SessionConnection {
        transport: Netcode,
        target: NetcodeToken(gameplay_token)
    }
}
    |
    v
client_app receives SessionEvent::Joined(SessionInfo)
    |
    v
bridge writes PendingNetcodeStartup {
    client: Some(NetcodeClientParams {
        connect_token: gameplay_token,
        ...
    })
}
    |
    v
start_session_transport spawns NetcodeClient using the token
    |
    v
client connects to gameplay_addr (which may be hidden from the player)
```

### Why this hides the IP

- The client never sees a direct gameplay address if `NetcodeToken` is used.
- The matchmaker can rotate servers, move sessions, or use a relay without
  changing the join code.
- Host migration is free: the matchmaker can point a code at a new server and
  return a new token.

## 3. Steam: Lobby + Steam Datagram Relay

Use this for Steam players. Steamworks provides identity, lobbies, invites, and
relay networking.

### Preconditions

- Game is published on Steam; client and server both initialize Steamworks.
- Steam SDR is enabled.

### Sequence

```text
host_app
    | world.write_message(SessionRequest::Create(
    |     SessionConfig { backend: Steam, transport: SteamSdr, ... },
    |     PlayerIdentity::Steam { steam_id, ticket }))
    v
SteamSessionProvider:
    - validates the Steam ticket against Steamworks
    - calls ISteamMatchmaking::CreateLobby
    - maps Afterglow SessionCode XFQ-KRB -> Steam LobbyId
    - sets lobby metadata (name, mode, capacity, code)
    |
host invites friends via Steam overlay or shares "XFQ-KRB"
    |
friend's Steam client receives invite (or friend enters code)
    |
client_app
    | world.write_message(SessionRequest::JoinByCode(
    |     backend: Steam,
    |     provider: ProviderEndpoint::Steam, // implicit
    |     code: SessionCode::new("XFQ-KRB"),
    |     identity: PlayerIdentity::Steam { steam_id, ticket }))
    v
SteamSessionProvider:
    - validates Steam ticket
    - resolves code -> Steam LobbyId
    - joins the Steam lobby via ISteamMatchmaking::JoinLobby
    - reads owner SteamID and member list from Steamworks
    - emits SessionInfo {
          backend: Steam,
          connection: SessionConnection {
              transport: SteamSdr,
              target: SteamPeer(owner_steam_id)
          }
      }
    |
    v
client_app receives SessionEvent::Joined(SessionInfo)
    |
    v
bridge writes PendingNetcodeStartup for the Steam transport path
    |
    v
start_session_transport spawns Lightyear Steam link entity
    - Steam peers connect via SteamID, not IP
    - Steam SDR handles NAT traversal and relaying
    |
    v
Gameplay begins
```

### Identity during the session

- `PlayerIdentity::Steam` carries `steam_id` and a ticket.
- Steam backend validates the ticket with Steamworks.
- `SessionMemberId` maps to a Steam `SteamId` for the duration of the session.
- Rejoin with the same SteamID returns the same `SessionMemberId`.
- The client's home IP is never exposed to peers.

## 4. Local / In-Process

Use this for single-player-as-multiplayer tests, split-screen simulation, or
unit tests.

```text
host_app
    | SessionRequest::Create(
    |     SessionConfig { backend: NonSteam, transport: Local, ... },
    |     identity)
    v
NonSteamSessionCatalog allocates code XFQ-KRB in-process
    |
client_app (same Bevy App for tests; or separate identical App)
    | SessionRequest::JoinByCode(
    |     backend: NonSteam,
    |     provider: ProviderEndpoint::InProcess,
    |     code: "XFQ-KRB",
    |     identity)
    v
Same NonSteamSessionCatalog processes the request
    |
AfterglowSessionLightyearBridge spawns Crossbeam link entities in-process
    |
Gameplay begins over Crossbeam channels
```

## Identity Comparison

| Concern | Native | Steam |
|---|---|---|
| What the client proves possession of | Ed25519 private key | Steam account (ticket from Steamworks) |
| What the server verifies | Signature over nonce + target | Steam ticket with Steamworks |
| What persists across sessions | Same public key -> same save/account | Same SteamID -> same account |
| What is session-local | `SessionMemberId` | `SessionMemberId` |
| How rejoin works | `key_to_member` map | SteamID lookup |
| IP exposure | Depends on transport | Hidden by SDR |

## Message Summary

| Message | Direction | Plane | Contents |
|---|---|---|---|
| `SessionRequest::Create` | client -> provider | control | config, identity |
| `SessionRequest::Search` | client -> provider | control | filters, provider endpoint |
| `SessionRequest::Join` | client -> provider | control | session id, identity, provider |
| `SessionRequest::JoinByCode` | client -> provider | control | code, identity, provider |
| `SessionRequest::Leave` | client -> provider | control | no payload |
| `SessionEvent::Created` | provider -> client | control | SessionInfo |
| `SessionEvent::Joined` | provider -> client | control | SessionInfo |
| `SessionEvent::SearchResults` | provider -> client | control | list of SessionInfo |
| `SessionEvent::MemberJoined` | provider -> all members | control | session + member id |
| `SessionEvent::MemberLeft` | provider -> all members | control | session + member id + reason |
| `SessionEvent::SessionEnded` | provider -> all members | control | session id |
| `SessionEvent::Error` | provider -> client | control | SessionError |

## Current Implementation Status

Implemented today:

- `PlayerIdentity`, `NativeIdentityProof`, `SessionIdentityNonce`.
- `SessionRequest`/`SessionEvent` in-process message protocol.
- In-memory `NonSteamSessionCatalog` with code allocation and identity
  verification.
- Native Ed25519 proof verification via `ed25519-dalek`.
- `SessionMemberId` reuse on native rejoin.
- `Local` Crossbeam bridge via `AfterglowSessionLightyearBridgePlugin`.
- `PendingNetcodeStartup` written for `DirectUdp { host }`.

Not yet implemented:

- `ProviderEndpoint` in `Join`/`JoinByCode`/`Search`.
- Networked `NonSteamSessionProvider` / `NonSteamSessionClient`.
- Engine consumer that drains `PendingNetcodeStartup` and spawns netcode links.
- `SessionConnection` / `SessionConnectionTarget` replacing `DirectUdp { host }`.
- `Steam` session backend.

See also:

- `docs/api/network.md` for the full type reference.
- `docs/research/session-transport-connection-design.md` for the transport
  resolution design and control-plane/data-plane split.
- `docs/research/player-identity-authentication.md` for identity conventions
  and security trade-offs.
- `docs/research/steam-multiplayer.md` for Steamworks API details.
