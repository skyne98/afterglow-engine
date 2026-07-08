# Session Transport and Connection Design (Historical)

This note is historical. Afterglow no longer owns session join codes,
matchmaking, lobby discovery, or a session-to-Lightyear bridge.

Current boundary:

- external platform/session tooling resolves an endpoint and identity;
- Afterglow consumes `ServerAddr` / `ServerListenAddr`, `NetcodeConfig`, and
  `LocalIdentity`;
- `AfterglowConnectionPlugin` spawns real Lightyear UDP/netcode links;
- `PlayerId = u64 = netcode client_id`.

The old ideas around `PendingNetcodeStartup`, `SessionRequest`, and join-code
bridges are retained only as background for future external launcher/platform
work. See `docs/api/network.md` for the current engine API.
