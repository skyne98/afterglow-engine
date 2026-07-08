# Player Identity & Authentication

**Status:** Historical / partially superseded
**Date:** 2026-06-26

> Current engine boundary: session/matchmaking is external. The engine consumes
> `LocalIdentity` plus UDP/netcode connection params through
> `AfterglowConnectionPlugin`; NonSteam auth uses the in-engine
> `ChallengeMessage`/`AuthResponse` handshake and `PlayerId = netcode client_id`.
> References below to an engine-owned `SessionProvider` describe an earlier
> design and are retained as background only.

## Goal

Define a unified identity model where:
- **Steam**: `SteamId` (u64) authenticated natively by Steam's backend.
- **NonSteam**: Ed25519 keypair generated on first launch, persisted to disk.
  `client_id = hash(public_key)`. Challenge-response on join.

In the current implementation, the engine's netcode layer does see the
NonSteam challenge-response crypto through `LocalIdentity`; Steam/session
providers remain external and hand resolved connection params to the engine.

## Core types

```rust
/// Stable, unique player identity. This IS the netcode `client_id`.
/// For Steam: SteamId (u64). For NonSteam: hash of Ed25519 public key (u64).
/// Possessing the private key (Steam: Steam backend; NonSteam: local file)
/// IS proof of identity.
pub type PlayerId = u64;

/// A player's identity proof, presented to the session layer on join.
/// The session layer validates it and returns the authenticated `PlayerId`.
pub enum PlayerIdentity {
    /// Steam: the SteamId is already authenticated by Steam's backend.
    /// No challenge-response needed.
    Steam { steam_id: PlayerId },

    /// NonSteam: Ed25519 public key. The `client_id` is derived as
    /// `hash_public_key(&pubkey)`. The session layer challenges the client
    /// to prove possession of the corresponding private key.
    NonSteam {
        public_key: [u8; 32],
        // The private key never leaves the client. It's stored locally
        // and used only to sign the server's challenge.
    },
}

impl PlayerIdentity {
    /// Derive the `PlayerId` (netcode client_id) from this identity.
    /// Steam: returns the SteamId directly.
    /// NonSteam: returns blake3(public_key)[0..8] as u64.
    pub fn player_id(&self) -> PlayerId {
        match self {
            PlayerIdentity::Steam { steam_id } => *steam_id,
            PlayerIdentity::NonSteam { public_key } => hash_public_key(public_key),
        }
    }
}

/// Hash an Ed25519 public key to a u64 player id.
/// Uses blake3 for collision resistance (birthday bound: ~4 billion
/// players before 50% collision chance).
fn hash_public_key(pubkey: &[u8; 32]) -> u64 {
    let hash = blake3::hash(pubkey);
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
}
```

## NonSteam identity lifecycle

### 1. First launch — keypair generation

```rust
/// Load or generate the local player's Ed25519 keypair.
/// Stored at `~/.afterglow/identity.key` (or platform equivalent).
fn load_or_create_identity() -> SigningKey {
    let path = identity_file_path();
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            return SigningKey::from_bytes(bytes[..32].try_into().unwrap());
        }
    }
    // Generate new keypair
    let mut rng = rand::thread_rng();
    let signing_key = SigningKey::generate(&mut rng);
    let verifying_key = signing_key.verifying_key();
    // Store the private key (32 bytes). Permissions: 0600.
    std::fs::write(&path, signing_key.to_bytes()).ok();
    signing_key
}
```

- `identity.key` is 32 bytes (Ed25519 private key seed).
- File permissions `0600` (owner read/write only).
- Persisted across sessions → same `client_id` every time the same player rejoins.
- On Steam: skip this entirely — `SteamId` is the identity.

### 2. Deriving client_id

```rust
let signing_key = load_or_create_identity();
let verifying_key = signing_key.verifying_key();
let public_key = verifying_key.to_bytes();
let client_id = hash_public_key(&public_key);
```

`client_id` is deterministic from the keypair. No server round-trip needed to
know your own id. The server derives the same id from the presented public key.

### 3. Join flow — challenge-response

```
Client                              Server
  │                                   │
  │ ├── join(client_id, public_key) ──▶│
  │                                   │ ├── derive client_id from public_key
  │                                   │ ├── check client_id matches claim
  │                                   │ ├── store client_id → public_key
  │                                   │ ├── generate random challenge nonce
  │ ◀──── challenge(nonce) ────────────│
  │                                   │
  │ ├── sign(nonce, private_key) ─────▶│
  │                                   │ ├── verify(signature, public_key, nonce)
  │                                   │ ├── if ok: emit SessionEvent::Joined
  │ ◀──── ConnectionParams ────────────│
```

The server stores `client_id → public_key` for:
- **Rejoin detection**: same `client_id` = same public key = same player.
  Restore their member slot, session membership, etc.
- **Subsequent challenges**: if the player reconnects, challenge them again
  with a fresh nonce to prove they still have the private key.

### 4. Steam join flow (no challenge)

```
Client                              Server
  │                                   │
  │ ├── join(steam_id) ───────────────▶│
  │                                   │ ├── Steam backend authenticates steam_id
  │                                   │ ├── emit SessionEvent::Joined
  │ ◀──── ConnectionParams ────────────│
```

Steam's networking layer (`ISteamNetworkingSockets`) validates the `SteamId`
during connection. No challenge-response needed — Steam is the identity
provider.

## SessionProvider trait (updated)

```rust
pub trait SessionProvider: Send + Sync + 'static {
    /// Create a new session. The caller becomes the owner.
    /// Uses the local player's identity (SteamId or NonSteam keypair).
    fn create(&mut self, config: &SessionCreateConfig);

    /// Search for sessions matching criteria.
    fn search(&mut self, criteria: &SessionSearch);

    /// Join a session by ID. The session layer validates the local player's
    /// identity (challenge-response for NonSteam, Steam backend for Steam)
    /// and emits SessionEvent::Joined on success.
    fn join(&mut self, session: SessionId);

    /// Leave the current session.
    fn leave(&mut self);

    /// Set lobby metadata (owner only).
    fn set_session_data(&mut self, key: &str, value: &str);

    /// Get lobby metadata.
    fn session_data(&self, key: &str) -> Option<String>;

    /// Set per-member metadata for the local player.
    fn set_member_data(&mut self, key: &str, value: &str);

    /// Get per-member metadata for another player.
    fn member_data(&self, member: PlayerId, key: &str) -> Option<String>;

    /// The local player's authenticated identity.
    /// For Steam: PlayerIdentity::Steam { steam_id }.
    /// For NonSteam: PlayerIdentity::NonSteam { public_key }.
    fn local_identity(&self) -> &PlayerIdentity;

    /// The local player's id (= client_id = hash(pubkey) or SteamId).
    fn local_player_id(&self) -> PlayerId {
        self.local_identity().player_id()
    }

    /// The current session ID, if in one.
    fn current_session(&self) -> Option<SessionId>;

    /// Set the game server address for the session (owner only).
    fn set_game_server(&mut self, addr: SocketAddr);
}
```

## What the engine sees

The engine's netcode layer receives:
- `ConnectionParams { server_addr, client_id }` from `SessionEvent::Joined`
- `client_id` is already authenticated by the session layer
- No crypto logic in the engine — just `NetcodeClient::new(client_id, server_addr, netcode_config)`

The engine's `ClientOf` observer fires on connection:
- Extracts `PeerId::Netcode(client_id)` from `RemoteId`
- `client_id` = `PlayerId` = the authenticated identity
- Populates `MemberLinkMap { client_id → link_entity }`
- Emits `ConnectionEvent::Connected { client_id }`

The game spawns the player:
- `PredictionTarget::to_clients(NetworkTarget::Single(client_id))`
- `InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(client_id))`
- `PlayerBox { owner: client_id.to_string() }` — matches `local_player_id()`

## Security properties

| Property | Steam | NonSteam |
|---|---|---|
| Identity is stable | ✅ SteamId never changes | ✅ Key persisted to disk |
| Can't steal someone's id | ✅ Steam backend | ✅ Need their private key |
| Can't replay a join | ✅ Steam session | ✅ Fresh nonce per join |
| Rejoin detection | ✅ Same SteamId | ✅ Same `client_id` (same pubkey) |
| client_id collision | ✅ Impossible (Steam assigns) | ✅ Negligible (blake3 birthday bound ~4B) |

## What gets deleted from current code

- `network/session/identity.rs` — the old `PlayerIdentity::demo` with `key_seed`
  derivation. Replaced by `load_or_create_identity()` + `hash_public_key()`.
- `SessionIdentityNonce` resource — the nonce is now per-challenge, generated
  server-side in the NonSteam provider.
- `key_seed` derivation in `client_join_flow` — replaced by loading the
  persistent keypair.
- `PlayerIdentity::Native(NativeIdentityProof { public_key, signature })` —
  replaced by the challenge-response flow.
- `ed25519-dalek` direct usage in the demo — moved into the NonSteam
  `SessionProvider` impl.

## Crate dependencies

- `ed25519-dalek` — keypair generation, signing, verification
- `blake3` — public key → u64 client_id hashing (collision-resistant)
- Both are already in the workspace or trivially addable.
