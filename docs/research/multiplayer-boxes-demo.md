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

## Components

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerBox {
    pub owner: PlayerName,        // "alice" / "bob" / etc
    pub color: Color,             // distinct per player
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KinematicBox {
    pub id: u32,                  // 0..N, server-assigned
    pub initial_pos: Vec3,        // for respawn
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveInput {
    pub direction: Vec2,          // normalized in input system
}
```

- `PlayerBox` is `Replicate`d with `PredictionTarget::to_clients(All)`,
  pre-spawned on server at session join. Server authoritative.
- `KinematicBox` is replicated, server-authoritative. Position/Rotation
  are the Avian `Position`/`Rotation` components (via `lightyear_avian3d`'s
  default `Position` replication mode).
- `MoveInput` flows via Leafwing `ActionState` → Lightyear's input plugin.

## Physics setup

- `lightyear_avian3d` with `AvianReplicationMode::Position` (server physics
  authoritative, predict on client).
- Floor: large static cuboid at y=0.
- Walls: 4 static cuboids around a 20×20 arena.
- 8 `KinematicBox`es scattered on the floor, each ~1m³, dynamic
  colliders so players can shove them.
- Player boxes: 0.8m dynamic colliders, ~50kg, controlled by movement.

## Movement

Server-side system in `FixedUpdate`:
- Read `MoveInput.direction` from authoritative player's entity
- Apply linear velocity = direction * speed (5 m/s)
- Avian handles collision response (pushing kinematic boxes)

Client-side prediction:
- Same system runs in `FixedUpdate` on Predicted entity
- Lightyear rolls back on server correction, re-applies buffered inputs

## Camera

Top-down (3/4):
- Position: above-and-behind player, fixed offset (e.g. (0, 8, -6))
- Look-at: player + small forward look-ahead
- Smooth follow with damping

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
   server-authoritative physics runs, both observe each other.

## Out of scope for v1

- Mouse-look / player rotation
- Animations
- UI / HUD (just keep window title "afterglow — multiplayer boxes")
- Audio
- Multiple physics box sizes
- Box stacking / joints
