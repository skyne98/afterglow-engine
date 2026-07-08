# Session Workflows (Historical)

Afterglow no longer owns session discovery or matchmaking workflows.

Current runtime flow:

1. External tooling/platform code discovers a server/lobby and chooses a
   connection endpoint.
2. The game starts `AfterglowConnectionPlugin::server` with `ServerListenAddr`
   or `AfterglowConnectionPlugin::client` with `ServerAddr`.
3. `LocalIdentity` supplies `PlayerId`; `PlayerId` is the Lightyear netcode
   `client_id`.
4. Gameplay listens for `ConnectionEvent` and spawns/despawns authoritative
   entities.

Steam/non-Steam lobby UX, invite flow, join codes, and NAT traversal belong in a
launcher/platform layer outside the engine. This document is retained only as
historical workflow context and should not be read as current API.
