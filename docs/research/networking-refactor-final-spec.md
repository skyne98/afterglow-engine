# Networking Refactor — Final Implementation Spec

**Status:** Final spec, ready for implementation
**Date:** 2026-06-26

## Architecture

### Core principle: one connection, one identity, synchronous lifecycle

```
┌─────────────────────────────────────────────────────────────┐
│  Engine: AfterglowConnectionPlugin                            │
│  - Spawns NetcodeClient/NetcodeServer                          │
│  - On ClientOf: synchronous observer inserts ReplicationSender,│
│    populates MemberLinkMap, sends challenge (NonSteam) or      │
│    emits Connected (Steam)                                     │
│  - Auth messages over Lightyear ActionsChannel                 │
│  - Emits ConnectionEvent::Connected/Disconnected               │
└───────────────────────────────────────────────────────────────┘
```

### Auth flow (over netcode connection)

**NonSteam:**
1. Client generates Ed25519 keypair (persistent), `client_id = blake3(pubkey)[..8]`
2. `NetcodeClient::new(client_id, server_addr)` connects
3. Server's `On<Add, ClientOf>` observer fires:
   - Insert `ReplicationSender` + channels (synchronous)
   - Populate `MemberLinkMap { client_id → link_entity }`
   - Send `ChallengeMessage { nonce }` to client via ActionsChannel
4. Client receives challenge, signs nonce with private key
   - Sends `AuthResponse { public_key, signature }` to server
5. Server verifies: `blake3(public_key)[..8] == client_id` && `verify(nonce, pubkey, sig)`
   - If ok: emit `ConnectionEvent::Connected { player_id, link_entity }`
   - If fail: disconnect the link
6. Player spawn happens in response to `ConnectionEvent::Connected`

**Steam:**
1. `SteamId` authenticated by Steam backend during `ISteamNetworkingSockets` connection
2. Server's `On<Add, ClientOf>` observer: emit `ConnectionEvent::Connected` directly
3. No challenge-response

### Types

```rust
// PlayerId = netcode client_id = authenticated identity
pub type PlayerId = u64;

// Local player's persistent identity (engine loads/stores this)
pub struct LocalIdentity {
    pub player_id: PlayerId,           // blake3(pubkey)[..8] or SteamId
    pub keypair: Option<SigningKey>,    // None for Steam, Some for NonSteam
}

impl LocalIdentity {
    pub fn load_or_create() -> Self;
    pub fn public_key(&self) -> Option<[u8; 32]>;
}

// Connection event (engine emits, game listens)
pub enum ConnectionEvent {
    Connected { player_id: PlayerId, link_entity: Entity },
    Disconnected { player_id: PlayerId, reason: String },
}

// Connection config (set at App setup)
pub struct ConnectionConfig {
    pub role: LightyearRole,           // Client | Server
    pub tick_rate: u64,
    pub input_delay_ticks: u16,
    pub rebroadcast_inputs: bool,
    pub link_conditioner: Option<LightyearLinkConditioner>,
    pub require_auth: bool,             // NonSteam: true, Steam: false (Steam authenticates)
}

// Netcode constants (static or generated at server startup)
pub struct NetcodeConfig {
    pub protocol_id: u64,
    pub private_key: [u8; 32],
}

// MemberLinkMap keyed by PlayerId
pub struct MemberLinkMap {
    pub links: HashMap<PlayerId, Entity>,
}

// Auth messages (over ActionsChannel)
#[derive(Serialize, Deserialize)]
pub struct ChallengeMessage { pub nonce: [u8; 32] }

#[derive(Serialize, Deserialize)]
pub struct AuthResponse {
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}
```

### Module layout (final)

```
crates/afterglow-engine/src/
  network/
    mod.rs                    — AfterglowNetworkPlugin (Lightyear + Connection + Physics)
    connection/
      mod.rs                  — AfterglowConnectionPlugin (server/client variants)
      link.rs                 — spawn links, configure input delay (one-time)
      auth.rs                 — challenge/response (NonSteam), identity load/store
      controlled.rs           — ControlledEntityPlugin, MemberLinkMap (PlayerId)
    lightyear/
      mod.rs                  — AfterglowLightyearPlugin (registers Lightyear plugins + input + frame interp + protocol)
      protocol.rs             — engine-level protocol registration (StableEntityId, Transform, HistoryTick, LinearVelocity)
    context.rs                — AfterglowNetworkContext (simplified: role, local_player_id)
    mod.rs                    — AfterglowNetworkPlugin (Lightyear + Connection)

  demos/multiplayer_boxes/
    mod.rs                    — server_plugin() + client_plugin() + shared protocol
    server.rs                 — spawn arena, spawn player on Connected, despawn on Disconnected
    client.rs                 — connect, attach visuals, camera, input, highlight
    shared.rs                 — movement, rope, rope_visual (FixedUpdate + Update, no role checks)
    protocol.rs               — demo components (PlayerBox, KinematicBox, RopeLink)
    camera.rs                 — camera setup + follow (client-only)
```

### Engine registers (in AfterglowLightyearPlugin)
- `StableEntityId` (with prediction)
- `Transform` (with prediction + linear correction + interpolation)
- `LinearVelocity` (with prediction)
- `HistoryTick` resource + reflection
- `FrameInterpolationPlugin::<Transform>`
- `InputPlugin::<AfterglowAction>`
- `AfterglowAvianPlugin` (Avian bridge)

### Demo registers (in register_demo_protocol)
- `PlayerBox`
- `KinematicBox`
- `RopeLink` (with prediction)

### Server App
```rust
App::new()
    .add_plugins((MinimalPlugins, TransformPlugin))
    .add_plugins(AfterglowCorePlugin)
    .add_plugins(AfterglowLightyearPlugin::server())
    .add_plugins(AfterglowConnectionPlugin::server(NetcodeConfig { ... }))
    .add_plugins(AfterglowPhysicsPlugin)
    .add_plugins(MultiplayerBoxesServerPlugin)
    .insert_resource(LocalIdentity::load_or_create())
```

### Client App
```rust
App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(AfterglowCorePlugin)
    .add_plugins(AfterglowLightyearPlugin::client())
    .add_plugins(AfterglowConnectionPlugin::client())
    .add_plugins(AfterglowPhysicsPlugin)
    .add_plugins(MultiplayerBoxesClientPlugin)
    .insert_resource(LocalIdentity::load_or_create())
    .insert_resource(ServerAddr(addr))  // simple: just the address to connect to
```

### ConnectionPlugin (server variant)
- Spawns `NetcodeServer` entity (bind to address)
- `On<Add, ClientOf>` observer (synchronous, fires on netcode handshake):
  1. Insert `ReplicationSender` + channels
  2. Populate `MemberLinkMap`
  3. If `require_auth`: send `ChallengeMessage` (store pending auth state)
     Else: emit `ConnectionEvent::Connected`
- Message receiver for `AuthResponse`:
  - Verify `blake3(pubkey)[..8] == client_id` && `verify(nonce, pubkey, sig)`
  - Emit `ConnectionEvent::Connected` or disconnect

### ConnectionPlugin (client variant)
- Reads `ServerAddr` + `LocalIdentity`
- Spawns `NetcodeClient` entity (connect to server)
- `On<Add, Connected>` (or link ready): insert `InputTimelineConfig` (one-time)
- Message receiver for `ChallengeMessage`:
  - Sign nonce with private key
  - Send `AuthResponse { public_key, signature }`

### Demo server plugin
```rust
pub struct MultiplayerBoxesServerPlugin;
impl Plugin for MultiplayerBoxesServerPlugin {
    fn build(&self, app: &mut App) {
        register_demo_protocol(app);
        app.add_systems(Startup, (spawn_arena, spawn_lights));
        app.add_systems(Update, (
            spawn_player_on_connected,    // Query<Without<PlayerBox>, Without<Replicate>> filtered by ConnectionEvent
            despawn_player_on_disconnected,
        ).chain());
        app.add_systems(FixedUpdate, (apply_movement, toggle_rope, sync_rope_joints)
            .chain().before(PhysicsSystems::Prepare));
        app.add_observer(on_rope_link_removed);
    }
}
```

### Demo client plugin
```rust
pub struct MultiplayerBoxesClientPlugin;
impl Plugin for MultiplayerBoxesClientPlugin {
    fn build(&self, app: &mut App) {
        register_demo_protocol(app);
        app.add_systems(Startup, (spawn_client_arena_visuals, spawn_lights));
        app.add_systems(PreUpdate, (attach_predicted_player_physics, attach_predicted_kinematic_physics)
            .after(ReplicationSystems::Receive));
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

### spawn_player_on_connected (no race)
```rust
fn spawn_player_on_connected(
    mut commands: Commands,
    mut events: EventReader<ConnectionEvent>,
    member_links: Res<MemberLinkMap>,
) {
    for event in events.read() {
        let ConnectionEvent::Connected { player_id, link_entity } = event else { continue; };
        let idx = member_links.links.len() as f32;
        let pos = Vec3::new(5.0 + idx * 2.0, PLAYER_SIZE, 0.0);
        // Link is ready → Replicate + targets in one bundle. No race.
        commands.spawn((
            PlayerBox { owner: player_id.to_string() },
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(pos),
            Rotation::default(),
            LinearVelocity::ZERO,
            Transform::from_translation(pos),
            ActionState::<AfterglowAction>::default(),
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::Single(PeerId::Netcode(*player_id))),
            InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(PeerId::Netcode(*player_id))),
        ));
    }
}
```

## Migration phases

### Phase 1: Engine connection + auth infrastructure
- Create `network/connection/mod.rs`, `link.rs`, `auth.rs`, `controlled.rs`.
- Implement `LocalIdentity` (keypair load/store, `blake3` hash).
- Implement `ChallengeMessage` / `AuthResponse` messages.
- Implement `AfterglowConnectionPlugin::server()` + `::client()`.
- Implement `On<Add, ClientOf>` synchronous observer.
- Implement `ConnectionEvent` + `EventReader`.
- Move `MemberLinkMap` to `PlayerId` key.
- Move protocol registration to engine (`StableEntityId`, `Transform`, `LinearVelocity`, `HistoryTick`).

### Phase 2: Demo rewrite
- Split `MultiplayerBoxesPlugin` into `ServerPlugin` + `ClientPlugin`.
- Rewrite `spawn_player_on_connected` (ConnectionEvent-driven, targets at spawn).
- Remove all `run_if(role)` gates.
- Replace `PlayerBox.owner` identity checks with `LocalPlayerId` resource.
- Remove `client_join_flow`, `client_start_search`, `ClientJoinState`, `ServerAddr` (simplified to just connect address), `LocalIdentity` (engine owns it now).

### Phase 3: Delete old code
- Delete `network/session/` (entire directory).
- Delete `network/lightyear/link/` (entire directory).
- Delete `AfterglowSessionPlugin`, `AfterglowSessionLightyearBridgePlugin`, `AfterglowNetcodeConsumerPlugin`.
- Delete `AfterglowLightyearConfig`, `AfterglowNetworkContext` (old), `SessionLightyearLinks`, `PendingNetcodeStartup`, `SessionIdentityNonce`.
- Delete `add_replication_sender_on_link_of`, `ensure_replication_sender_and_channels`, `configure_input_defaults`.
- Delete `bind_player_prediction_and_interpolation_target`.
- Delete `network/interpolation.rs` if unused (check).

### Phase 4: Harness update
- Update `LightyearTestRig` to use `AfterglowConnectionPlugin`.
- Rig sets `require_auth: false` (skip challenge-response in tests).
- Update scenario `register_protocol` calls if engine now handles registration.

### Phase 5: Verify + cleanup
- `bun run check` + `cargo test` (default + test-support).
- In-game test: host + connect, both players visible, movement works.
- Update `docs/api/network.md`.
- Update `docs/ROADMAP.md`.
- Remove old test files for deleted session code.

## LOC impact
- Deleted: ~5240 LOC (session/, link/, old config, demo join flow, lazy binding)
- Added: ~1000 LOC (connection plugin, auth, identity, simplified demo)
- Net: ~4200 LOC deleted

## Crate dependencies to add
- `blake3` (workspace dep) — public key → u64 hashing
- `ed25519-dalek` (already a dep) — keypair gen, sign, verify
- `rand` (for keypair generation)
