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

// Host a listen-server session.
app.session().host_with_endpoint(
    SessionConfig {
        backend: SessionBackend::NonSteam,
        transport: SessionTransport::Netcode,
        ..Default::default()
    },
    my_identity(),
    "0.0.0.0:7777".parse::<SocketAddr>().unwrap(),
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

The API can be implemented as a Bevy `App` extension trait and a thin helper
resource:

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
    pub fn host(
        &mut self,
        config: SessionConfig,
        identity: PlayerIdentity,
    ) {
        self.app.world_mut().write_message(
            SessionRequest::Create(config, identity)
        );
    }

    pub fn host_with_endpoint(
        &mut self,
        config: SessionConfig,
        identity: PlayerIdentity,
        provider: SocketAddr,
    ) {
        // In the full engine, this also starts the provider listener on the given address.
        // The Create request itself remains the same; the provider address is stored
        // or registered by the engine-side listener plugin.
        self.host(config, identity);
        self.app
            .world_mut()
            .resource_mut::<SessionProviderConfig>()
            .listen_addr = Some(provider);
    }

    pub fn join_non_steam(
        &mut self,
        code: SessionCode,
        provider: SocketAddr,
        identity: PlayerIdentity,
    ) {
        self.app.world_mut().write_message(SessionRequest::JoinByCode {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::Udp(provider),
            code,
            identity,
        });
    }

    pub fn join_steam(
        &mut self,
        code: SessionCode,
        identity: PlayerIdentity,
    ) {
        self.app.world_mut().write_message(SessionRequest::JoinByCode {
            backend: SessionBackend::Steam,
            provider: ProviderEndpoint::Steam,
            code,
            identity,
        });
    }

    pub fn join_local(
        &mut self,
        code: SessionCode,
        identity: PlayerIdentity,
    ) {
        self.app.world_mut().write_message(SessionRequest::JoinByCode {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::InProcess,
            code,
            identity,
        });
    }

    pub fn leave(&mut self) {
        self.app.world_mut().write_message(SessionRequest::Leave);
    }

    pub fn status(&self) -> &SessionStatus {
        self.app.world().resource::<SessionStatus>()
    }

    pub fn is_in_session(&self) -> bool {
        self.app.world().resource::<SessionStatus>().is_in_session()
    }
}
```

## Why this hides the message protocol

Most games only need seven operations:

1. Host a game.
2. Host on a specific address.
3. Join by code + provider (NonSteam friend).
4. Join by code (Steam).
5. Join a local session.
6. Leave.
7. Query status.

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
| `host_with_endpoint` | `SessionRequest::Create` + records listener address |
| `join_non_steam` | `SessionRequest::JoinByCode(Udp(addr))` |
| `join_steam` | `SessionRequest::JoinByCode(Steam)` |
| `join_local` | `SessionRequest::JoinByCode(InProcess)` |
| `leave` | `SessionRequest::Leave` |
| `status` | read `SessionStatus` resource |
| `is_in_session` | `SessionStatus::is_in_session()` |

## See Also

- [`session-workflows.md`](session-workflows.md) — detailed sequences for each
  workflow.
- [`network.md`](network.md) — full type and plugin reference.
- [`docs/research/session-transport-connection-design.md`](../research/session-transport-connection-design.md)
  — design trade-offs and future extensions (dedicated server, matchmaker).
