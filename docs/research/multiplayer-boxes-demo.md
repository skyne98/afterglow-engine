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

```

- `PlayerBox`, `KinematicBox`, `Transform`, and Avian physics state are registered
  in the Lightyear component protocol.
- Server-spawned player and dynamic box entities use `Replicate::to_clients(All)`.
- Server entities explicitly carry `Transform` so Lightyear replicates pose.
- Bevy mesh/material handles are not replicated. Clients attach local
  `Mesh3d`/`MeshMaterial3d` presentation components to replicated `PlayerBox`
  and `KinematicBox` entities, and spawn local arena floor/wall visuals plus
  static local wall/floor colliders.
- Client-to-server movement uses Lightyear's native Leafwing input path for
  `AfterglowAction`: local `InputMap<AfterglowAction>` components produce
  `ActionState`s, Lightyear buffers them, sends them over `InputChannel`, and
  applies them to the mapped server entity.
- The demo enables Lightyear input rebroadcast for `AfterglowAction` so remote
  predicted entities can receive other players' input history instead of waiting
  only for transform snapshots. Client links receive a fixed two-tick
  `InputTimelineConfig`; this delays server-side consumption, not local predicted
  visuals. Both are now engine-owned defaults (`AfterglowLightyearConfig.rebroadcast_inputs`
  and `AfterglowLightyearConfig.input_delay_ticks`). Keyboard-to-action writes
  run in `FixedPreUpdate` / `InputSystems::WriteClientInputs`, before Lightyear
  buffers inputs and restores delayed snapshots.
- Link transports must include Lightyear's native `InputChannel` in addition to
  replication channels. The engine link setup extends transports instead of
  replacing them, preserving replication metadata/update/action channels.
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
- Host-local keyboard input is represented by a local `InputMap` on the host
  player entity and applies only to the host player's box.
- Remote clients use the same movement system locally on the Lightyear
  `Predicted` player copy. The system writes `ActionState<AfterglowAction>` for
  Lightyear networking, but local predicted presentation reads the current
  `DemoInput` keyboard sample directly so a delayed or rollback-restored zero
  `ActionState` cannot freeze the local player after focus changes.
- The authoritative server reads the same `ActionState<AfterglowAction>` after
  Lightyear maps and applies the client's input message to the server entity.
- Avian handles collision response. Client worlds now include local static arena
  colliders and Lightyear-predicted dynamic cubes so the local player collides
  with the same kind of physics world instead of visually penetrating
  server-only/interpolated props. The workspace builds Avian without `parallel`,
  with `enhanced-determinism`, and with `serialize` for Lightyear component
  replication.

Client-side prediction:
- Session-member player boxes now use Lightyear-native prediction targets:
  `PredictionTarget::Single(PeerId::Netcode(member_id))` for the owner and
  `InterpolationTarget::AllExceptSingle(...)` for everyone else. The host also
  binds numeric-owner `PlayerBox` entities to the matching server-side client
  link with `ControlledBy` once that link exists.
- The demo uses `PlayerOwned::from_member(member)` on server-spawned player
  entities. The engine's `ControlledEntityPlugin` + `MemberLinkMap` (populated
  by the Lightyear bridge) automatically binds `ControlledBy` using the stable
  `SessionMemberId`, not ephemeral `PeerId`. `MemberLinkMap` waits until the
  `ClientOf` link also has `ReplicationSender`, avoiding early `ControlledBy`
  bindings that Lightyear cannot replicate yet. Games only write spawn/despawn
  logic; the engine handles link binding.
- The client renders and simulates the Lightyear `Predicted` copy for its local
  player. Predicted player/cube copies receive local Avian physics components in
  `PreUpdate` after replication receive and before fixed simulation. Remote
  players render through Lightyear `Interpolated` copies, while dynamic cubes are
  predicted to all clients so local player/cube/wall contacts can resolve
  immediately. Confirmed roots are not rendered as gameplay presentation.
- Server snapshots still provide authority/correction through Lightyear's
  rollback/reconciliation machinery. Input transport is now Lightyear/Leafwing;
  no demo-local movement message remains. Discrete rope toggles are still driven
  from `ActionState<AfterglowAction>`, but only the authoritative side writes
  the replicated `RopedTo` component; clients do not locally mutate it. The
  authoritative path uses a per-player release latch, minimum observed press
  duration, and short cooldown so repeated/stale observations of the same
  release cannot attach and immediately detach the cube.
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
   Alice moved Bob. Remote input must use Lightyear's native `InputChannel` on
   the relevant link transports; adding input components without channel wiring
   is not enough.
5. Side checks should stay explicit and simple. `AfterglowNetworkContext` is the
   Fabric-like global resource for querying whether a world runs authority,
   client prediction, host mode, and which `SessionMemberId` belongs to the
   local player.
6. Client replay/correction should be handled by Lightyear, not an engine-local
   smoothing shim. Render the local `Predicted` copy and remote `Interpolated`
   copies; avoid rendering confirmed roots directly as gameplay presentation.
7. Local prediction must include the collision world that affects the local
   player. A predicted player colliding against server-only/interpolated cubes
   will visually penetrate them and then snap out. For interactive dynamic props,
   predict the prop (or at least a deterministic local proxy) and include local
   static colliders for walls/floors.
8. Do not mix predicted `Transform` with predicted Avian `Position`/`Rotation`
   unless the Lightyear-Avian bridge for the exact Avian version owns the sync
   order. Afterglow forks the bridge as `afterglow-lightyear-avian3d` for Avian
   0.6 (Lightyear 0.26's official `lightyear_avian3d` targets Avian 0.5). The
   fork supports `Transform` mode only, which is what the demo uses.
9. Do not drive local presentation from a delayed Lightyear `ActionState`.
   Lightyear may restore an older zero-input snapshot after focus changes,
   timeline sync, or rollback. Write that `ActionState` for networking, mirror
   Leafwing's update/fixed mirrors, and render/move the local predicted actor
   from the immediate keyboard sample.

## Out of scope for v1

- Mouse-look / player rotation
- Animations
- UI / HUD (just keep window title "afterglow — multiplayer boxes")
- Audio
- Multiple physics box sizes
- Box stacking / joints
