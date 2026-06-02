# Lightyear Networking Boundary

## Status

This note supersedes the previous custom `NetworkTransport`/Iroh/Steam backend
abstraction plan. Afterglow should remove the old in-house transport, session,
handshake, replication, prediction, reconciliation, interpolation, baseline, and
interest-management stack and rebuild multiplayer around:

```text
Leafwing Input Manager
  -> lightyear_inputs_leafwing
  -> Lightyear input buffering, transport, replication, prediction, interpolation
  -> Afterglow fixed-input-delay authoritative simulation
  -> Afterglow gameplay systems
```

The old custom network modules remain useful as migration reference only. They
should not be preserved as parallel production paths.

## New Boundary

Afterglow should own only the pieces that are game-specific:

| Layer | Owner |
|---|---|
| Physical/raw input bindings | Leafwing Input Manager plus thin Afterglow action enum |
| Networked input transport | `lightyear_inputs_leafwing` |
| Connection, message channels, replication | Lightyear |
| Client prediction and remote interpolation | Lightyear |
| Gameplay authority rules | Afterglow systems |
| Fixed input-delay command processing | Afterglow gameplay harness/systems |
| Persistence and stable world IDs | Afterglow |
| Presentation cues | Afterglow cue entities, derived from corrected gameplay facts |

This is intentionally less code than the previous architecture. If Lightyear
already owns a networking concern, Afterglow should not duplicate it.

## Removed Concepts

Delete or gut these old engine-level abstractions:

| Old concept | Replacement |
|---|---|
| `NetworkTransport` trait | Lightyear transport/link entities |
| `MemoryTransport` fake backend | Lightyear test transport/backend facilities |
| `NetworkPacket`, `PacketHeader`, `NetChannel`, `DeliveryMode` | Lightyear channels/messages |
| `NetworkSession`, `PeerId`, `NetworkPlayerId` session stack | Lightyear peer/client identity plus Afterglow stable avatar mapping |
| `network::handshake` | Lightyear protocol/config plus optional platform admission layer |
| Custom command wire DTOs | Leafwing action state through Lightyear input messages |
| Custom replication snapshot/delta API | Lightyear component replication |
| Custom client prediction/reconciliation/interpolation | Lightyear prediction/interpolation |
| Custom interest map | Lightyear replication filtering or a later small Afterglow adapter only if needed |
| Reconnect baselines | Lightyear replication/connect flow plus Afterglow persistence |

## Kept Concepts

Keep these concepts, but re-home them above Lightyear:

| Concept | New home |
|---|---|
| `StableEntityId` | Persistence, save/load, and network gameplay identity |
| Chunk/cell membership | Persistence, streaming, and optional Lightyear replication filtering |
| Server-authoritative gameplay validation | Fixed-tick Bevy gameplay systems |
| Late valid command handling | Fixed input delay; process tick `T` at `T + delay` |
| Correction-sensitive presentation | Lightyear reconciliation and entity-backed `PreSpawned` cue outputs |
| RPG scenarios | `engine-rpg-harness` on Lightyear clients/server plus fixed input delay |

## Transport Notes

Do not rebuild Iroh and Steam as generic engine transports in version one of the
rewrite. Use what Lightyear already offers first.

Phase-one transport order:

1. Lightyear local/in-process transport for tests.
2. Lightyear UDP/netcode for native dev multiplayer.
3. Lightyear WebTransport/WebSocket only if browser multiplayer is needed.
4. Lightyear Steam only after core gameplay networking works.
5. No Iroh work unless a concrete Lightyear transport gap remains.

Steam should use Lightyear's Steam support if it fits, while Steam lobbies,
invites, identity proofs, and ownership checks remain a thin platform admission
layer around Lightyear. Iroh is future research, not part of the migration plan.
If it is needed later, implement it as a Lightyear-compatible IO/link layer
rather than reviving the old engine `NetworkTransport` trait.

## Testing Strategy

The replacement test harness should exercise gameplay behavior, not the old wire
format:

| Test area | Required coverage |
|---|---|
| Leafwing input | action press/hold/release, axes, local routes, scripted input |
| Lightyear integration | client/server connect, replicated spawn/despawn, predicted local entity, interpolated remote entity |
| Fixed input delay | late shield blocks arrow, deterministic ordering, stale command rejection |
| Security | spoofed ownership, duplicate/reordered input, impossible commands, stale input window |
| Stress | many clients/NPCs with packet loss/reorder/latency through Lightyear test transport |

## Decision

Use Lightyear as the multiplayer substrate. Use Leafwing as the input substrate.
Build only the missing Afterglow-specific layer: deterministic fixed-tick gameplay
that processes client input after a configured server delay. Do not revive the
old server-rewind history, replay, or correction-diff layer unless a future
feature proves it is necessary.
