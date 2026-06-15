# Session API

Proposed simple public API for Afterglow multiplayer sessions.

This document intentionally scopes out dedicated-server and matchmaker
complexity. The supported flows are:

- **Local** — in-process Crossbeam links for tests and local co-op.
- **NonSteam listen-server** — host exposes a control-plane listener and shares
  the code plus their address with friends.
- **Steam** — Steam lobbies and Steam Datagram Relay hide all addressing.

The lower-level message protocol (`SessionRequest` / `SessionEvent`) remains the
engine's internal contract. The API below is a thin convenience layer on top.

## High-level surface

```rust
use afterglow_engine::network::session::prelude::*;
use std::net::SocketAddr;

// Inside a system or setup function:

// Host a local session.
app.session().host(
    SessionConfig {
        backend: SessionBackend::NonSteam,
        transport: SessionTransport::Local,
        ..Default::default()
    },
    my_identity(),
);

// Host a listen-server session that announces a real UDP/netcode address
// for remote clients to connect to. Start the provider listener first, then
// create the session; the Lightyear transport is separate from the control
// plane.
app.world_mut().insert_resource(
    NonSteamSessionProvider::new("0.0.0.0:7777".parse().unwrap()).unwrap(),
);
app.session().host_with_endpoint(
    SessionConfig {
        backend: SessionBackend::NonSteam,
        transport: SessionTransport::DirectUdp {
            host: "0.0.0.0:5000".to_string(),
        },
        ..Default::default()
    },
    my_identity(),
    "0.0.0.0:7777".parse().unwrap(),
);

// Join a Steam lobby by code.
app.session().join_steam(code, my_identity());

// Join a friend's NonSteam session.
app.session().join_non_steam(
    code,
    "203.0.113.42:7777".parse::<SocketAddr>().unwrap(),
    my_identity(),
);

// Join a local session in the same process.
app.session().join_local(code, my_identity());

// Leave the current session.
app.session().leave();

// Query status.
let status = app.session().status();
assert!(status.is_in_session());
println!("members: {}", status.member_count());
```

The underlying status data is already implemented:

- `AfterglowSessionState::is_in_session()`
- `SessionStatus` resource with `info`, `members`, `state`,
  `is_in_session()`, `member_count()`, `is_error()`

The `app.session().status()` helper is the remaining convenience layer.

## Trait / resource shape

The API is implemented in
`crates/afterglow-engine/src/network/session/api.rs` as a Bevy `App` extension
trait and a thin helper handle. Remote operations (`join_non_steam`,
`search_non_steam`) are sent through `NonSteamSessionClient` so they reach the
networked provider; `host`, `host_with_endpoint`, and `join_local` use the
in-process path.

```rust
pub trait AfterglowSessionExt {
    fn session(&mut self) -> SessionHandle<'_>;
}

impl AfterglowSessionExt for App {
    fn session(&mut self) -> SessionHandle<'_> {
        SessionHandle { app: self }
    }
}

pub struct SessionHandle<'a> {
    app: &'a mut App,
}

impl SessionHandle<'_> {
    pub fn host(&mut self, config: SessionConfig, identity: PlayerIdentity);
    pub fn host_with_endpoint(
        &mut self,
        config: SessionConfig,
        identity: PlayerIdentity,
        provider: SocketAddr,
    ) -> Result<(), std::io::Error>;
    pub fn join_non_steam(
        &mut self,
        code: SessionCode,
        provider: SocketAddr,
        identity: PlayerIdentity,
    );
    pub fn join_steam(&mut self, code: SessionCode, identity: PlayerIdentity);
    pub fn join_local(&mut self, code: SessionCode, identity: PlayerIdentity);
    pub fn search_non_steam(
        &mut self,
        provider: SocketAddr,
        metadata: HashMap<String, String>,
    );
    pub fn leave(&mut self);
    pub fn status(&self) -> &SessionStatus;
    pub fn is_in_session(&self) -> bool;
    pub fn state(&self) -> &AfterglowSessionState;
}
```

## Netcode link consumer

For `SessionTransport::DirectUdp` sessions to open real UDP sockets, also add
[`AfterglowNetcodeConsumerPlugin`](network.md) to the app. It is separate so
unit tests can inspect `PendingNetcodeStartup` without binding sockets, and so
games can override transport establishment if needed.

```rust
app.add_plugins((
    AfterglowLightyearPlugin,
    AfterglowSessionPlugin,
    AfterglowSessionLightyearBridgePlugin,
    AfterglowNetcodeConsumerPlugin,
));
```

## Why this hides the message protocol

Most games only need eight operations:

1. Host a game.
2. Host on a specific address.
3. Join by code + provider (NonSteam friend).
4. Join by code (Steam).
5. Join a local session.
6. Search a provider.
7. Leave.
8. Query status.

The helpers produce the same `SessionRequest`s and consume the same
`SessionEvent`s, so the engine keeps the flexible message protocol while games
get a small surface to learn.

## Identity helpers

A game still needs to produce a `PlayerIdentity`. Two common helpers:

```rust
pub fn native_identity_from_keypair(
    nonce: &[u8; 32],
    keypair: &ed25519_dalek::SigningKey,
) -> PlayerIdentity { ... }

pub fn steam_identity(
    steam_id: u64,
    ticket: Vec<u8>,
) -> PlayerIdentity {
    PlayerIdentity::Steam { steam_id, ticket }
}
```

The native keypair is stored by the game using whatever platform storage it
prefers (keychain, save file, etc.). The engine never sees the private key.

## What is NOT in this scope

| Out of scope | Reason |
|---|---|
| Dedicated server hosting | The engine runs the provider in the hosting player app. |
| Matchmaker / lobby browser | Not needed for friend invites; can be layered later. |
| NAT punch-through relay | Not needed when using Steam SDR or sharing the host address. |
| WebTransport / WebSocket | Future cross-platform web slice. |

## Lifecycle

```text
game calls app.session().host(...)
    |
    v
engine starts NonSteamSessionProvider listener (if needed)
engine allocates SessionCode
    |
    v
SessionEvent::Created / Joined emitted
    |
    v
AfterglowSessionLightyearBridge spawns Lightyear links
    |
    v
gameplay begins
game calls app.session().leave()
    |
    v
engine tears down links, emits SessionEnded
```

## Relationship to lower-level messages

| Helper | Writes |
|---|---|
| `host` | `SessionRequest::Create` |
| `host_with_endpoint` | insert `NonSteamSessionProvider`, then `SessionRequest::Create` |
| `join_non_steam` | `SessionRequest::JoinByCode(Udp(addr))` |
| `join_steam` | `SessionRequest::JoinByCode(Steam)` |
| `join_local` | `SessionRequest::JoinByCode(InProcess)` |
| `search_non_steam` | `SessionRequest::Search(Udp(addr))` |
| `leave` | `SessionRequest::Leave` |
| `status` | read `SessionStatus` resource |
| `is_in_session` | `SessionStatus::is_in_session()` |
| `state` | read `AfterglowSessionState` resource |

## See Also

- [`session-workflows.md`](session-workflows.md) — detailed sequences for each
  workflow.
- [`network.md`](network.md) — full type and plugin reference.
- [`docs/research/session-transport-connection-design.md`](../research/session-transport-connection-design.md)
  — design trade-offs and future extensions (dedicated server, matchmaker).
