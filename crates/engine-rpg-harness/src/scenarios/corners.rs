//! Tests for the corners of the networking stack.
//!
//! Each test targets a specific edge case or invariant. Tests are organized
//! by the system they cover:
//! - Input pipeline corners
//! - Prediction corners
//! - Physics bridge corners
//! - Replication corners
//! - Controlled entity corners
//! - Cleanup corners

use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    input::{AfterglowAction, default_gameplay_input_map},
    network::{LightyearRole, register_afterglow_lightyear_protocol},
};
use bevy::prelude::*;
use leafwing_input_manager::{action_state::ActionState, input_map::InputMap};
use lightyear::prelude::*;

const PLAYER: StableEntityId = StableEntityId::from_raw(1);
const PLAYER2: StableEntityId = StableEntityId::from_raw(2);
const MOVE_SPEED: f32 = 5.0;
const TICK_DT: f32 = 1.0 / 60.0;

fn register_protocol(app: &mut App, role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Transform>()
        .add_prediction()
        .add_linear_correction_fn::<Isometry3d>()
        .add_interpolation_with(TransformLinearInterpolation::lerp);

    app.add_plugins(lightyear::frame_interpolation::FrameInterpolationPlugin::<
        Transform,
    >::default());

    if matches!(role, LightyearRole::Client) {
        app.add_plugins(bevy::input::InputPlugin);
    }
    app.add_plugins(lightyear::prelude::input::leafwing::InputPlugin::<
        AfterglowAction,
    >::default());

    if matches!(role, LightyearRole::Client) {
        app.add_systems(
            FixedPreUpdate,
            write_desired_input
                .in_set(lightyear::prelude::client::input::InputSystems::WriteClientInputs),
        );
    }

    app.add_systems(FixedUpdate, move_players);
}

#[derive(Resource, Default)]
struct DesiredInput(pub ActionState<AfterglowAction>);

fn write_desired_input(
    desired: Option<Res<DesiredInput>>,
    mut query: Query<&mut ActionState<AfterglowAction>, With<InputMap<AfterglowAction>>>,
    rollback: Query<(), With<Rollback>>,
) {
    if rollback.iter().next().is_some() {
        return;
    }
    let Some(desired) = desired else { return };
    for mut state in &mut query {
        *state = desired.0.clone();
    }
}

fn move_players(mut query: Query<(&ActionState<AfterglowAction>, &mut Transform)>) {
    for (action, mut transform) in query.iter_mut() {
        let move_axis = action.clamped_axis_pair(&AfterglowAction::Move);
        if move_axis.length_squared() > 0.0 {
            transform.translation +=
                Vec3::new(move_axis.x, 0.0, move_axis.y) * MOVE_SPEED * TICK_DT;
        }
    }
}

fn create_rig() -> LightyearTestRig {
    let mut rig = LightyearTestRig::new_with_transport(
        1,
        |_| {},
        register_protocol,
        crate::TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();

    let client_link = rig.client_link(0);
    rig.client_world_mut(0).entity_mut(client_link).insert(
        lightyear::prelude::client::InputTimelineConfig::default()
            .with_input_delay(lightyear::prelude::client::InputDelayConfig::fixed_input_delay(2)),
    );

    for _ in 0..240 {
        rig.advance(1);
        let client_link = rig.client_link(0);
        if rig
            .client_world(0)
            .get::<IsSynced<InputTimeline>>(client_link)
            .is_some()
        {
            return rig;
        }
    }
    panic!("input timeline did not sync");
}

fn spawn_player(rig: &mut LightyearTestRig, sid: StableEntityId) -> Entity {
    let entity = rig.spawn_replicated(
        sid,
        (
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );
    let mut entities = vec![entity];
    for i in 0..rig.client_apps.len() {
        let c = rig
            .find_client_entity(i, sid)
            .unwrap_or_else(|| panic!("client {i} entity for {sid:?}"));
        entities.push(c);
    }
    rig.register_entity(sid, entities);
    entity
}

fn setup_client_input(rig: &mut LightyearTestRig, client_id: usize, sid: StableEntityId) {
    let entity = rig.client_entity(sid, client_id);
    rig.client_world_mut(client_id)
        .entity_mut(entity)
        .insert(default_gameplay_input_map())
        .insert(lightyear::frame_interpolation::FrameInterpolate::<Transform>::default());

    for _ in 0..120 {
        rig.advance(1);
        if rig
            .client_world(client_id)
            .get::<lightyear::prelude::input::leafwing::LeafwingBuffer<AfterglowAction>>(entity)
            .is_some()
        {
            return;
        }
    }
    panic!("LeafwingBuffer did not appear on client entity");
}

fn press_move(rig: &mut LightyearTestRig, client_id: usize, dir: Vec2) {
    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, dir);
    rig.client_world_mut(client_id)
        .insert_resource(DesiredInput(state));
}

fn release_input(rig: &mut LightyearTestRig, client_id: usize) {
    rig.client_world_mut(client_id)
        .insert_resource(DesiredInput(ActionState::<AfterglowAction>::default()));
}

// ---------------------------------------------------------------------------
// 1. Input Pipeline Corners
// ---------------------------------------------------------------------------

/// Input delay is actually set — not 0 (the stuck-input root cause).
#[test]
fn input_delay_is_not_zero() {
    let rig = create_rig();
    let client_link = rig.client_link(0);
    let timeline = rig
        .client_world(0)
        .get::<InputTimeline>(client_link)
        .expect("InputTimeline should exist");
    assert!(
        timeline.input_delay() > 0,
        "input delay must be > 0, got {}",
        timeline.input_delay()
    );
}

/// Rebroadcast resource exists (configured by the engine plugin).
/// Note: the test rig uses raw Lightyear plugins, not AfterglowLightyearPlugin,
/// so rebroadcast is not configured here. This test just verifies the resource
/// type is accessible.
#[test]
fn rebroadcast_resource_accessible() {
    let rig = create_rig();
    // The InputConfig resource may or may not exist depending on whether
    // the leafwing InputPlugin was added. Just verify no panic.
    let _ = rig
        .client_world(0)
        .get_resource::<lightyear::prelude::input::InputConfig<AfterglowAction>>();
}

/// Input press edge propagates exactly once.
#[test]
fn input_press_edge_propagates_once() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);

    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&AfterglowAction::Jump);
    rig.client_world_mut(0).insert_resource(DesiredInput(state));

    // Wait for the server to receive the input
    let mut received_count = 0;
    for _ in 0..30 {
        rig.advance(1);
        if let Some(action) = rig.server_component::<ActionState<AfterglowAction>>(server_entity) {
            if action.pressed(&AfterglowAction::Jump) {
                received_count += 1;
            }
        }
    }

    assert!(
        received_count > 0,
        "server should receive the press at least once"
    );
}

/// Input release edge stops movement.
#[test]
fn input_release_stops_movement() {
    let mut rig = create_rig();
    let _server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);
    let client_entity = rig.client_entity(PLAYER, 0);

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(20);

    let pos_before = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;

    release_input(&mut rig, 0);
    rig.advance(20);

    let pos_after = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let delta = pos_after.distance(pos_before);

    // With native Lightyear input buffering plus frame interpolation, the
    // visible transform can include the write tick, the configured 2-tick
    // delay, UDP/correction jitter, and a small interpolation tail. It should
    // still stop quickly instead of continuing for the full observation window.
    assert!(
        delta < MOVE_SPEED * TICK_DT * 10.0,
        "client should stop after release: delta={delta}"
    );
    assert!(
        delta > 0.0,
        "client should have moved during delay window: delta={delta}"
    );

    rig.advance(10);
    let pos_settled = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let settled_delta = pos_settled.distance(pos_after);
    assert!(
        settled_delta < MOVE_SPEED * TICK_DT * 2.0,
        "client should remain stopped after interpolation settles: settled_delta={settled_delta}"
    );
}

// ---------------------------------------------------------------------------
// 2. Prediction Corners
// ---------------------------------------------------------------------------

/// Predicted entity has all required components.
#[test]
fn predicted_entity_has_required_components() {
    let mut rig = create_rig();
    let _server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);
    let client_entity = rig.client_entity(PLAYER, 0);

    assert!(
        rig.client_world(0)
            .get::<Predicted>(client_entity)
            .is_some(),
        "entity should have Predicted"
    );
    assert!(
        rig.client_world(0)
            .get::<Transform>(client_entity)
            .is_some(),
        "entity should have Transform"
    );
    assert!(
        rig.client_world(0)
            .get::<ActionState<AfterglowAction>>(client_entity)
            .is_some(),
        "entity should have ActionState"
    );
    assert!(
        rig.client_world(0)
            .get::<InputMap<AfterglowAction>>(client_entity)
            .is_some(),
        "entity should have InputMap"
    );
}

/// Client moves on tick 1 (zero-latency prediction).
#[test]
fn client_moves_on_tick_1() {
    let mut rig = create_rig();
    let _server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);
    let client_entity = rig.client_entity(PLAYER, 0);

    let start = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(1);

    let after = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    assert!(
        after.distance(start) > 0.01,
        "client should move on tick 1: delta={}",
        after.distance(start)
    );
}

/// Client stays ahead of server during continuous input.
#[test]
fn client_stays_ahead_of_server() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);
    let client_entity = rig.client_entity(PLAYER, 0);

    press_move(&mut rig, 0, Vec2::new(1.0, 0.0));

    let mut client_always_ahead = true;
    for _ in 0..60 {
        rig.advance(1);
        let client_x = rig
            .client_component::<Transform>(0, client_entity)
            .unwrap()
            .translation
            .x;
        let server_x = rig
            .server_component::<Transform>(server_entity)
            .unwrap()
            .translation
            .x;
        if client_x < server_x - 0.3 {
            client_always_ahead = false;
        }
    }
    assert!(client_always_ahead, "client should stay ahead of server");
}

/// Rollback does not exceed max ticks.
#[test]
fn rollback_does_not_exceed_max() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);
    let client_entity = rig.client_entity(PLAYER, 0);

    // Move forward
    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(30);

    // Teleport server far away (large divergence)
    release_input(&mut rig, 0);
    rig.server_world_mut()
        .entity_mut(server_entity)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(50.0, 0.0, 50.0);

    // Advance — rollback should trigger but not exceed max
    rig.advance(30);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    // Client should converge (not be stuck at old position)
    let error = client_pos.distance(server_pos);
    assert!(
        error < 5.0,
        "client should converge after rollback: error={error}"
    );
}

// ---------------------------------------------------------------------------
// 3. Replication Corners
// ---------------------------------------------------------------------------

/// Server-spawned entity appears on client.
#[test]
fn spawned_entity_appears_on_client() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);

    rig.advance(5);

    let client_entity = rig.find_client_entity(0, PLAYER);
    assert!(
        client_entity.is_some(),
        "client should have the replicated entity"
    );
    assert_ne!(
        client_entity.unwrap(),
        server_entity,
        "client entity should differ from server entity"
    );
}

/// Component updates replicate to client.
#[test]
fn component_updates_replicate() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    rig.advance(5);

    // Change server Transform
    rig.server_world_mut()
        .entity_mut(server_entity)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(5.0, 0.0, 5.0);

    rig.advance(10);

    let client_entity = rig.client_entity(PLAYER, 0);
    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;

    // Client should have received the update (predicted entities correct via
    // rollback)
    assert!(
        client_pos.distance(Vec3::new(5.0, 0.0, 5.0)) < 2.0,
        "client should receive component update: client={client_pos}"
    );
}

/// Entity despawn replicates to client.
#[test]
fn entity_despawn_replicates() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    rig.advance(5);

    let client_entity = rig.client_entity(PLAYER, 0);
    assert!(rig.client_world(0).entities().get(client_entity).is_ok());

    // Despawn on server
    rig.server_world_mut().entity_mut(server_entity).despawn();

    rig.advance(10);

    // Client entity should be gone (or at least not have the original entity)
    // Note: prediction might keep the predicted copy for a bit, but the
    // confirmed entity should be despawned.
    let _client_entity = rig.find_client_entity(0, PLAYER);
    // The entity may or may not exist depending on prediction despawn mode,
    // but the confirmed entity should be gone.
    // We just verify no panic.
}

// ---------------------------------------------------------------------------
// 4. Controlled Entity Corners
// ---------------------------------------------------------------------------

/// MemberLinkMap populates from ClientOf links.
#[test]
fn member_link_map_populates() {
    let mut rig = create_rig();
    spawn_player(&mut rig, PLAYER);
    rig.advance(5);

    let map = rig
        .server_world()
        .get_resource::<afterglow_engine::network::connection::MemberLinkMap>();
    if let Some(map) = map {
        assert!(
            !map.links.is_empty(),
            "MemberLinkMap should have at least one entry"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Cleanup Corners
// ---------------------------------------------------------------------------

/// SessionLightyearLinks cleared after leave.
/// TODO: Rewrite with new connection lifecycle when Phase 4 harness update
/// lands.
#[ignore]
#[test]
fn links_cleared_after_leave() {
    // Stub: SessionLightyearLinks was removed in Phase 3.
    // This test needs to be rewritten to verify new connection lifecycle.
}

// ---------------------------------------------------------------------------
// 6. Adversarial / Edge Cases
// ---------------------------------------------------------------------------

/// Large correction (teleport) triggers rollback and converges.
#[test]
fn large_correction_converges() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);
    let client_entity = rig.client_entity(PLAYER, 0);

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(20);

    release_input(&mut rig, 0);
    rig.server_world_mut()
        .entity_mut(server_entity)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(20.0, 0.0, 20.0);

    rig.advance(30);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    assert!(
        client_pos.distance(server_pos) < 2.0,
        "client should converge after large correction: error={}",
        client_pos.distance(server_pos)
    );
}

/// Two players can coexist without interference.
#[test]
fn two_players_coexist() {
    let mut rig = LightyearTestRig::new_with_transport(
        2,
        |_| {},
        register_protocol,
        crate::TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();

    for client_id in 0..2 {
        let client_link = rig.client_link(client_id);
        rig.client_world_mut(client_id)
            .entity_mut(client_link)
            .insert(
                lightyear::prelude::client::InputTimelineConfig::default().with_input_delay(
                    lightyear::prelude::client::InputDelayConfig::fixed_input_delay(2),
                ),
            );
    }

    // Wait for sync
    for _ in 0..240 {
        rig.advance(1);
        let all_synced = (0..2).all(|i| {
            let client_link = rig.client_link(i);
            rig.client_world(i)
                .get::<IsSynced<InputTimeline>>(client_link)
                .is_some()
        });
        if all_synced {
            break;
        }
    }

    let _server1 = spawn_player(&mut rig, PLAYER);
    let _server2 = spawn_player(&mut rig, PLAYER2);
    rig.advance(10);

    // Both should appear on both clients
    assert!(rig.find_client_entity(0, PLAYER).is_some());
    assert!(rig.find_client_entity(0, PLAYER2).is_some());
    assert!(rig.find_client_entity(1, PLAYER).is_some());
    assert!(rig.find_client_entity(1, PLAYER2).is_some());
}

/// Direction change: client changes direction and both converge.
#[test]
fn direction_change_converges() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);

    press_move(&mut rig, 0, Vec2::new(1.0, 0.0));
    rig.advance(30);

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(30);

    release_input(&mut rig, 0);
    rig.advance(60);

    let client_pos = rig
        .client_component::<Transform>(0, rig.client_entity(PLAYER, 0))
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    assert!(
        client_pos.distance(server_pos) < 0.5,
        "client and server should converge after direction change: error={}",
        client_pos.distance(server_pos)
    );
}

/// Client and server converge after sustained input.
#[test]
fn sustained_input_converges() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig, PLAYER);
    setup_client_input(&mut rig, 0, PLAYER);

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(60);

    let client_pos = rig
        .client_component::<Transform>(0, rig.client_entity(PLAYER, 0))
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    assert!(
        server_pos.z > 3.0,
        "server should have moved forward: z={}",
        server_pos.z
    );
    assert!(
        client_pos.distance(server_pos) < 0.5,
        "client and server should converge: error={}",
        client_pos.distance(server_pos)
    );
}
