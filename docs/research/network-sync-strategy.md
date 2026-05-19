# Network Sync Strategy

This note formalizes how Afterglow assigns network behavior per entity. The goal
is to keep gameplay trust server-authoritative while preserving local first-
person feel and avoiding one-size-fits-all smoothing.

## Strategy Taxonomy

| Strategy | Entity class | Authority | Client presentation | Current status |
|---|---|---|---|---|
| Owned predicted avatar | Locally controlled player bodies | Server validates authoritative body state | Local fixed controller predicts; server snapshots acknowledge commands and correct large divergence | Strategy documented; needs a non-demo implementation |
| Remote avatar snapshot | Debug mirrors and non-critical one-off state | Server | Direct replicated transform mirroring | Available as a fallback |
| Buffered interpolation | Replicated physics props, projectiles, breakables, remote avatars that need smooth motion | Server/master | Render samples from a delayed snapshot buffer | Implemented as a generic transform buffer |
| Chunk interest filter | Replication routing by chunk/area | Server | Peers receive entities in interested chunks | Implemented as `ChunkInterestPeer` + `PeerChunkInterest`; Lightyear target consumption is a later routing slice |
| Rewind tracked gameplay | Combat truth: health, shields, inventories, projectiles, hit facts | Server rewind domain | Client receives correction facts after replay | History capture and mock RPG correction path implemented; lifecycle/correction publication still expanding |
| Local only | Cameras, UI, debug helpers, non-network presentation children | Local world | Not replicated | Implemented by absence of network markers |

## Assignment Rules

- Gameplay truth uses `StableEntityId`; raw Bevy `Entity` values never cross the network.
- A locally controlled player body uses owned prediction plus authoritative correction.
- The camera attached to a local player is local-only presentation, even when the body is networked.
- Non-local avatars may start as direct snapshots, but twitch/gameplay-facing motion should move to buffered interpolation before real latency tests.
- Arbitrary physics objects should use buffered interpolation unless the local client owns an explicit interaction mode such as grab/link/release.
- Rewind-tracked entities opt into `RewindedEntity` and component history when late input can change combat truth.
- Debug/UI/console/camera entities stay local-only unless a gameplay feature explicitly needs replication.

## Current FPS Demo Mapping

| Entity/data | Strategy | Notes |
|---|---|---|
| `FpsDemoPlayer` body | Local only | Local fixed controller remains the only movement path. The demo no longer attaches replicated avatar state, prediction buffers, input command messages, or network correction snapshots. |
| `FirstPersonCameraRig` | Local only | Render-rate look, eye-height smoothing, bob, FOV, and overstep presentation read the local motor/body and never materialize network avatars. |
| `PhysicsKinematicRemote` objects | Buffered interpolation | Fixed interaction ticks write authoritative transform samples into `NetworkTransformInterpolationBuffer` for server/master-driven objects. |
| Mock RPG combat entities | Rewind tracked gameplay | The mock harness drives late shield/death/pickup/inventory correction through Lightyear messages and server rewind history. |

## Buffered Interpolation Design

Buffered interpolation is implemented as a small transform sample component, not
a revival of the deleted legacy interpolation stack.

Implemented behavior:

- Store snapshots as `(tick, translation, rotation)` in a bounded ring.
- Render from `current_network_tick - interpolation_delay_ticks`.
- Lerp/slerp between the two nearest snapshots.
- Hold the newest sample when the render tick is newer than the buffer can cover.
- Snap and clear the buffer for teleports or discontinuities above a configured threshold.
- Keep physics authority server/master-owned; interpolated transforms are presentation for remote observers.

Covered regression envelope:

- interpolation between two adjacent samples returns an in-between transform
- missing samples hold or interpolate over gaps deterministically
- stale/duplicate samples do not reorder the ring incorrectly
- teleport threshold clears the buffer and resumes from the new sample
- transform interpolation buffers cover generic remote presentation; the FPS demo no longer owns remote avatar mirrors

## Lightyear Mapping

Afterglow should keep using Lightyear for connection, channels, replication,
prediction metadata, and interpolation hooks where they fit. The current FPS demo
is now local-only because it is a controller regression harness; general-purpose
entities should prefer Lightyear prediction or interpolation components once
their strategy assignment is explicit.

`Replicate`/registered components move state. `StableEntityId` identifies the
entity. Strategy-specific Afterglow systems decide whether received state is
applied directly, replayed as correction, stored in an interpolation buffer, or
captured in server rewind history.
