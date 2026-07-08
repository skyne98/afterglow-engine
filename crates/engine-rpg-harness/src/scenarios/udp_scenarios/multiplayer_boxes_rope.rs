//! Production UDP/netcode rope regressions for multiplayer_boxes.
//!
//! These run through `AfterglowLightyearPlugin` + `AfterglowConnectionPlugin`,
//! use real Leafwing input (`KeyF`) for rope attach/release, and assert the
//! local visible/predicted rope state never reappears after a local release.

use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    demos::multiplayer_boxes::{
        movement::{
            DemoInput, add_input_map_to_local_predicted_player, apply_movement,
            apply_predicted_movement, collect_input,
        },
        network::register_demo_protocol,
        protocol::{KINEMATIC_BOX_SIZE, KinematicBox, PLAYER_SIZE, PlayerBox, RopeLink},
        rope::{
            LocallyReleasedRopes, RopeJointDetachPending, hide_local_rope_on_physical_release,
            on_rope_link_removed, rope_id_for_input, rope_link_hash,
            suppress_locally_released_rope_reappearances, sync_rope_joints, toggle_rope,
        },
        scene::{attach_predicted_kinematic_physics, attach_predicted_player_physics},
    },
    input::AfterglowAction,
    network::{
        LightyearRole,
        connection::{ConnectionEvent, ConnectionEventKind, PlayerOwned},
    },
};
use avian3d::{prelude::*, schedule::PhysicsSystems};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{
    NetworkTarget, PreSpawned, Predicted, PredictionDisable, PredictionMetrics, PredictionTarget,
    Replicate, ReplicationSystems, client::input::InputSystems,
};
use lightyear_inputs_leafwing::prelude::LeafwingBuffer;

const ALICE_ID: u64 = 1;
const ROPE_BLOCK_SID: StableEntityId = StableEntityId::from_raw(70_004);

fn spawn_player_on_connected_for_rope_test(trigger: On<ConnectionEvent>, mut commands: Commands) {
    let ConnectionEventKind::Connected = trigger.event().kind else {
        return;
    };
    let player_id = trigger.event().player_id;
    let pos = Vec3::new(0.0, PLAYER_SIZE, 0.0);
    commands.spawn((
        PlayerBox {
            owner: player_id.to_string(),
        },
        RigidBody::Dynamic,
        Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
        Position::from(pos),
        Rotation::default(),
        LinearVelocity::ZERO,
        LockedAxes::ROTATION_LOCKED,
        Transform::from_translation(pos),
        ActionState::<AfterglowAction>::default(),
        LeafwingBuffer::<AfterglowAction>::default(),
        PlayerOwned::from_player_id(player_id),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
    ));
}

fn integrate_velocity_for_test(
    mut bodies: Query<(&mut Transform, Option<&mut Position>, &LinearVelocity), With<RigidBody>>,
) {
    const FIXED_DT: f32 = 1.0 / 60.0;
    for (mut transform, position, velocity) in &mut bodies {
        transform.translation += velocity.0 * FIXED_DT;
        if let Some(mut position) = position {
            position.0 = transform.translation;
        }
    }
}

fn apply_rope_pull_for_test(
    links: Query<(
        &RopeLink,
        Has<PredictionDisable>,
        Has<RopeJointDetachPending>,
    )>,
    players: Query<(&PlayerBox, &Transform), Without<KinematicBox>>,
    mut boxes: Query<
        (
            &StableEntityId,
            &mut Transform,
            Option<&mut Position>,
            Option<&mut LinearVelocity>,
        ),
        (With<KinematicBox>, Without<PlayerBox>),
    >,
) {
    const PULL_STEP: f32 = 0.08;
    const TAUT_DISTANCE: f32 = 1.25;
    const FIXED_DT: f32 = 1.0 / 60.0;

    for (link, disabled, detach_pending) in &links {
        if disabled || detach_pending {
            continue;
        }
        let Some((_, player_transform)) = players
            .iter()
            .find(|(player, _)| player.owner == link.player_owner)
        else {
            continue;
        };
        let Some((_, mut box_transform, position, velocity)) =
            boxes.iter_mut().find(|(id, _, _, _)| **id == link.target)
        else {
            continue;
        };
        let delta = player_transform.translation - box_transform.translation;
        if delta.length() <= TAUT_DISTANCE {
            continue;
        }
        let movement = delta.normalize_or_zero() * PULL_STEP;
        box_transform.translation += movement;
        if let Some(mut position) = position {
            position.0 = box_transform.translation;
        }
        if let Some(mut velocity) = velocity {
            velocity.0 = movement / FIXED_DT;
        }
    }
}

fn register_rope_boxes(app: &mut App, role: LightyearRole) {
    register_demo_protocol(app);
    app.add_observer(on_rope_link_removed);
    match role {
        LightyearRole::Server => {
            app.add_observer(spawn_player_on_connected_for_rope_test);
            app.add_systems(
                FixedUpdate,
                (
                    apply_movement,
                    integrate_velocity_for_test,
                    toggle_rope,
                    sync_rope_joints,
                    apply_rope_pull_for_test,
                )
                    .chain()
                    .before(PhysicsSystems::Prepare),
            );
        }
        LightyearRole::Client => {
            app.init_resource::<DemoInput>();
            app.init_resource::<LocallyReleasedRopes>();
            app.add_systems(
                PreUpdate,
                (
                    attach_predicted_player_physics,
                    attach_predicted_kinematic_physics,
                    add_input_map_to_local_predicted_player,
                    suppress_locally_released_rope_reappearances,
                )
                    .after(ReplicationSystems::Receive),
            );
            app.add_systems(
                FixedPreUpdate,
                collect_input.in_set(InputSystems::WriteClientInputs),
            );
            app.add_systems(
                FixedUpdate,
                (
                    apply_predicted_movement,
                    integrate_velocity_for_test,
                    toggle_rope,
                    hide_local_rope_on_physical_release,
                    sync_rope_joints,
                    apply_rope_pull_for_test,
                )
                    .chain()
                    .before(PhysicsSystems::Prepare),
            );
        }
    }
}

fn rope_udp_rig() -> LightyearTestRig {
    LightyearTestRig::new_afterglow_udp(
        2,
        |app| {
            app.add_plugins((bevy::input::InputPlugin, bevy::transform::TransformPlugin));
            app.add_plugins(
                PhysicsPlugins::default()
                    .build()
                    .disable::<PhysicsTransformPlugin>()
                    .disable::<PhysicsInterpolationPlugin>(),
            );
            app.add_plugins(afterglow_lightyear_avian3d::prelude::AfterglowAvianPlugin::default());
            app.insert_resource(Gravity(Vec3::ZERO));
        },
        register_rope_boxes,
        0,
    )
}

fn physics_block_bundle(pos: Vec3) -> impl Bundle {
    (
        KinematicBox { initial_pos: pos },
        RigidBody::Dynamic,
        Collider::cuboid(
            KINEMATIC_BOX_SIZE * 2.0,
            KINEMATIC_BOX_SIZE * 2.0,
            KINEMATIC_BOX_SIZE * 2.0,
        ),
        Position::from(pos),
        Rotation::default(),
        LinearVelocity::ZERO,
        LockedAxes::ROTATION_LOCKED,
        Transform::from_translation(pos),
    )
}

fn find_server_player(rig: &mut LightyearTestRig, owner: u64) -> Option<Entity> {
    let world = rig.server_world_mut();
    let mut q = world.query::<(Entity, &PlayerBox)>();
    q.iter(world)
        .find_map(|(entity, player)| (player.owner == owner.to_string()).then_some(entity))
}

fn find_predicted_entity(
    rig: &mut LightyearTestRig,
    client_id: usize,
    sid: StableEntityId,
) -> Option<Entity> {
    let world = rig.client_world_mut(client_id);
    let mut query = world.query_filtered::<(Entity, &StableEntityId), With<Predicted>>();
    query
        .iter(world)
        .find_map(|(entity, id)| (*id == sid).then_some(entity))
}

fn server_pos(rig: &LightyearTestRig, entity: Entity) -> Vec3 {
    rig.server_world()
        .get::<Transform>(entity)
        .expect("server entity should have Transform")
        .translation
}

fn client_pos(rig: &LightyearTestRig, client_id: usize, entity: Entity) -> Vec3 {
    rig.client_world(client_id)
        .get::<Transform>(entity)
        .expect("client entity should have Transform")
        .translation
}

fn set_client_move(rig: &mut LightyearTestRig, client_id: usize, dir: Vec2) {
    rig.client_world_mut(client_id)
        .resource_mut::<DemoInput>()
        .0 = dir;
    let mut keys = rig
        .client_world_mut(client_id)
        .resource_mut::<ButtonInput<KeyCode>>();
    for key in [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
        keys.release(key);
    }
    if dir.y > 0.0 {
        keys.press(KeyCode::KeyW);
    }
    if dir.y < 0.0 {
        keys.press(KeyCode::KeyS);
    }
    if dir.x > 0.0 {
        keys.press(KeyCode::KeyA);
    }
    if dir.x < 0.0 {
        keys.press(KeyCode::KeyD);
    }
}

fn tap_rope_toggle(rig: &mut LightyearTestRig, client_id: usize) {
    rig.client_world_mut(client_id)
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    rig.advance(2);
    rig.client_world_mut(client_id)
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    rig.advance(1);
}

fn active_rope_count(world: &mut World, owner: u64) -> usize {
    let owner = owner.to_string();
    world
        .query::<(
            &RopeLink,
            Option<&PredictionDisable>,
            Option<&RopeJointDetachPending>,
        )>()
        .iter(world)
        .filter(|(link, disabled, detach_pending)| {
            link.player_owner == owner && disabled.is_none() && detach_pending.is_none()
        })
        .count()
}

fn rope_hash_count(world: &mut World, rope_id: StableEntityId) -> usize {
    let hash = rope_link_hash(rope_id);
    world
        .query::<(&RopeLink, &PreSpawned)>()
        .iter(world)
        .filter(|(link, prespawned)| link.rope_id == rope_id && prespawned.hash == Some(hash))
        .count()
}

fn rollback_metrics(world: &World) -> (u32, u32) {
    world
        .get_resource::<PredictionMetrics>()
        .map(|metrics| (metrics.rollbacks, metrics.rollback_ticks))
        .unwrap_or_default()
}

fn assert_rollback_resim_budget(before: (u32, u32), after: (u32, u32), label: &str) {
    let ticks = after.1.saturating_sub(before.1);
    assert!(
        ticks <= 120,
        "{label}: excessive rollback resimulation; before={before:?}, after={after:?}, delta={ticks}, budget=120"
    );
}

fn assert_no_idle_rollback_resim(before: (u32, u32), after: (u32, u32), label: &str) {
    let ticks = after.1.saturating_sub(before.1);
    assert_eq!(ticks, 0, "{label}: idle rollback resimulation");
}

fn advance_until(
    rig: &mut LightyearTestRig,
    max_ticks: u32,
    mut predicate: impl FnMut(&LightyearTestRig) -> bool,
    reason: &str,
) {
    for _ in 0..max_ticks {
        rig.advance(1);
        if predicate(rig) {
            return;
        }
    }
    panic!("condition not met after {max_ticks} ticks: {reason}");
}

#[test]
fn udp_rope_pull_then_release_while_moving_away_does_not_reappear() {
    let mut rig = rope_udp_rig();
    rig.connect();
    rig.advance(30);

    let block_start = Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0);
    let server_block = rig.spawn_replicated(ROPE_BLOCK_SID, physics_block_bundle(block_start));
    rig.advance(30);
    let client0_block = find_predicted_entity(&mut rig, 0, ROPE_BLOCK_SID)
        .expect("Alice client should receive predicted block");
    let client1_block = find_predicted_entity(&mut rig, 1, ROPE_BLOCK_SID)
        .expect("Bob client should receive predicted block");
    let server_alice = find_server_player(&mut rig, ALICE_ID).expect("server should spawn Alice");

    let rope_id = rope_id_for_input(&ALICE_ID.to_string(), ROPE_BLOCK_SID);
    tap_rope_toggle(&mut rig, 0);
    rig.advance(40);

    assert_eq!(
        active_rope_count(rig.server_world_mut(), ALICE_ID),
        1,
        "server should create exactly one authoritative rope after Alice releases F"
    );
    assert_eq!(
        active_rope_count(rig.client_world_mut(0), ALICE_ID),
        1,
        "Alice should see one active predicted rope after attach"
    );
    assert_eq!(
        active_rope_count(rig.client_world_mut(1), ALICE_ID),
        1,
        "Bob should see Alice's authoritative replicated rope after attach"
    );
    assert!(
        rope_hash_count(rig.client_world_mut(0), rope_id) <= 1,
        "Alice should not have duplicate PreSpawned rope hashes after attach"
    );

    let metrics_before_pull = rollback_metrics(rig.client_world(0));
    set_client_move(&mut rig, 0, Vec2::NEG_X);
    rig.advance(140);
    let metrics_after_pull = rollback_metrics(rig.client_world(0));
    assert_rollback_resim_budget(
        metrics_before_pull,
        metrics_after_pull,
        "stable deterministic rope pull",
    );

    let pulled_server_player = server_pos(&rig, server_alice);
    let pulled_server_block = server_pos(&rig, server_block);
    let player_position = rig
        .server_world()
        .get::<Position>(server_alice)
        .map(|p| p.0)
        .unwrap_or(pulled_server_player);
    let player_velocity = rig
        .server_world()
        .get::<LinearVelocity>(server_alice)
        .map(|v| v.0)
        .unwrap_or(Vec3::ZERO);
    assert!(
        pulled_server_player.x < -0.75 || player_position.x < -0.75,
        "test must move Alice away from the block before release: transform={pulled_server_player:?}, position={player_position:?}, velocity={player_velocity:?}, block={pulled_server_block:?}"
    );
    assert!(
        pulled_server_block.x < block_start.x - 0.05,
        "rope should pull the server block left: start={block_start:?}, after={pulled_server_block:?}"
    );
    assert!(
        client_pos(&rig, 0, client0_block).x < block_start.x - 0.05,
        "Alice should visibly predict/observe the pulled block"
    );
    advance_until(
        &mut rig,
        80,
        |rig| client_pos(rig, 1, client1_block).x < block_start.x - 0.05,
        "Bob should visibly observe the pulled block",
    );

    rig.client_world_mut(0)
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    rig.advance(2);
    let alice_rope_pressed = {
        let world = rig.client_world_mut(0);
        let mut query = world.query::<(&PlayerBox, &ActionState<AfterglowAction>)>();
        query.iter(world).any(|(player, action)| {
            player.owner == ALICE_ID.to_string() && action.pressed(&AfterglowAction::RopeToggle)
        })
    };
    assert!(
        alice_rope_pressed,
        "Alice action state should see RopeToggle pressed before release"
    );
    rig.client_world_mut(0)
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    rig.advance(1);
    let immediate_release_active = active_rope_count(rig.client_world_mut(0), ALICE_ID);
    assert_eq!(
        immediate_release_active, 0,
        "Alice's rope should hide on the release tick before stale snapshots can arrive"
    );
    let metrics_before_release_settle = rollback_metrics(rig.client_world(0));
    for tick_after_release in 0..120 {
        set_client_move(&mut rig, 0, Vec2::NEG_X);
        rig.advance(1);
        let active = active_rope_count(rig.client_world_mut(0), ALICE_ID);
        let hashes = rope_hash_count(rig.client_world_mut(0), rope_id);
        assert_eq!(
            active, 0,
            "Alice's released rope must not visibly reappear at tick {tick_after_release} after release"
        );
        assert!(
            hashes <= 1,
            "Alice must not accumulate duplicate PreSpawned rope hashes after release: tick={tick_after_release}, hashes={hashes}"
        );
    }

    let metrics_after_release_settle = rollback_metrics(rig.client_world(0));
    assert_rollback_resim_budget(
        metrics_before_release_settle,
        metrics_after_release_settle,
        "post-release deterministic settle",
    );

    set_client_move(&mut rig, 0, Vec2::ZERO);
    rig.advance(200);
    let idle_metrics_before = rollback_metrics(rig.client_world(0));
    rig.advance(80);
    let idle_metrics_after = rollback_metrics(rig.client_world(0));
    assert_no_idle_rollback_resim(
        idle_metrics_before,
        idle_metrics_after,
        "post-release idle deterministic settle",
    );

    assert_eq!(
        active_rope_count(rig.server_world_mut(), ALICE_ID),
        0,
        "authoritative rope should eventually be despawned after release"
    );
    assert_eq!(
        active_rope_count(rig.client_world_mut(0), ALICE_ID),
        0,
        "Alice should still have no visible active rope after server catches up"
    );
}
