# Host Role Removal — Server-as-Thread + Predict-Local/Interpolate-Remote

**Status:** Spec
**Date:** 2026-06-26
**Motivation:** `LightyearRole::Host` (Server+Client plugins in one `App`) forces
dual-role branching in every demo/engine system and breaks host-side rope usage
(`authority && predicted` has no valid `toggle_rope` mode). Replace with:
`agx --host` spawns a server `App` in a thread, then runs a normal client `App`
dialing back to `127.0.0.1`. Roles collapse to `Client | Server` only.

## Goal end state

1. `LightyearRole` is `Client | Server` only (no `Host`).
2. `agx --host` = spawn server thread (MinimalPlugins + ServerPlugins) bound to
   `--listen`, then run the client `App` (DefaultPlugins + ClientPlugins)
   connecting to `127.0.0.1:<listen>`. The client is identical to `--connect`.
3. No system branches on `Host`. `run_if` gates use `Server` or `Client`.
4. `MultiplayerBoxesPlugin` runs in both the server `App` (role=Server) and the
   client `App` (role=Client); `run_if(config.role, …)` selects which systems
   execute per App. No plugin split needed.
5. Predict-local + interpolate-remote: owning client predicts its own player;
   other clients interpolate that player from authoritative Transform snapshots.

## Phase A — Engine core: drop `LightyearRole::Host`

### `crates/afterglow-engine/src/network/lightyear/mod.rs`
- `enum LightyearRole`: remove `Host` variant. Keep `Client` (default), `Server`.
- `AfterglowLightyearPlugin::build`: remove the `LightyearRole::Host =>` match
  arm that adds both `ServerPlugins` + `ClientPlugins`. Only Client/Server arms
  remain.

### `crates/afterglow-engine/src/network/context.rs`
- `AfterglowConnectionStatus`:
  - `runs_authority()` → `self.role == LightyearRole::Server`
  - `runs_client_prediction()` → `self.role == LightyearRole::Client`
  - remove `is_host()`. Update callers.
  - `is_client_only()`/`is_server_only()` stay (still useful).
- Tests: fix the `is_host()` test if present.

### `crates/afterglow-engine/src/network/lightyear/link/mod.rs`
- `queue_direct_udp_startup`:
  - client branch: `matches!(cfg.role, LightyearRole::Client)` (drop `| Host`)
  - server branch: `matches!(cfg.role, LightyearRole::Server)` (drop `| Host`)

## Phase B — `agx` host = server thread + client App

### `crates/afterglow-engine/src/lib.rs`
- `MultiplayerBoxesDemoConfig`: keep `host: bool`, `listen`, `connect`,
  `player_name`.
- `run_multiplayer_boxes_demo(config)`:
  - If `config.host`: spawn a server thread running
    `run_multiplayer_boxes_server(listen)` (see below), then run
    `run_multiplayer_boxes_client(connect=127.0.0.1:listen, player_name)`.
  - Else: run `run_multiplayer_boxes_client(connect, player_name)`.
- Add `run_multiplayer_boxes_server(listen: SocketAddr) -> !` (runs in a
  thread; never returns while the server lives). Server App uses
  `MinimalPlugins` (no rendering) + `AfterglowCorePlugin` +
  `AfterglowLightyearPlugin` (role=Server) + `AfterglowSessionPlugin` +
  `AfterglowSessionLightyearBridgePlugin` + `AfterglowNetcodeConsumerPlugin` +
  `ControlledEntityPlugin` + `AfterglowPhysicsPlugin` + `MultiplayerBoxesPlugin`.
  Host the session on `listen`. Loop `app.run()`.
- Refactor the existing client body into `run_multiplayer_boxes_client(...)`:
  DefaultPlugins + the same plugin set with role=Client + connect to server.
- Server thread lifetime: detach is acceptable for the demo; on client AppExit
  the process exits and the thread is killed. (If a clean shutdown is trivial,
  add a shared `Arc<AtomicBool>` stop flag + `app.exit()`; otherwise detach.)
- **The server and client MUST share all gameplay code/logic.** The only
  difference is which `LightyearRole` and which `DefaultPlugins` vs
  `MinimalPlugins`. `MultiplayerBoxesPlugin` is added to BOTH.

### `crates/agx/src/main.rs`
- `--host` still works; it now means "server thread + client". No CLI change
  needed (the `host: bool` flag flows through). Update the help/error text if it
  implies in-process hosting.

## Phase C — Demo (boxes): rewrite Host branches

### `crates/afterglow-engine/src/demos/multiplayer_boxes/mod.rs`
- Startup `run_if(matches!(config.role, LightyearRole::Host))` (arena + host
  player spawn) → `LightyearRole::Server`.
- `run_if(matches!(config.role, LightyearRole::Client))` stays.
- `run_if(matches!(config.role, LightyearRole::Host | LightyearRole::Client))`
  (FixedUpdate rope/movement) → `LightyearRole::Client` (only clients run input
  + rope toggle; server runs authoritative rope via the Server branch). Wait —
  the server ALSO runs `toggle_rope`/`sync_rope_joints` authoritatively. So
  this gate should be `Server | Client` (i.e. always, both roles run it). Verify
  against the existing semantics: the chain runs on Host|Client today; Host
  covered the server-authoritative path. After removal, the server App
  (role=Server) must also run it. Change to `LightyearRole::Server |
  LightyearRole::Client` (or drop the gate — every role runs it).

### `movement.rs` — `apply_movement`
- Currently: `if authority && predicted { continue; }`. With no Host, the
  server App has no `Predicted` entities (server entities are authoritative, no
  `PredictionTarget` from the server's own App — but the server App DOES
  receive replicated... no, the server is the authority, it doesn't receive its
  own entities as predicted). So `predicted` is always false on the server App.
  The `authority && predicted` guard becomes dead but harmless. Keep it as a
  defensive guard, or drop it. Prefer: drop it and let `apply_movement` move
  every entity with an `ActionState` (matches harness `move_players`).

### `rope.rs` — `toggle_rope`, `sync_rope_joints`, `rope_should_drive_physics`
- `toggle_rope` mode selection:
  - `authority && !predicted` → `Authoritative` (server). Stays.
  - `client_only && predicted && is_local` → `ClientPredicted`. Stays.
  - The `authority && predicted` hole (host) no longer occurs (no Host role,
    server App has no Predicted entities). So the mode selection is now
    correct without changes. Verify.
- `rope_should_drive_physics`: `authority` branch (Server) → true; `client_only`
  branch → local owner only. The `else { true }` branch was for Host; with no
  Host, simplify to `authority || (client_only && local_owner)`. Verify the
  `else { true }` is unreachable now and remove it.
- Add the **rollback guard** to `toggle_rope` (separate bug fix, symptom 3):
  `rollback: Query<(), With<Rollback>>` + early return. Matches `collect_input`.

### `scene.rs` — predict-local + interpolate-remote (Phase E)
See Phase E.

## Phase D — Harness: drop dead Host branches

### `crates/engine-rpg-harness/src/rig/setup.rs`
- `lightyear_app_udp` + `add_crossbeam_lightyear_plugins`: remove the
  `LightyearRole::Host =>` arms. The rig only ever constructs Server/Client
  apps (dead code).
- Verify no scenario passes `LightyearRole::Host` to the rig.

### Scenarios (`corners.rs`, `prediction.rs`, `udp_scenarios/*`)
- `matches!(role, LightyearRole::Client | LightyearRole::Host)` →
  `LightyearRole::Client`. These gates select client-only setup (InputPlugin,
  write_desired_input). Host was dead (rig never passes Host).

## Phase E — Predict-local + interpolate-remote

### `crates/afterglow-engine/src/demos/multiplayer_boxes/scene.rs`
- `player_prediction_target(owner)` → predict only to the owning client.
  Requires the owning client's id at spawn time. On the server, when spawning a
  player on `MemberJoined`, we have `PlayerOwned { member }` →
  `MemberLinkMap::link_for(member)` → the `ClientOf` link → its `RemoteId` →
  `PeerId::Netcode(id)` → `NetworkTarget::Single(id)`.
  - BUT: at spawn time the link may not exist yet (`MemberLinkMap` is populated
    by `update_member_link_map` in `Update`). The host's own player is spawned
    in `Startup` before any client connects. For the host's own player, there
    is no remote client — it's the local client. Hmm.
  - **Reconsider:** with the new model (server is a separate App, host's client
    is a normal client), the server spawns ALL players (including the host's)
    on `MemberJoined`. The host's client connects like any other. So every
    player has an owning client link by the time gameplay runs. But at the
    moment of `spawn_player_on_member_joined`, the `ClientOf` link may not yet
    be replicated-ready. `ControlledBy` binding already gates on
    `ReplicationSender` readiness (see `update_member_link_map` retain logic).
  - **Simplest correct approach:** spawn the player with
    `PredictionTarget::to_clients(NetworkTarget::All)` initially is WRONG now.
    Instead, set `PredictionTarget` lazily: spawn without it, then a system
    adds `PredictionTarget` once the owning link is ready (similar to how
    `bind_controlled_entities` adds `ControlledBy` lazily). OR compute the
    target at spawn if the link is already present.
  - **Alternative:** Lightyear's `PredictionTarget` can be set to
    `NetworkTarget::Single(peer_id)`. For the host's own player on the server
    App, the server doesn't predict (it's authoritative) — so
    `PredictionTarget` on the server's spawn only affects which CLIENTS get
    predicted copies. So we set `PredictionTarget::to_clients(NetworkTarget::Single(owner_peer_id))`.
  - This needs a follow-up system that, for each player entity with
    `PlayerOwned` but a `PredictionTarget::All` (or none), resolves the owner
    link's `RemoteId` and replaces it with `PredictionTarget::Single(...)`.
  - `player_interpolation_target` → `InterpolationTarget::to_clients(NetworkTarget::All)`
    but EXCLUDING the owner. Lightyear's `InterpolationTarget` may support
    exclusion, or we set it to All and rely on PredictionTarget::Single not
    overlapping. **Check Lightyear API:** can `InterpolationTarget` exclude a
    single client? If not, predict-to-owner + interpolate-to-all is the
    standard pattern and Lightyear resolves the overlap (predicted entity on
    the owner, interpolated on others). Per AGENTS.md, avoid Predicted+Interpolated
    on the SAME entity to the SAME client — which this satisfies (owner gets
    predicted, others get interpolated).
- Add `ActionState::<AfterglowAction>::default()` to `spawn_player_box` (fixes
  symptom 2a startup stutter; matches harness). Low risk.
- Add `With<Predicted>` filter to `follow_camera_system` query (fixes latent
  camera-follows-Confirmed bug, symptom 2c).
- `attach_replicated_player_visuals`: interpolated entities need
  `FrameInterpolate::<Transform>` too (currently only predicted get it). Add it
  for the interpolated branch so remote players render smoothly.

## Verification (regression loop)

1. `bun run check` (workspace) passes.
2. `cargo test` (default + `--features test-support`) passes. The harness
   corners/prediction tests are the oracle for prediction correctness.
3. Manually: `agx --name multiplayer-boxes --host` starts a server thread +
   client; `agx --name multiplayer-boxes --connect 127.0.0.1:5000` joins.
   Remote player movement is smooth (interpolated). Local rope toggle does not
   misfire under rollback.
4. No `LightyearRole::Host` references remain repo-wide (except historical docs).

## Out of scope (separate tasks)
- The pre-existing cfg-gate build bug (`cargo check -p afterglow-engine`
  default features fails — `demos.rs`/`network/mod.rs` lack
  `#[cfg(feature = "lightyear")]` gates). Fix separately.
- Flaky `corners`/`prediction` UDP tests under parallel load.
