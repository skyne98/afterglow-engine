# Session API Notes (Historical)

Afterglow no longer exposes an engine-owned session or matchmaking API.

Session discovery, lobby membership, invites, join codes, NAT traversal, and
platform admission are external to the engine. The current engine networking API
starts from already-known connection parameters (`ServerAddr`,
`ServerListenAddr`, `NetcodeConfig`) plus `LocalIdentity` / `PlayerId`.

See `docs/api/network.md` for the current `AfterglowConnectionPlugin` surface.
This file is retained only as historical design context for possible future
external launcher/session tooling; it is not an implemented engine API.
