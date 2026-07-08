# Engine Networking & Replication — Full Refactor Plan

**Status:** Implementation spec
**Date:** 2026-06-26

## Inventory of problems (every fragility, every inelegance)

### A. Session layer (external, should not be in engine)
1. **Dual connection systems**: TCP `NonSteamSessionProvider` + UDP netcode, manually bridged by 3 plugins (`AfterglowSessionPlugin` + `AfterglowSessionLightyearBridgePlugin` + `AfterglowNetcodeConsumerPlugin`).
2. **`MemberLinkMap` keyed by `SessionMemberId` (u128)** instead of `PlayerId` (u64). Translation step everywhere.
3. **`SessionIdentityNonce`** — a shared `[42u8; 32]` nonce, same for all players. Broken crypto.
4. **`PlayerIdentity::demo`** — derives Ed25519 from a `u8 key_seed`, not a real keypair. `key_seed=1` hardcoded in `client_join_flow`. Same key = same identity = collision.
5. **`SessionCode`** — string codes with a generate/allocate/track system. Steam uses `LobbyId` (u64) directly.
6. **`client_join_flow`** state machine — `Idle → Search → SearchSent → Joining → Joined`. Demo-level join logic living in the demo because the session layer is too complex to use cleanly.
7. **0.0.0.0 → 127.0.0.1 address translation** — the session transport reports the server's bind address, which is invalid as a connect target. Patched in `queue_direct_udp_startup`.
8. **`SessionLightyearLinks`** resource — tracks link entities produced by the bridge. Yet another resource to keep in sync.
9. **`PendingNetcodeStartup`** — params queued by the bridge, drained by the consumer. Two-step indirection for what should be one call.
10. **`~2000 LOC`** of session/bridge/consumer/identity code in `network/session/` + `network/lightyear/link/`.

### B. Engine connection layer (netcode setup)
11. **`AfterglowLightyearConfig`** has 11 fields including `server_addr`, `remote_addr`, `connect_token`, `input_delay_ticks`, `rebroadcast_inputs`. Demo sets them all manually. Most should be netcode-internal defaults, not per-demo config.
12. **`configure_input_defaults`** — runs every `Update`, checks `InputDelayConfigured` marker, inserts `InputTimelineConfig` on the client link. Should be done once at link creation, not polled.
13. **`add_replication_sender_on_link_of`** observer — doesn't fire for netcode-spawned `LinkOf` entities (spawn-bundle components don't trigger `On<Add>`). Band-aided by `ensure_replication_sender_and_channels` polling system.
14. **`ensure_replication_sender_and_channels`** — runs every `PreUpdate`, checks every `LinkOf` entity for missing `ReplicationSender` + channels. Should be synchronous in the link lifecycle, not a poll.
15. **`AfterglowNetworkContext` / `AfterglowConnectionStatus`** — wraps session state + role. Every demo system reads `context.as_deref().map(|ctx| ctx.get_connection_status())` to check `runs_authority()` / `is_client_only()`. Should be a simple `LightyearRole` resource.
16. **`AfterglowSessionPlugin`** added to BOTH server and client Apps even though only the server needs the session catalog. Client only needs the session client.
17. **`register_afterglow_lightyear_protocol`** — engine helper that inits `HistoryTick` + registers `StableEntityId` as replicated. Never called in the demo runtime. `HistoryTick` is used only by the harness, not the demo.

### C. Replication & prediction setup
18. **`spawn_player_box` added `Replicate` at spawn** before `PredictionTarget`/`InterpolationTarget` were set → entity replicated as bare `Confirmed`, targets added later didn't retroactively convert. Fixed by removing `Replicate` from spawn and adding it together with targets. But this is a fragile temporal coupling — the fix requires the developer to know "don't add Replicate until targets are ready."
19. **`bind_player_prediction_and_interpolation_target`** — lazy system that adds `Replicate` + targets once the `MemberLinkMap` entry appears. Exists because the link isn't ready at spawn time. Another temporal coupling band-aid.
20. **`attach_replicated_player_visuals`** — waits for `Transform` before attaching visuals (otherwise defaults to `(0,0,0)`). Another timing workaround.
21. **`attach_predicted_player_physics` / `attach_predicted_kinematic_physics`** — `Option<&Transform>` with fallback to zero. Same timing issue.
22. **`PlayerBox.owner` = `"2"`** (member_id as string) vs `PlayerName` = `"alice"`. Every system that finds the local player checks BOTH: `player_box.owner == player_name.0 || local_member == Some(player_box.owner.as_str())`. Identity confusion.
23. **`ServerAddr`** resource — demo-specific resource holding the connect address. Should be in the session/connection layer.
24. **`LocalIdentity`** resource — created but never used (the `client_join_flow` creates its own identity).

### D. Demo structure
25. **11 `run_if(role)` gates** in `mod.rs` — `matches!(config.role, LightyearRole::Server)` vs `Client` vs `Server | Client`. Systems shouldn't need to check role at runtime; the schedule should separate them.
26. **`collect_input` gated to Client** but runs in `FixedPreUpdate` alongside Lightyear's input systems. The gate is correct but the system reads raw `ButtonInput<KeyCode>` which doesn't exist on the server.
27. **`apply_movement`** checks `authority && predicted`, `client_only && !predicted` at runtime. With the server-as-thread model, the server App has no `Predicted` entities, so these checks are dead code.
28. **Visual systems** (`highlight_nearest_box`, `update_highlight_colors`, `draw_ropes`, camera) gated to Client via `run_if`. Correct but verbose.
29. **`spawn_arena` / `spawn_player_box`** had render assets removed (correct), but the split between "server spawns physics-only" and "client attaches visuals" is implicit, not enforced by types.
30. **`MultiplayerBoxesPlugin` added to BOTH server and client** — the plugin has systems for both roles gated by `run_if`. Should be split: `MultiplayerBoxesServerPlugin` + `MultiplayerBoxesClientPlugin` so each App only gets what it needs.

### E. Engine-rpg-harness
31. **`LightyearTestRig`** bypasses sessions entirely — spawns crossbeam/UDP links directly. This is correct for tests but means the rig doesn't exercise the session/connection layer at all. After the refactor, the rig should test through the new `ConnectionPlugin`.

---

## Target architecture

### Module layout (new)

```
crates/afterglow-engine/src/
  network/
    mod.rs                    — re-exports, AfterglowNetworkPlugin (just Lightyear + connection)
    connection/
      mod.rs                  — AfterglowConnectionPlugin, ConnectionEvent, ConnectionConfig
      link.rs                 — link lifecycle: spawn client/server links, configure input delay
      controlled.rs           — ControlledEntityPlugin, MemberLinkMap (keyed by PlayerId u64)
    lightyear/
      mod.rs                  — AfterglowLightyearPlugin (registers Lightyear plugins + input + frame interp)
      protocol.rs             — register_afterglow_protocol (HistoryTick, StableEntityId, Transform, etc.)
    context.rs                — AfterglowNetworkContext (simplified: role + local_player_id + session_id)

crates/afterglow-session/     — NEW crate, external to engine
  src/
    lib.rs                    — SessionProvider trait, SessionEvent, ConnectionParams, PlayerIdentity, PlayerId
    identity.rs               — NonSteam identity: keypair gen, load/store, challenge-response
    non_steam.rs              — NonSteamSessionProvider impl (TCP matchmaking or in-process for host)
    steam.rs                  — SteamSessionProvider impl (stub for now, behind feature flag)

crates/afterglow-engine/src/demos/multiplayer_boxes/
  mod.rs                      — plugin split: server_plugin() + client_plugin() + shared protocol
  scene.rs                    — server: spawn arena + players; client: attach visuals
  movement.rs                 — shared FixedUpdate movement (no role checks)
  rope.rs                     — shared FixedUpdate rope (no role checks)
  rope_visual.rs              — client-only visual systems
  camera.rs                   — client-only camera
  protocol.rs                 — shared protocol constants
```

### Type changes

```rust
// PlayerId replaces SessionMemberId everywhere
pub type PlayerId = u64;

// ConnectionConfig replaces AfterglowLightyearConfig (simplified)
pub struct ConnectionConfig {
    pub role: LightyearRole,           // Client | Server
    pub tick_rate: u64,
    pub input_delay_ticks: u16,
    pub rebroadcast_inputs: bool,
    pub link_conditioner: Option<LightyearLinkConditioner>,
}
// protocol_id + private_key are static constants or generated at server startup,
// NOT in this config. They're in NetcodeServerConfig.

// ConnectionEvent replaces the session event → bridge → consumer chain
pub enum ConnectionEvent {
    Connected { player_id: PlayerId, link_entity: Entity },
    Disconnected { player_id: PlayerId, reason: DisconnectReason },
}

// MemberLinkMap keyed by PlayerId
pub struct MemberLinkMap {
    pub links: HashMap<PlayerId, Entity>,
}
```

### Connection flow (new, clean)

```
SERVER:
  App::new()
    .add_plugins(MinimalPlugins + TransformPlugin)
    .add_plugins(AfterglowCorePlugin)
    .add_plugins(AfterglowLightyearPlugin::server())     // ServerPlugins + input + frame interp
    .add_plugins(AfterglowConnectionPlugin::server())    // link lifecycle + ControlledEntityPlugin
    .add_plugins(AfterglowPhysicsPlugin)
    .add_plugins(MultiplayerBoxesServerPlugin)           // arena + player spawn + movement + rope

  The server App:
    1. Starts a NetcodeServer (bind to listen addr, protocol_id, private_key)
    2. On incoming connection: Lightyear inserts ClientOf
    3. AfterglowConnectionPlugin's On<Add, ClientOf> observer (synchronous):
       a. Insert ReplicationSender
       b. Add replication/input channels to Transport
       c. Populate MemberLinkMap { player_id → link_entity }
       d. Emit ConnectionEvent::Connected { player_id, link_entity }
    4. MultiplayerBoxesServerPlugin listens for ConnectionEvent::Connected:
       spawn player with Replicate + PredictionTarget::Single(player_id)
                                 + InterpolationTarget::AllExceptSingle(player_id)
       ALL IN ONE BUNDLE — no race

CLIENT:
  App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(AfterglowCorePlugin)
    .add_plugins(AfterglowLightyearPlugin::client())     // ClientPlugins + input + frame interp
    .add_plugins(AfterglowConnectionPlugin::client())    // link lifecycle
    .add_plugins(AfterglowPhysicsPlugin)
    .add_plugins(MultiplayerBoxesClientPlugin)           // visuals + camera + input + movement

  The client App:
    1. SessionProvider (external) produces ConnectionParams { server_addr, client_id }
    2. AfterglowConnectionPlugin calls NetcodeClient::new(client_id, server_addr, netcode_config)
    3. On Connected: Lightyear marks the link
    4. AfterglowConnectionPlugin inserts InputTimelineConfig (input delay) — once, at link creation
    5. Replicated entities arrive as Predicted/Interpolated (targets were set at spawn)
    6. MultiplayerBoxesClientPlugin attaches visuals + physics to replicated entities
```

### What the demo needs to do (simplified)

```rust
// Host:
let server = spawn_server_thread(listen_addr);  // server App in thread
let client = run_client(connect_addr, player_name);  // client App

// Client (remote):
let client = run_client(connect_addr, player_name);

// run_client:
fn run_client(addr: &str, player_name: &str) -> AppExit {
    let identity = load_or_create_identity();  // Ed25519 keypair
    let client_id = identity.player_id();

    // Session provider (NonSteam for dev):
    let session = NonSteamSessionProvider::new();
    session.join(addr, &identity);  // challenge-response, produces ConnectionParams

    // App setup:
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_plugins(AfterglowCorePlugin);
    app.add_plugins(AfterglowLightyearPlugin::client());
    app.add_plugins(AfterglowConnectionPlugin::client());
    app.add_plugins(AfterglowPhysicsPlugin);
    app.add_plugins(MultiplayerBoxesClientPlugin);
    app.insert_resource(LocalPlayerId(client_id));
    app.insert_resource(SessionProviderHandle(session));
    app.run()
}
```

### Plugin split (no more run_if(role))

```rust
// Server plugin: only server-side systems
pub struct MultiplayerBoxesServerPlugin;
impl Plugin for MultiplayerBoxesServerPlugin {
    fn build(&self, app: &mut App) {
        register_demo_protocol(app);
        app.add_systems(Startup, (spawn_arena, spawn_lights));
        app.add_systems(Update, (
            spawn_player_on_connected,   // listens for ConnectionEvent::Connected
            despawn_player_on_disconnected,
            bind_player_targets,        // adds PredictionTarget/InterpolationTarget (if not done at spawn)
        ).chain());
        app.add_systems(FixedUpdate, (apply_movement, toggle_rope, sync_rope_joints)
            .chain().before(PhysicsSystems::Prepare));
        app.add_observer(on_rope_link_removed);
    }
}

// Client plugin: only client-side systems
pub struct MultiplayerBoxesClientPlugin;
impl Plugin for MultiplayerBoxesClientPlugin {
    fn build(&self, app: &mut App) {
        register_demo_protocol(app);
        app.add_systems(Startup, (spawn_client_arena_visuals, spawn_lights, client_connect));
        app.add_systems(PreUpdate, (
            attach_predicted_player_physics,
            attach_predicted_kinematic_physics,
        ).after(ReplicationSystems::Receive));
        app.add_systems(Update, (
            attach_replicated_player_visuals,
            attach_replicated_kinematic_visuals,
            add_input_map_to_local_predicted_player,
            setup_camera,
        ));
        app.add_systems(FixedUpdate, (apply_movement, toggle_rope, sync_rope_joints)
            .chain().before(PhysicsSystems::Prepare));
        app.add_systems(PostUpdate, follow_camera_system
            .after(FrameInterpolationSystems::Interpolate)
            .after(RollbackSystems::VisualCorrection));
        app.add_systems(Update, (highlight_nearest_box, sync_kinematic_box_materials, update_highlight_colors, draw_ropes).chain());
        app.add_systems(FixedPreUpdate, collect_input.in_set(InputSystems::WriteClientInputs));
    }
}
```

No `run_if(role)` anywhere. The server App gets the server plugin, the client App gets the client plugin. `apply_movement` / `toggle_rope` / `sync_rope_joints` run on both because both need to simulate — but they don't check role, they just query `With<Predicted>` / `Without<Predicted>`.

---

## Migration sequence

### Phase 1: Extract types (no behavior change)
- Create `crates/afterglow-session/` with `SessionProvider` trait, `SessionEvent`, `ConnectionParams`, `PlayerIdentity`, `PlayerId`.
- Add `ed25519-dalek` + `blake3` deps to the new crate.
- Implement `NonSteamIdentity` (keypair gen, load/store, challenge-response).
- Keep old session code running. No behavior change.

### Phase 2: Engine connection layer
- Create `network/connection/mod.rs` with `AfterglowConnectionPlugin` (server + client variants).
- Move link lifecycle (spawn `NetcodeClient`/`NetcodeServer`, `ReplicationSender` insertion, channel setup) into the connection plugin.
- Replace `On<Add, LinkOf>` observer + polling with synchronous `On<Add, ClientOf>` handler (fires on netcode handshake, not spawn).
- Replace `MemberLinkMap` key from `SessionMemberId` to `PlayerId`.
- Simplify `AfterglowNetworkContext` to `{ role, local_player_id, session_id }`.
- Move `configure_input_defaults` into link creation (one-time, not polled).
- Keep old bridge/consumer running for the demo. No behavior change yet.

### Phase 3: Demo rewrite
- Split `MultiplayerBoxesPlugin` into `MultiplayerBoxesServerPlugin` + `MultiplayerBoxesClientPlugin`.
- Remove all `run_if(role)` gates.
- Replace `PlayerBox.owner = member_id.to_string()` with `PlayerBox.owner = player_id.to_string()`.
- Replace `PlayerName` identity checks with `LocalPlayerId` resource.
- Replace `client_join_flow` state machine with `NonSteamSessionProvider::join()`.
- Replace `ServerAddr` / `LocalIdentity` with `SessionProviderHandle`.
- Spawn players in response to `ConnectionEvent::Connected` (not `SessionEvent::MemberJoined`).
- Set `Replicate` + `PredictionTarget` + `InterpolationTarget` in the spawn bundle (no lazy binding).
- Remove `attach_replicated_player_visuals` Transform-wait workaround (targets set at spawn → Transform arrives with first replication).

### Phase 4: Delete old code
- Delete `network/session/` (entire directory).
- Delete `network/lightyear/link/` (entire directory).
- Delete `AfterglowSessionPlugin`, `AfterglowSessionLightyearBridgePlugin`, `AfterglowNetcodeConsumerPlugin`.
- Delete `SessionLightyearLinks`, `PendingNetcodeStartup`, `SessionIdentityNonce`.
- Delete `SessionCode`, `SessionMemberId`, `PlayerIdentity::demo`, `NativeIdentityProof`.
- Delete `ServerAddr`, `LocalIdentity`, `ClientJoinState`, `client_join_flow`, `client_start_search`.
- Delete `AfterglowLightyearConfig` (replaced by `ConnectionConfig` + static netcode config).
- Delete `add_replication_sender_on_link_of` observer + `ensure_replication_sender_and_channels` system.
- Delete `configure_input_defaults` system + `InputDelayConfigured` marker.
- Delete `bind_player_prediction_and_interpolation_target` lazy system.

### Phase 5: Harness update
- Update `LightyearTestRig` to use `AfterglowConnectionPlugin` instead of direct link spawning.
- OR: keep the rig as-is (test-only bypass) and document that it doesn't test the session layer.

### Phase 6: Cleanup
- Update `docs/api/network.md` with the new architecture.
- Update `AGENTS.md` Lightyear correctness rules.
- Update `docs/ROADMAP.md`.
- Run full test suite + in-game verification.

---

## LOC impact

| Deleted | LOC |
|---|---|
| `network/session/` (all) | ~3500 |
| `network/lightyear/link/` (all) | ~1300 |
| `network/context.rs` (old) | ~130 |
| `network/lightyear/mod.rs` (old parts) | ~200 |
| Demo `client_join_flow` + `client_start_search` + `ClientJoinState` | ~80 |
| Demo `bind_player_prediction_and_interpolation_target` | ~30 |
| **Total deleted** | **~5240** |

| Added | LOC (est) |
|---|---|
| `afterglow-session` crate (trait + types + NonSteam impl + identity) | ~500 |
| `network/connection/` (plugin + link lifecycle + controlled) | ~400 |
| Demo plugin split + `ConnectionEvent`-driven spawn | ~100 |
| **Total added** | **~1000** |

**Net: ~4200 LOC deleted.** The networking stack goes from ~7100 to ~2900 LOC.

---

## Open questions (to resolve before implementing)

1. **Host shortcut?** Does the host's client go through the full `NonSteamSessionProvider::join` flow (search → challenge → ConnectionParams), or does the server thread write connection params to a shared channel the client reads? The full flow is cleaner (one code path); the shortcut is faster for localhost. **Recommendation: full flow for correctness, optimize later if needed.**

2. **NonSteam matchmaking transport.** Keep the TCP provider for remote clients, or switch to in-process channels for the host case only? **Recommendation: TCP for all NonSteam (works for both host-localhost and remote), simplify the TCP provider to ~200 LOC.**

3. **`register_afterglow_lightyear_protocol`.** Keep it (called by `AfterglowLightyearPlugin`), or fold protocol registration into the demo? **Recommendation: engine registers `StableEntityId` + `HistoryTick` + `Transform` (engine-level concerns). Demo registers `PlayerBox`, `KinematicBox`, `RopeLink`, `LinearVelocity`.**

4. **`engine-rpg-harness`.** Update to use `AfterglowConnectionPlugin`, or keep bypass? **Recommendation: keep bypass for now (tests are green, the rig validates prediction/replication correctness, not session layer). Add a separate integration test that exercises the session layer later.**
