You are implementing a new multiplayer demo for afterglow-engine. The full
spec is at `docs/research/multiplayer-boxes-demo.md` — read it first.

Context you need:

- `crates/afterglow-engine/src/lib.rs` defines `run_default_demo` and
  `run_fps_controller_demo` as Bevy entry points. The fps demo lives at
  `crates/afterglow-engine/src/demos/fps_controller.rs` and is a good
  structural reference.
- The session API is at `crates/afterglow-engine/src/network/session/api.rs`
  — use `AfterglowSessionExt` and the high-level `app.session().host`,
  `host_with_endpoint`, `join_non_steam`, `search_non_steam` helpers.
- Subject docs cover the engines we use:
  - `docs/subject/lightyear.md` — networking, prediction, components
  - `docs/subject/lightyear-avian3d.md` — physics replication
  - `docs/subject/avian3d.md` — physics API
- Existing integration test that exercises the netcode link is at
  `crates/afterglow-engine/src/network/session/tests/netcode.rs` — copy
  its Bevy-app setup pattern (MinimalPlugins, `AfterglowLightyearPlugin`,
  `AfterglowSessionPlugin`, `AfterglowSessionLightyearBridgePlugin`,
  `AfterglowNetcodeConsumerPlugin`).
- `AfterglowLightyearConfig` lives at
  `crates/afterglow-engine/src/network/lightyear/link/mod.rs`. It has a
  `role: LightyearRole` (Host | Server | Client) and `netcode_private_key`.
- `crates/afterglow-engine/src/network/session/mod.rs` exports
  `SessionBackend`, `SessionConfig`, `SessionTransport`, `SessionVisibility`,
  `SessionCode`, `PlayerIdentity`, etc.
- `crates/agx/src/main.rs` is the CLI entry point with `--name` selecting
  the demo.

Implementation guidance:

- **File budget: no file above 500 LOC.** Split into modules under
  `crates/afterglow-engine/src/demos/multiplayer_boxes/`:
  - `mod.rs` — plugin struct + Plugin impl
  - `protocol.rs` — networked components (`PlayerBox`, `KinematicBox`)
  - `scene.rs` — floor, walls, kinematic box spawn
  - `camera.rs` — top-down follow camera system
  - `movement.rs` — input → linear velocity (server-authoritative)
  - `network.rs` — Lightyear plugin/Protocol setup, replication config
  - `playground.rs` — visual marker boxes / named spawn points
  - `tests.rs` — unit + integration tests

- **Components must be networked.** Use Lightyear's
  `#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]`
  and `Replicate` + `PredictionTarget::to_clients(All)` patterns. Read
  `docs/subject/lightyear.md` sections 4.4–4.7 for component registration.

- **Physics replication:** use `lightyear_avian3d` with
  `AvianReplicationMode::Position`. Player boxes are Dynamic bodies;
  kinematic boxes are Dynamic too (so players can push them).

- **Top-down camera:** fixed offset relative to player position. Position
  the camera at `player_pos + Vec3::new(0.0, 8.0, -6.0)` (above and
  slightly behind). Look-at = player position + small forward look-ahead.
  Smooth follow with exponential damping (lerp factor ~10/s).

- **Movement:** server-authoritative. WASD/arrows → `MoveInput { direction }`
  via Leafwing `InputMap`. Server reads direction in `FixedUpdate`,
  sets `LinearVelocity::from(direction * speed)`. Avian handles collision
  response — that's how players push kinematic boxes.

- **CLI extension:** add `--host`, `--connect`, `--listen`, `--name-player`
  to `agx`. Mutually exclusive: `--host` xor `--connect`. `--listen`
  defaults to `0.0.0.0:5000`. `--name multiplayer-boxes` requires one
  of them. Update existing fps_cli_rejects_multiplayer_flags test to
  use multiplayer-boxes as the positive case.

- **lib.rs entry:** add `pub fn run_multiplayer_boxes_demo()` mirroring
  `run_fps_controller_demo`. The demo function should read CLI args
  (or accept them via env or a builder) and configure the Bevy app
  appropriately.

- **Server vs client app differences:**
  - Host: role=Host, listens for clients, runs server plugins + client
    plugins (for prediction), hosts a `NonSteamSessionProvider`
  - Pure client: role=Client, connects to host via `join_non_steam`
  - Show different cursors / window titles for clarity? Just title for v1.

- **Tests:** at minimum:
  1. Plugin builds and registers components (MinimalPlugins, no networking)
  2. Scene spawns expected entity counts (1 floor, 4 walls, 8 kinematic
     boxes, 0 player boxes pre-join)
  3. Camera follow system updates camera position when target moves
  4. CLI: `--host` and `--connect` mutual exclusion validated
  5. CLI: invalid combos rejected
  6. Two-app integration: host + client via `NonSteamSessionProvider`,
     both see player entities after join (reuses the netcode test pattern)

- **AGENTS.md rules:** semver, semantic commits, no files above 500 LOC,
  tests for everything (especially edge cases), exhaustive edge-case
  testing for the algorithm (collision response, prediction, reconciliation).

Verification steps:
1. `cargo check -p afterglow-engine --features multiplayer` clean
2. `cargo test -p afterglow-engine --features multiplayer` all green
3. `cargo test -p agx` all green (CLI validation)
4. `cargo check -p agx` clean
5. `bun run check` clean

When you're done, write a summary to `/home/fox/Project/afterglow-engine/.work-out/multiplayer-boxes-summary.md`:
- What you built (file list + LOC per file)
- What tests you added
- What trade-offs / scope cuts you made
- What didn't work or what's still missing
- Any new TODOs for follow-up

Important: think carefully and exhaustively about this. Use maximum
reasoning effort. Consider all edge cases, cross-cutting concerns, and
subtle interactions before responding.
