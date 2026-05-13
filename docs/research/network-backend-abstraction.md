# Network Backend Abstraction

## TLDR

Iroh and Steam should be thin platform/network adapters. The engine should own
the stable gameplay protocol, session model, channels, command/snapshot
serialization, prediction, reconciliation, rollback, interest management, and
reconnect baselines.

For the itch-first target, implement Iroh first because it works without Steam.
For a later Steam release, add Steam lobbies, Steam identity/auth, and
SteamNetworkingSockets behind the same backend boundary. Game code should not
change when switching backends.

## Current Engine Pieces

The existing code already has the correct core split:

- `PeerId`: short-lived transport-session endpoint inside one engine session.
- `NetworkPlayerId`: session player identity, owned by a peer.
- `PlatformIdentity`: authenticated external identity such as local, Iroh node,
  Steam ID, or anonymous/dev identity.
- `NetworkTransport`: backend-neutral packet interface.
- `NetChannel` and `DeliveryMode`: engine-owned channel semantics.
- `NetworkSession`: maps peers to platform identities, players, and avatars.
- `network::handshake`: shared reliable control handshake for protocol/build/
  content compatibility, external identity admission, and gameplay packet gating.
- Replication, commands, rollback, prediction, reconciliation, interpolation,
  baselines, and interest management are already above the transport layer.

This means Iroh and Steam must not duplicate those systems. They should only
turn external connection APIs into `TransportEvent`s and translate engine
packets into backend sends.

## Shared Layers

The networking stack should be layered like this:

```text
Game Bevy systems
  |
Replicated components/resources, commands, messages
  |
Engine replication, rollback, prediction, reconciliation, interest
  |
Engine session and protocol layer
  |
NetworkTransport trait plus connection/session events
  |
Iroh adapter       Steam adapter       Memory/fake adapter
```

Only the bottom adapter changes per platform.

## Required Backend Boundary

Keep this interface small:

```rust
pub trait NetworkTransport {
    fn local_peer(&self) -> PeerId;
    fn poll_events(&mut self, out: &mut Vec<TransportEvent>);
    fn send(
        &mut self,
        to: PeerId,
        channel: NetChannel,
        delivery: DeliveryMode,
        payload: Vec<u8>,
    );
    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason);
}
```

Add glue around it only when there is a real shared need:

- backend connect/listen requests
- backend diagnostics
- external identity proof/auth results
- out-of-band session tickets or lobby metadata

Do not put gameplay packets, command parsing, snapshots, rollback, player spawn,
or replicated ECS state into a backend crate.

## Peer And Identity Mapping

Use one mapping path for all backends:

```text
External identity -> PeerId -> NetworkPlayerId -> StableEntityId
```

Examples:

- Iroh: `EndpointId/PublicKey -> PeerId -> NetworkPlayerId -> avatar`
- Steam: `SteamID64 -> PeerId -> NetworkPlayerId -> avatar`
- Memory tests: `Local/Anonymous -> PeerId -> NetworkPlayerId -> avatar`

`PeerId` is not saved. `NetworkPlayerId` is session scoped. `StableEntityId` is
the persistent replicated world identity.

## Channel Mapping

The engine owns channel meaning:

| Engine channel | Delivery | Iroh mapping | Steam mapping |
|---|---:|---|---|
| Control | reliable | reliable stream | reliable message |
| Commands | unreliable sequenced | datagram, or thin sequencer over stream if datagram is unavailable | unreliable message with engine sequence |
| Snapshots | unreliable sequenced | datagram chunks or sequenced stream fallback | unreliable message with engine sequence |
| Events | reliable | reliable stream | reliable message |
| Bulk | reliable | dedicated reliable stream | reliable message/stream-like batching |

The engine sequence in `PacketHeader` remains authoritative for stale
unreliable-sequenced packet rejection. Backends may offer their own sequencing,
but gameplay correctness should not depend on backend-specific behavior.

## Iroh Adapter

Iroh should own:

- endpoint creation and shutdown
- node ID/public key storage
- relay mode and discovery/ticket configuration
- async accept/connect tasks
- QUIC stream/datagram polling
- mapping Iroh connection IDs to `PeerId`
- emitting `TransportEvent::Connected`, `Packet`, and `Disconnected`
- optional diagnostics for direct vs relayed path and connection quality

Iroh should not own:

- player IDs
- replicated ECS snapshots
- command validation
- rollback replay
- save/load baselines
- chunk interest

For itch, this becomes the first real transport because it gives encrypted P2P
connectivity with relay-assisted NAT traversal and no Steam dependency.

## Steam Adapter

Steam should own:

- Steam initialization and callback pumping, either through `bevy_steamworks` or
  direct `steamworks`
- lobby create/join/list/invite flow
- lobby metadata for protocol version, build hash, world/session info, and host
  routing payload
- Steam auth/session ticket validation
- SteamNetworkingSockets connection lifecycle
- mapping Steam connection handles and SteamID64 values to `PeerId`
- emitting backend-neutral transport/session events

Steam should not own the game protocol. A Steam lobby is discovery and handoff,
not the simulation. Steam auth proves platform identity; it does not bypass
server-authoritative command validation.

## Shared Session Flow

Every backend should feed the same engine flow:

1. Backend reports a low-level connection candidate.
2. Engine sends/receives a reliable `Control` hello with protocol version,
   build hash, content hash, backend kind, and external identity proof.
3. Engine validates compatibility and authentication.
4. Engine assigns or restores `PeerId` and `NetworkPlayerId` ownership in
   `NetworkSession`.
5. Server sends a baseline snapshot filtered by interest.
6. Normal command/snapshot/delta traffic begins.

The same handshake packets should be used for Iroh, Steam, memory transport,
and future dedicated-server adapters. Backend-specific auth bytes belong inside
the control handshake payload, not in gameplay systems.

## Testing Strategy

Keep most tests backend neutral:

- protocol version and build/content mismatch
- hello/auth accept and reject paths
- peer reconnect and baseline restore
- duplicate external identity rejection
- channel delivery semantics
- unreliable sequenced stale packet rejection
- disconnect cleanup for session players and avatar ownership
- command spam and hostile clients using `MemoryTransport`

Backend-specific tests should be small:

- Iroh: gated local integration test for two endpoints exchanging packets.
- Steam: manual gated test for lobby, auth, and socket send/receive.

The fake transport remains mandatory even after real backends exist because it
deterministically creates packet loss, duplication, reorder, latency, and
disconnects.

## Implementation Order

1. Tighten the shared transport/session API around the current
   `NetworkTransport`, `TransportEvent`, `NetworkSession`, and `PlatformIdentity`
   types. Done: `network::handshake` provides the shared admission path.
2. Add backend-neutral control handshake packets and tests. Done: hello,
   accept, reject, malformed payloads, mismatch rejection, duplicate identity
   rejection, repeated identity-change rejection, disconnect cleanup,
   same-batch post-reject packet dropping, accepted-message ordering, and
   pre-handshake gameplay packet gating are covered with `MemoryTransport`.
3. Add backend-neutral session lifecycle systems that consume transport events
   and update `NetworkSession`. Done at the helper boundary:
   `service_control_handshake()` polls any `NetworkTransport`, updates
   `NetworkSession`, and forwards only authorized gameplay events.
4. Implement the Iroh backend as an optional native feature.
5. Add Iroh smoke/integration tests behind a feature or environment gate.
6. Implement Steam as an optional native feature using the same handshake,
   channel, session, and packet code.

## References

- Iroh docs: https://docs.rs/iroh/latest/iroh/
- Steamworks Rust docs: https://docs.rs/steamworks/latest/steamworks/
- bevy_steamworks docs: https://docs.rs/bevy-steamworks/latest/bevy_steamworks/
- Steam research note: `docs/research/steam-multiplayer.md`
- Iroh research note: `docs/research/iroh-networking.md`
