# Player Identity and Authentication Conventions

## Status

Research note comparing how successful multiplayer games handle player identity
and authentication, with implications for Afterglow. Implemented in the engine:

- `PlayerIdentity` enum with `Native` (Ed25519 keypair proof) and `Steam` (ticket
  passthrough) variants.
- `NativeIdentityProof` carrying public key + signature over a server nonce and
  request target.
- Identity attached to `SessionRequest::Create` / `Join` / `JoinByCode` and to
  `SessionInfo` / `AfterglowSessionState`.
- Non-Steam provider verifies Ed25519 proofs with `ed25519_dalek::verify_strict`
  and binds public keys to `SessionMemberId` for rejoin detection.

Steam ticket validation is intentionally deferred to the future Steam session
backend.

## TL;DR

Games that actually shipped at scale do **not** have servers hand clients a
private key. Instead they use one of these patterns:

1. **Platform identity** (Steam, Xbox, PlayStation): the platform issues a
   signed ticket or token; the server validates it with the platform. The only
   durable identity is the platform ID (e.g., SteamID64).
2. **Backend-issued tickets/certs**: a trusted game coordinator signs a short-
   lived ticket or certificate; the client presents it to the game server, which
   verifies the signature.
3. **Client-generated keypairs**: for anonymous/device identity, the client
   generates its own keypair, stores the private key locally, and registers the
   public key with the backend. The server verifies signatures but never sees
   the private key.

Afterglow should mirror this: `PlayerIdentity` is an enum like
`SessionBackend`, with `Native` (client keypair or backend ticket) and `Steam`
(platform ticket) variants. `SessionMemberId` stays session-local.

## Authoritative Sources

### Patrick Wyatt — battle.net and Guild Wars

Patrick Wyatt was Blizzard's second employee, lead programmer/producer on
Warcraft I/II/Diablo/StarCraft, and later team lead for Battle.net and
ArenaNet/Guild Wars network technology. His GDC 2012 talk *"Writing Server and
Network Code for Your Online Game"* is explicitly framed around shipping
Battle.net and Guild Wars.

Key points from the talk abstract and his blog:

- The server code must be "resistant to hacking".
- Services should handle common failure conditions (clients losing connection,
  backend hiccups) without requiring human intervention.
- Distributed systems should recover automatically so players do not quit in
  anger.
- He recommends building reliable game services first, not just the wire
  protocol.

Source: GDC Vault, "Writing Server and Network Code for Your Online Game",
Patrick Wyatt, GDC 2012; Patrick Wyatt biography on MobyGames/Wikipedia.

### Valve / Steamworks

Valve documents two identity-related systems that are directly used in Team
Fortress 2, Dota 2, Counter-Strike, and any other Steam game.

#### Steam ID as canonical identity

Every Steam user is identified by a 64-bit Steam ID (`CSteamID`). This is the
only durable player identifier; everything else (lobby membership, sessions,
friends, bans) references it.

#### Session Tickets

For client-to-client (P2P) or client-to-game-server:

- Client A calls `ISteamUser::GetAuthSessionTicket`.
- Client A sends the ticket to client/server B.
- B calls `ISteamUser::BeginAuthSession`. Steam's backend verifies the ticket
  and returns `ValidateAuthTicketResponse_t`.
- The ticket proves the user's SteamID, ownership of the app, and VAC status.
- Tickets are single-use and must be cancelled/ended when the session ends.

For client-to-trusted-backend:

- Client requests `GetAuthTicketForWebApi`.
- Server calls `ISteamUserAuth/AuthenticateUserTicket` against
  `partner.steam-api.com`.
- Response includes the user's SteamID64.

Source: Steamworks Documentation, "User Authentication and Ownership",
`partner.steamgames.com/doc/features/auth`.

#### Steam Datagram Relay (SDR)

SDR is Valve's relay network. It uses:

- A proprietary PKI to authenticate clients and servers.
- Individual, short-term certificates tied to a specific player identity.
- Ticket-based server connection flow where a game coordinator signs a
  `SteamDatagramRelayAuthTicket` authorizing a specific client to talk to a
  specific server.
- Hosts identified by `SteamID` or `SteamNetworkingIdentity`, not by raw IP.

Key quote: "We use a proprietary public key infrastructure (PKI) to
authenticate clients and servers. Players are issued individual, short-term
certificates, tied to their specific player identity."

Source: Steamworks Documentation, "Steam Datagram Relay",
`partner.steamgames.com/doc/features/multiplayer/steamdatagramrelay`.

### Curated Industry Resources

The "Awesome Game Networking" list (maintained by Fatih MAR, with contributions
from network programmers across the industry) points to foundational GDC talks:

- Yahn Bernier (Valve) — Half-Life / Team Fortress Networking
- Patrick Wyatt (En Masse, ex-Blizzard) — Writing Server and Network Code
- Joe Rumsey (Blizzard) — World of Warcraft network serialization/routing
- David Aldridge (Bungie) — Halo: Reach networking
- Tim Ford & Philip Orwig (Blizzard) — Overwatch netcode
- Glenn Fiedler (Respawn) — networked physics

Common theme across these talks: authoritative server, validate input, do not
trust the client, and handle identity at the platform/transport boundary rather
than inventing a new account system.

Source: GitHub, `MongkonEiadon/Awesome-Game-Networking`.

## What Successful Games Do

| Concern | Convention |
|---|---|
| Durable identity | Platform ID (SteamID, XUID, PSN ID) or backend account ID |
| Join proof | Platform ticket, signed backend ticket, or signature with client-held private key |
| Session/local ID | Per-session member number or entity handle, not a security boundary |
| Key ownership | Private key never leaves the device; backend only stores public key |
| Ticket lifetime | Short-lived and scoped to a specific server/session |
| Recovery | Platform account recovery handles lost devices/keys |
| Anti-spoofing | Server validates a proof; does not trust client-sent IDs |

## What They Do Not Do

- **Do not** generate a private key on the server and send it to the client.
  That makes the server a single point of compromise and allows the server (or
  anyone who steals server data) to impersonate players.
- **Do not** use session-local IDs as a security boundary. `SessionMemberId`
  is fine for routing gameplay messages, but anyone who knows the value can
  claim it unless it is backed by a verified identity.
- **Do not** roll custom crypto primitives. Use vetted libraries and protocols
  (Ed25519, X25519, Steamworks auth, Web API validation).

## Implications for Afterglow

The current `SessionMemberId(u128)` is the right shape for a session-local
handle, but it is not a player identity. Afterglow should add a parallel
identity layer:

```text
PlayerIdentity
├── Native { proof: NativeIdentityProof }
└── Steam { steam_id: u64, ticket: Vec<u8> }
```

### Native Afterglow identity options

Two valid patterns, with different trade-offs:

#### Option A: Client keypair (device identity)

- Client generates Ed25519 keypair on first launch.
- Stores private key locally (save file, OS keychain, or WASM localStorage).
- Registers public key with backend/account service.
- Join message contains public key + signature over a server nonce.
- Server verifies signature and maps public key to `SessionMemberId`.

Pros: no backend needed for key issuance; works fully offline/self-hosted.
Cons: key loss = identity loss unless paired with account recovery; key storage
is the client's responsibility.

#### Option B: Backend-issued identity ticket/cert

- Client logs into a trusted game coordinator (web service, local-first auth
  server, or Steam-style platform).
- Coordinator issues a short-lived signed ticket/certificate.
- Client presents ticket to game server.
- Game server verifies coordinator's signature.

Pros: supports revocation, banning, expiration, and account recovery naturally.
Cons: requires a running coordinator/backend for auth.

For Afterglow's early goals, **Option A** is sufficient for non-Steam sessions.
A coordinator can be added later as an optional backend.

### Steam identity

When `SessionBackend::Steam` is selected:

- Use `ISteamUser::GetAuthSessionTicket` or
  `ISteamUser::GetAuthTicketForWebApi`.
- Pass the ticket to the host/server through the existing `SessionEvent` flow.
- Server validates the ticket via Steamworks callbacks or Web API.
- Identity becomes `PlayerIdentity::Steam { steam_id }`.

This should be implemented in the future Steam session backend, not in the
non-Steam provider.

### Where identity lives in the session flow

```text
SessionRequest::Create/Join/JoinByCode
    contains PlayerIdentity proof
        |
        v
SessionBackend::NonSteam  -> verify signature or ticket
SessionBackend::Steam     -> verify Steam ticket
        |
        v
AfterglowSessionState stores SessionMemberId (local) + PlayerIdentity (trusted)
        |
        v
Lightyear link connection
```

`SessionMemberId` remains the gameplay-facing handle. `PlayerIdentity` is the
anti-spoofing/persistence handle. They are separate exactly the way `Entity`
(local) and `StableEntityId` (durable) are separate.

## Implemented Slice

The identity slice is now implemented in `crates/afterglow-engine/src/network/session/`.

### What was added

- `PlayerIdentity` enum with `Native` and `Steam` variants.
- `NativeIdentityProof` containing an Ed25519 public key and signature over the
  canonical challenge `"afterglow-session:" + backend + target + nonce`.
- `SessionIdentityNonce` resource initialized from the OS CSPRNG (deterministic
  in tests).
- Identity attached to `SessionRequest::Create` / `Join` / `JoinByCode` and
  stored in `SessionInfo` and `AfterglowSessionState`.
- Non-Steam provider verifies `Native` proofs with
  `ed25519_dalek::VerifyingKey::verify_strict`.
- `NonSteamSessionCatalog` stores a `key_to_member` map so rejoining with the
  same native public key returns the existing `SessionMemberId`.
- Steam identities pass through the non-Steam provider unvalidated.

### Tests added

- Valid native create/join signatures.
- Invalid public key / signature rejected with `PermissionDenied`.
- Rejoin with the same native key returns the same `SessionMemberId`.
- Join with a different native key allocates a new member slot.
- Steam identity create/join passthrough.
- Owner identity exposed in `SessionInfo`.

### What is still deferred

- Steamworks ticket validation in a Steam session backend.
- Persistent account backend, key rotation, and recovery.
- Client-side private key storage strategy (keychain, save file, etc.).
- Replay protection beyond the current single server nonce (e.g., per-session or
  per-request nonces, timestamps, or sequence numbers).

## References

- Patrick Wyatt GDC 2012 talk: GDC Vault "Writing Server and Network Code for
  Your Online Game".
- Steamworks User Authentication and Ownership:
  `https://partner.steamgames.com/doc/features/auth`
- Steam Datagram Relay:
  `https://partner.steamgames.com/doc/features/multiplayer/steamdatagramrelay`
- Awesome Game Networking resources:
  `https://github.com/MongkonEiadon/Awesome-Game-Networking`
- Afterglow session design:
  `docs/research/network-backend-abstraction.md`
- Afterglow Steam research:
  `docs/research/steam-multiplayer.md`
