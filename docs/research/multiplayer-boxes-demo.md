# Multiplayer Boxes Demo

A runnable demo that exercises Lightyear + Avian3d + the session layer
end-to-end. Two players each control a box on a shared 3D plane, with
top-down angled camera, plus a few physics boxes that get knocked around
when players walk into them.

## Goals

1. **Visually verifiable netcode** — show client prediction, server
   reconciliation, and replication working in a real window.
2. **Interactive** — WASD/arrows to move, mouse-look or static camera,
   ESC to release cursor.
3. **Self-contained** — one `agx` binary with `--name multiplayer-boxes`,
   `--host` or `--connect` flag.

## Run Modes

```bash
# Terminal 1 — host
bun run native -- --name multiplayer-boxes --host --listen 0.0.0.0:5000 \
  --name-player alice

# Terminal 2 — client
bun run native -- --name multiplayer-boxes --connect 127.0.0.1:5000 \
  --name-player bob
```

Or via `cargo run -p agx --` with the same flags.

## Implemented Components

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerBox {
    pub owner: String,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KinematicBox {
    pub id: u32,
    pub initial_pos: Vec3,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveInput {
    pub direction: Vec2,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveInputMsg {
    pub owner: String,
    pub direction: Vec2,
}
```

- `PlayerBox`, `KinematicBox`, `MoveInput`, and `Transform` are registered
  in the Lightyear component protocol.
- Server-spawned player and dynamic box entities use `Replicate::to_clients(All)`.
- Server entities explicitly carry `Transform` so Lightyear replicates pose.
- Bevy mesh/material handles are not replicated. Clients attach local
  `Mesh3d`/`MeshMaterial3d` presentation components to replicated `PlayerBox`
  and `KinematicBox` entities, and spawn local arena floor/wall visuals.
- Client-to-server movement currently uses explicit Lightyear messages on
  `MoveInputChannel`, not shared resources or Leafwing input replication. This
  remains a demo-local bridge; the documented long-term path is Lightyear's
  native Leafwing input buffering.
- `MoveInputChannel` must be added to the client link transport as a sender
  and to each server-side `LinkOf` transport as a receiver. The helper systems
  extend the existing Lightyear `Transport` instead of replacing it, preserving
  replication channels.
- Shared movement systems query `AfterglowNetworkContext::get_connection_status()`
  for side/session facts instead of hiding logic in separate client/server
  implementations.

## Physics setup

- Uses standard `avian3d::PhysicsPlugins`; `lightyear_avian3d` is not used in
  v1 because the available crate versions are not compatible.
- Floor: large static cuboid at y=0.
- Walls: 4 static cuboids around a 20×20 arena.
- 8 `KinematicBox`es scattered on the floor, each ~1m³, dynamic colliders so
  players can shove them.
- Player boxes: 0.8m dynamic colliders, ~50kg, controlled by movement.

## Movement

Shared movement system in `FixedUpdate`:
- Host-local keyboard input applies only to the host player's box, selected by
  `PlayerName`.
- Remote clients use the same movement system locally: when
  `AfterglowNetworkContext` says this world runs client prediction, the system
  matches the local `SessionMemberId` string (for example `"2"`) to the
  replicated `PlayerBox.owner` and applies immediate velocity.
- The authoritative server still receives `MoveInputMsg { owner, direction }`
  and applies velocity only to the matching `PlayerBox`.
- Avian handles collision response (pushing kinematic boxes). The workspace now
  builds Avian without `parallel` and with `enhanced-determinism` for a simpler
  deterministic baseline.

Client-side prediction:
- Session-member player boxes now use Lightyear-native prediction targets:
  `PredictionTarget::Single(PeerId::Netcode(member_id))` for the owner and
  `InterpolationTarget::AllExceptSingle(...)` for everyone else. The host also
  binds numeric-owner `PlayerBox` entities to the matching server-side client
  link with `ControlledBy` once that link exists.
- `Transform` is registered for Lightyear prediction history and visual
  correction (`Isometry3d` correction delta) plus transform interpolation.
- The client renders and simulates the Lightyear `Predicted` copy for its local
  player. Remote players/physics boxes render through Lightyear `Interpolated`
  copies. Confirmed roots are not rendered as gameplay presentation.
- Server snapshots still provide authority/correction through Lightyear's
  rollback/reconciliation machinery. The remaining demo-local bridge is only the
  explicit `MoveInputMsg`; the next step is replacing it with the documented
  Lightyear/Leafwing native input path.
- The runnable test verifies that the remote client receives replicated
  `PlayerBox` entities over real UDP/netcode, gains client-side presentation and
  local physics components on the predicted copy, and that client input moves the
  authoritative server entity.

## Camera

Top-down (3/4):
- Position: above-and-behind player, fixed offset (e.g. (0, 8, -6))
- Look-at: player + small forward look-ahead
- Smooth follow with damping
- Uses replicated `Transform` as the target pose. The server names remote
  player boxes by `SessionMemberId`, so pure clients fall back from
  `PlayerName` to their local session member id (for example `"2"`) when
  choosing the local camera target.

No mouse-look in v1 — keep it simple. Static offset relative to player yaw
(which defaults to facing +Z). Or: arrow keys for movement, no rotation.

Actually, to keep it really simple: **no rotation in v1**. Player boxes move
on the XZ plane with WASD/arrows. Camera follows at fixed angle.

## CLI

Extend `crates/agx/src/main.rs`:
- `--name <demo>` — select demo (`fps-controller`, `multiplayer-boxes`, ...)
- `--host` — server mode (also a client for prediction)
- `--connect <addr>` — pure client mode
- `--listen <addr>` — server listen address (default `0.0.0.0:5000`)
- `--name-player <name>` — local player name (default hostname)
- `--prediction` — opt-in flag to enable prediction (server-only hosts
  don't predict; only clients do)

Validation: `--host` and `--connect` are mutually exclusive.
`--name multiplayer-boxes` requires one of them.

## Files

New:
- `crates/afterglow-engine/src/demos/multiplayer_boxes.rs` — plugin
- `crates/afterglow-engine/src/demos/multiplayer_boxes/protocol.rs` — components
- `crates/afterglow-engine/src/demos/multiplayer_boxes/scene.rs` — floor/walls/boxes
- `crates/afterglow-engine/src/demos/multiplayer_boxes/camera.rs` — top-down follow
- `crates/afterglow-engine/src/demos/multiplayer_boxes/movement.rs` — input → velocity
- `crates/afterglow-engine/src/demos/multiplayer_boxes/network.rs` — server/client/plugin wiring
- `crates/afterglow-engine/src/demos/multiplayer_boxes/playground.rs` — visual demo pieces
- `crates/afterglow-engine/src/demos/multiplayer_boxes/tests.rs` — unit + integration tests

Modified:
- `crates/afterglow-engine/src/lib.rs` — add `run_multiplayer_boxes_demo`
- `crates/agx/src/main.rs` — extend CLI
- `crates/agx/src/main.rs` tests — assert CLI validation

## Tests

1. Plugin builds and registers correctly (unit test, MinimalPlugins)
2. Scene spawns expected entity counts (floor, walls, kinematic boxes)
3. Camera follows a moving entity (unit test)
4. CLI validation: `--host`/`--connect` mutual exclusion
5. CLI validation: invalid combinations rejected
6. Integration: two MinimalPlugin apps, host creates session, client joins,
   Lightyear UDP/netcode links connect, replicated `PlayerBox` entities arrive
   on the client, client-side mesh/material presentation is attached, and
   client input moves the authoritative server entity.

Current regression result: `cargo test -p afterglow-engine --lib --features
multiplayer demos::multiplayer_boxes::tests::net::host_and_client_share` passes.
The full afterglow-engine lib suite reports 321 passed, 0 ignored.

## Debugging Notes

Two Lightyear setup details were required for the integration test to become
honest and pass:

1. Bevy tests that manually call `App::update()` must emulate the runner
   lifecycle with `plugins_state()`, `finish()`, and `cleanup()` after protocol
   registration. Lightyear builds its replication buffer system in
   `ReplicationSendPlugin::finish()`; without this, entities get
   `ReplicationState` entries but `spawned` remains false forever.
2. The session→Lightyear bridge cannot rely on a single `Joined` event.
   `Joined` carries the transport while `MemberJoined` may be the event that
   makes the local `SessionMemberId` valid. The bridge now reconciles DirectUdp
   startup from `SessionStatus` + `AfterglowSessionState` each frame.
3. A logical replicated entity is not automatically renderable. The runtime
   client grey screen happened after networking was fixed because the client
   received protocol components but no mesh/material presentation and the
   camera queried Avian `Position`, which was not replicated. The fix is to
   replicate `Transform`, target the camera by `Transform`, and attach local
   visual prefabs to replicated logical entities.
4. Host input must be scoped to the host-owned player box. The initial server
   movement system wrote the host keyboard velocity to every `PlayerBox`, so
   Alice moved Bob. Remote input also needs explicit `MoveInputChannel`
   receiver wiring on server-side `LinkOf` transports; adding a
   `MessageReceiver` component alone is not enough.
5. Side checks should stay explicit and simple. `AfterglowNetworkContext` is the
   Fabric-like global resource for querying whether a world runs authority,
   client prediction, host mode, and which `SessionMemberId` belongs to the
   local player.
6. Client replay/correction should be handled by Lightyear, not an engine-local
   smoothing shim. Render the local `Predicted` copy and remote `Interpolated`
   copies; avoid rendering confirmed roots directly as gameplay presentation.

## Out of scope for v1

- Mouse-look / player rotation
- Animations
- UI / HUD (just keep window title "afterglow — multiplayer boxes")
- Audio
- Multiple physics box sizes
- Box stacking / joints
