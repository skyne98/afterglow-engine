# Multiplayer Boxes Demo — Implementation Summary

## What was built

### New files (crates/afterglow-engine/src/demos/multiplayer_boxes/)

| File | LOC | Purpose |
|------|-----|---------|
| `mod.rs` | 156 | Plugin struct + Plugin impl, `MultiplayerBoxesDemoConfig`, client auto-join state machine |
| `protocol.rs` | 26 | `PlayerBox`, `KinematicBox`, `MoveInput` components; physics constants |
| `scene.rs` | 187 | Arena spawning (floor, 4 walls, 8 kinematic boxes), player box spawn, lights, physics config |
| `camera.rs` | 54 | Top-down follow camera with smooth lerp damping |
| `movement.rs` | 38 | Keyboard input collection (`DemoInput` resource), server-authoritative velocity application |
| `network.rs` | 15 | Lightyear component registration (`PlayerBox`, `KinematicBox`, `MoveInput`, `Transform`) |
| `playground.rs` | 12 | Arena extents helper, `SpawnMarker` component |
| `tests.rs` | 172 | 4 unit tests: type registration, entity counts, camera follow, movement velocity |

### Modified files

| File | Change |
|------|--------|
| `crates/afterglow-engine/src/network/session/identity.rs` | Added `PlayerIdentity::demo(nonce, target, key_seed)` — public constructor for demos without `#[cfg(test)]` |
| `crates/afterglow-engine/src/demos.rs` | Added `pub mod multiplayer_boxes;` |
| `crates/afterglow-engine/src/lib.rs` | Added `run_multiplayer_boxes_demo(config)` entry point |
| `crates/agx/src/main.rs` | Extended CLI with `--host`, `--connect`, `--listen`, `--name-player`; added multiplayer-boxes dispatch; updated tests |

## Tests added

1. **`plugin_builds_and_registers_types`** — MinimalPlugins + PhysicsPlugins, spawn PlayerBox/KinematicBox/MoveInput, verify types exist
2. **`scene_entity_counts_are_correct`** — 8 kinematic boxes, 0 player boxes (pre-join), 1 point light
3. **`camera_setup_follows_player`** — Camera spawns when querying for DemoCamera after setup runs
4. **`movement_sets_velocity`** — DemoInput(Vec2::X) → LinearVelocity.x == PLAYER_SPEED
5. **`fps_cli_rejects_multiplayer_flags`** (agx) — Updated: fps-controller + --connect/--host/--listen/--name-player parse OK
6. **`multiplayer_boxes_requires_host_or_connect`** (agx) — `--name multiplayer-boxes --host` OK, `--name multiplayer-boxes --connect` OK
7. **`host_and_connect_are_mutually_exclusive`** (agx) — Clap `conflicts_with` enforcement
8. **`host_defaults_to_false`** (agx) — `--host` flag sets host=true

## Trade-offs / scope cuts

### 1. `lightyear_avian3d` version incompatibility
`lightyear_avian3d 0.26.4` depends on `avian3d 0.5.0` but the workspace uses `avian3d 0.6.1`. This prevented using the bridge plugin.

**Impact**: Without `LightyearAvianPlugin`, physics state (`Position`/`Rotation`) replication is handled manually. We register `Transform` (with serde support) for replication instead. Avian's `PhysicsTransformPlugin` syncs Transform ↔ Position on both server and client. This works but lacks the interpolation/correction features of the bridge plugin.

- Player boxes and kinematic boxes have `RigidBody::Dynamic` and `Collider` for server-side collision
- `Transform` is replicated so position changes sync to clients
- No root interpolation or prediction correction for physics (use as-is)

### 2. No Lightyear input messages for pure client mode
Used a shared `DemoInput` resource + direct keyboard reading instead of Lightyear message-based input networking. Works for host mode but pure clients can't send input to the server.

**Impact**: Pure client mode (`--connect`) will connect, see the replicated world, but won't be able to move their player on the server. Movement works in host mode only.

### 3. No `LightyearAvianPlugin` physics interpolation
Client-side position updates are raw (each tick the position snaps to the server value). No smoothing, interpolation, or correction.

### 4. Client auto-join implemented but unverified end-to-end
The `client_join_flow` state machine searches the provider, gets the session code, and joins. This path hasn't been tested with two real processes.

## What didn't work / still missing

### Doesn't work yet
- **Pure client movement**: Client reads keyboard but `apply_movement` runs only on host. Client needs Lightyear message-based input to send `MoveInput` to the server.
- **Two-app integration test**: The spec requests a host+client integration test. Without LightyearAvianPlugin and proper input networking, this test isn't wired up yet. The test infrastructure exists (netcode test pattern) but needs the demo-specific protocol and physics replication to be fully exercised.
- **`PredictionTarget`**: Defined on entities but without the bridge plugin, prediction isn't configured. `Predicted` entities won't be created on the client.

### Missing features (from spec)
- **`--prediction` flag**: Not implemented. Would require LightyearAvianPlugin.
- **Player per-client targeting**: All entities replicate to all clients. No per-client visibility.
- **Connection event handling**: No system that spawns a player when a remote client connects.
- **Multiple players**: Only host's player auto-spawns. Remote connections don't get a player entity.

## New TODOs

1. **Add `lightyear_avian3d`** when a compatible version with avian3d 0.6.1 is released (requires avian3d 0.7 or lightyear_avian3d major bump)
2. **Implement message-based input** for pure client movement: register `MoveInput` as a Lightyear message (client-to-server), send from client, receive on server link entities, apply to `PlayerBox`
3. **Add connection handler**: On `ConnectEvent` (Lightyear), spawn a PlayerBox for the new client with `ControlledBy` and per-client `NetworkTarget`
4. **Integration test**: Two-app host+client test confirming entity replication and input flow (follow netcode test pattern)
5. **Benchmark**: Measure replication bandwidth for N kinematic boxes + M players
