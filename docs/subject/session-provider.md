# Session Provider Notes (Historical)

The engine-owned `SessionProvider` design was retired.

Current boundary:

- matchmaking/session discovery is external to `afterglow-engine`;
- the engine consumes connection parameters and identity only;
- `PlayerId = u64 = netcode client_id`;
- Steam should use `PlayerId = SteamId`;
- non-Steam should use `PlayerId = blake3(Ed25519_public_key)[..8]`;
- gameplay observes `ConnectionEvent` from `AfterglowConnectionPlugin`.

See `docs/api/network.md` for the authoritative networking API. This subject
note remains only as historical design context for future platform tooling.
