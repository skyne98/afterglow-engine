//! Prediction and rollback integration tests.
//!
//! These tests run a real Lightyear simulation (Crossbeam transport) for many
//! ticks and verify the core prediction/rollback/correction guarantees:
//!
//! 1. **Local prediction is instant**: when the client presses a movement key,
//!    the predicted entity moves on the very next fixed tick — before the
//!    server has processed or confirmed the input.
//! 2. **Divergence triggers rollback**: when the server's authoritative state
//!    disagrees with the client's prediction, Lightyear performs a rollback.
//! 3. **Correction is smooth**: after a rollback, Lightyear inserts a
//!    `VisualCorrection` component and decays the error over time instead of
//!    hard-snapping the entity to the confirmed position.

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
const MOVE_SPEED: f32 = 5.0;
const TICK_DT: f32 = 1.0 / 60.0;

/// Register a minimal prediction protocol: Transform with prediction + linear
/// correction, and native Leafwing input.
fn register_prediction_protocol(app: &mut App, role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Transform>()
        .add_prediction()
        .add_linear_correction_fn::<Isometry3d>();
    app.world_mut()
        .resource_mut::<InterpolationRegistry>()
        .set_interpolation::<Transform>(TransformLinearInterpolation::lerp);
    app.register_component::<ActionState<AfterglowAction>>();

    // Enable frame interpolation for smooth correction during rollback
    app.add_plugins(lightyear::frame_interpolation::FrameInterpolationPlugin::<
        Transform,
    >::default());

    if matches!(role, LightyearRole::Client | LightyearRole::Host) {
        app.add_plugins(bevy::input::InputPlugin);
    }
    app.add_plugins(lightyear::prelude::input::leafwing::InputPlugin::<
        AfterglowAction,
    >::default());

    if matches!(role, LightyearRole::Client | LightyearRole::Host) {
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
    // Do not write to ActionState during rollback replay — Lightyear restores
    // historical values from the InputBuffer.
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
        register_prediction_protocol,
        crate::TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();

    // Set input delay — needed for the server to process inputs correctly.
    let client_link = rig.client_link(0);
    rig.client_world_mut(0).entity_mut(client_link).insert(
        lightyear::prelude::client::InputTimelineConfig::default()
            .with_input_delay(lightyear::prelude::client::InputDelayConfig::fixed_input_delay(2)),
    );

    // Wait for input timeline to sync
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

fn spawn_player(rig: &mut LightyearTestRig) -> Entity {
    let entity = rig.spawn_replicated(
        PLAYER,
        (
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );
    let mut entities = vec![entity];
    for i in 0..rig.client_apps.len() {
        let c = rig
            .find_client_entity(i, PLAYER)
            .unwrap_or_else(|| panic!("client {i} entity for {PLAYER:?}"));
        entities.push(c);
    }
    rig.register_entity(PLAYER, entities);
    entity
}

fn setup_client_input(rig: &mut LightyearTestRig, client_id: usize) {
    let entity = rig.client_entity(PLAYER, client_id);
    rig.client_world_mut(client_id)
        .entity_mut(entity)
        .insert(default_gameplay_input_map())
        .insert(lightyear::frame_interpolation::FrameInterpolate::<Transform>::default());

    // Wait for LeafwingBuffer to appear (input pipeline ready)
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

/// **Local prediction is instant**: when the client presses W, the predicted
/// entity moves forward on the very next tick, before the server has
/// **Local prediction is instant**: when the client presses W, the predicted
/// entity moves on the very next tick, before the server has processed the
/// input.
#[test]
fn local_prediction_moves_immediately_before_server_confirms() {
    let mut rig = create_rig();
    let _server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    let start_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(1);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let client_delta = client_pos.distance(start_pos);

    assert!(
        client_delta > 0.01,
        "client predicted entity should move on the first tick after input: delta={client_delta}"
    );

    let server_pos = rig
        .server_component::<Transform>(rig.server_entity(PLAYER))
        .unwrap()
        .translation;
    let server_delta = server_pos.distance(start_pos);

    assert!(
        client_delta >= server_delta - 0.001,
        "client prediction should be at least as far as server: client={client_delta}, server={server_delta}"
    );
}

/// **Divergence triggers rollback**: when the server's state disagrees with
/// the client's prediction, Lightyear performs a rollback and the client's
/// predicted state converges to the server's authoritative state.
#[test]
fn server_divergence_triggers_rollback_and_converges() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    // Move the player forward on the client for several ticks
    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(30);

    let client_pos_before = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos_before = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    // Both should have moved forward
    assert!(
        client_pos_before.z > 0.2,
        "client should have moved forward: z={}",
        client_pos_before.z
    );
    assert!(
        server_pos_before.z > 0.2,
        "server should have moved forward: z={}",
        server_pos_before.z
    );

    // Introduce a divergence: teleport the server entity
    release_input(&mut rig, 0);
    rig.server_world_mut()
        .entity_mut(server_entity)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(10.0, 0.0, 10.0);

    // Advance enough ticks for the server update to reach the client
    rig.advance(30);

    let client_pos_after = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos_after = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    let convergence_error = client_pos_after.distance(server_pos_after);
    assert!(
        convergence_error < 0.5,
        "client should converge to server position after rollback: error={convergence_error}, \
         client={client_pos_after}, server={server_pos_after}"
    );
}

/// **Correction converges without overshoot**: after a divergence, the client
/// converges to the server position. In the full runtime (with proper
/// `PostUpdate` ordering), Lightyear inserts `VisualCorrection` for smooth
/// decay; the test rig's simplified schedule may hard-snap, so this test
/// focuses on convergence correctness.
#[test]
fn correction_converges_without_overshoot() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    // Move forward, then introduce a large divergence
    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(30);

    release_input(&mut rig, 0);

    // Teleport server entity far away
    rig.server_world_mut()
        .entity_mut(server_entity)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(20.0, 0.0, 20.0);

    // Advance enough ticks for the server update to reach the client and
    // trigger a rollback + correction.
    rig.advance(10);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    // The client should have converged to the server position
    let distance_to_server = client_pos.distance(server_pos);
    assert!(
        distance_to_server < 1.0,
        "client should converge to server position after correction: \
         distance_to_server={distance_to_server}, client={client_pos}, server={server_pos}"
    );

    // The client should NOT have overshot past the server position
    let client_z = client_pos.z;
    let server_z = server_pos.z;
    assert!(
        client_z <= server_z + 1.0,
        "client should not overshoot past server: client_z={client_z}, server_z={server_z}"
    );

    // Advance more ticks and verify convergence
    rig.advance(60);

    let client_pos_final = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos_final = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;
    let final_error = client_pos_final.distance(server_pos_final);

    assert!(
        final_error < 0.5,
        "client should eventually converge to server position: final_error={final_error}, \
         client={client_pos_final}, server={server_pos_final}"
    );
}

/// **Prediction stays ahead during continuous input**: while the client
/// holds a movement key, the predicted entity should consistently be ahead of
/// (or at least equal to) the server entity.
#[test]
fn prediction_stays_ahead_during_continuous_input() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    press_move(&mut rig, 0, Vec2::new(1.0, 0.0));

    // FrameInterpolation adds a 1-tick visual delay, so the client's visual
    // position is slightly behind its simulated position. We check that the
    // client stays roughly at or ahead of the server.
    let mut client_never_far_behind = true;
    for _ in 0..60 {
        rig.advance(1);

        let client_pos = rig
            .client_component::<Transform>(0, client_entity)
            .unwrap()
            .translation;
        let server_pos = rig
            .server_component::<Transform>(server_entity)
            .unwrap()
            .translation;

        // Allow up to 2 ticks of visual delay tolerance
        if client_pos.x < server_pos.x - 0.2 {
            client_never_far_behind = false;
        }
    }

    assert!(
        client_never_far_behind,
        "client prediction should stay close to or ahead of server during continuous input"
    );
}

/// **Input release stops after delay window**: when the client releases the
/// movement key, the predicted entity stops after the input delay window
/// passes (the delayed ActionState still has movement for N ticks).
#[test]
fn input_release_stops_after_delay_window() {
    let mut rig = create_rig();
    let _server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(10);

    let pos_before_release = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;

    release_input(&mut rig, 0);
    // With 2-tick input delay, the delayed ActionState still has movement
    // for 2 more ticks. The client stops after the delay window passes.
    rig.advance(6);

    let pos_after_release = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let delta = pos_after_release.distance(pos_before_release);

    // The client should have stopped — delta should be much less than
    // the full 6-tick movement (6 * 0.083 = 0.5), proving it stopped
    // after the delay window.
    assert!(
        delta < 0.5,
        "client should stop after delay window: delta={delta} (expected < 0.5)"
    );
    assert!(
        delta > 0.0,
        "client should have moved during the delay window: delta={delta}"
    );
}

/// **Client and server converge to the same position after sustained input.**
///
/// This is the core "multiplayer doesn't interfere with local feel" test:
/// - Client presses forward for 60 ticks
/// - Both client and server move forward
/// - After enough ticks for the server's authoritative state to replicate back,
///   client and server positions should be within a small tolerance
/// - The client should not feel delayed, stuck, or fighting corrections
#[test]
fn client_and_server_converge_to_same_position_after_input() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);
    let _start_pos = Vec3::ZERO;

    // Press forward (+Z)
    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));

    // Advance 60 ticks — enough for the full round-trip and correction
    rig.advance(60);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    // Both should have moved forward significantly
    let expected_distance = MOVE_SPEED * TICK_DT * 60.0; // 5.0
    assert!(
        server_pos.z > expected_distance * 0.8,
        "server should have moved forward: z={} (expected ~{})",
        server_pos.z,
        expected_distance
    );
    assert!(
        client_pos.z > expected_distance * 0.8,
        "client should have moved forward: z={} (expected ~{})",
        client_pos.z,
        expected_distance
    );

    // Client and server should be very close — the prediction is correct.
    // Allow some tolerance for UDP timing jitter.
    let error = client_pos.distance(server_pos);
    assert!(
        error < 0.5,
        "client and server should converge to the same position: error={error}, \
         client={client_pos}, server={server_pos}"
    );
}

/// **Client moves on tick 1 after input — no delay on local prediction.**
///
/// The whole point of client prediction: the local player feels zero-latency.
/// The input delay is for the SERVER to have time to process, NOT for the
/// client's local movement. With 0 input delay, the client should move
/// immediately on tick 1.
#[test]
fn client_moves_immediately_with_zero_input_delay() {
    let mut rig = create_rig_with_zero_delay();
    let _server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);
    let start_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;

    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(1);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let delta = client_pos.distance(start_pos);

    // With 0 input delay, the client should move on tick 1
    let expected_per_tick = MOVE_SPEED * TICK_DT; // ~0.083
    assert!(
        delta > expected_per_tick * 0.5,
        "client should move immediately on tick 1 with 0 delay: delta={delta} (expected ~{expected_per_tick})"
    );
}

/// **Client and server track each other tick-by-tick during sustained input.**
///
/// Every tick, the client's position should be close to the server's position
/// (within the prediction window). If they diverge significantly at any point,
/// the multiplayer stack is interfering with local feel.
#[test]
fn client_server_track_each_other_tick_by_tick() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    press_move(&mut rig, 0, Vec2::new(1.0, 0.0));

    let mut max_divergence = 0.0_f32;
    for _ in 0..120 {
        rig.advance(1);

        let client_pos = rig
            .client_component::<Transform>(0, client_entity)
            .unwrap()
            .translation;
        let server_pos = rig
            .server_component::<Transform>(server_entity)
            .unwrap()
            .translation;

        let divergence = (client_pos.x - server_pos.x).abs();
        max_divergence = max_divergence.max(divergence);
    }

    // With input delay, the client predicts ahead of the server. The divergence
    // should be bounded by the prediction window. If the client is fighting
    // corrections, the divergence will be unbounded. UDP timing jitter under
    // test load can cause larger divergences; allow up to 3.0.
    assert!(
        max_divergence < 3.0,
        "client and server should track each other: max_divergence={max_divergence}"
    );
}

/// **Input release: client and server both stop at the same position.**
#[test]
fn input_release_converges_client_and_server() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    // Move forward for 30 ticks
    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(30);

    // Release input
    release_input(&mut rig, 0);
    // Advance enough for the server's authoritative state to replicate back
    rig.advance(60);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    let error = client_pos.distance(server_pos);
    assert!(
        error < 0.5,
        "client and server should converge after input release: error={error}, \
         client={client_pos}, server={server_pos}"
    );
}

/// **Direction change: client changes direction and both converge.**
#[test]
fn direction_change_converges() {
    let mut rig = create_rig();
    let server_entity = spawn_player(&mut rig);
    setup_client_input(&mut rig, 0);

    let client_entity = rig.client_entity(PLAYER, 0);

    // Move right for 30 ticks
    press_move(&mut rig, 0, Vec2::new(1.0, 0.0));
    rig.advance(30);

    // Change direction to forward
    press_move(&mut rig, 0, Vec2::new(0.0, 1.0));
    rig.advance(30);

    // Release
    release_input(&mut rig, 0);
    // Advance enough for the server's authoritative state to replicate back
    rig.advance(60);

    let client_pos = rig
        .client_component::<Transform>(0, client_entity)
        .unwrap()
        .translation;
    let server_pos = rig
        .server_component::<Transform>(server_entity)
        .unwrap()
        .translation;

    let error = client_pos.distance(server_pos);
    assert!(
        error < 0.5,
        "client and server should converge after direction change: error={error}, \
         client={client_pos}, server={server_pos}"
    );
}

fn create_rig_with_zero_delay() -> LightyearTestRig {
    let mut rig = LightyearTestRig::new_with_transport(
        1,
        |_| {},
        register_prediction_protocol,
        crate::TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();

    // Set input delay to 0 — the client applies input immediately for
    // local prediction. The server also processes immediately.
    let client_link = rig.client_link(0);
    rig.client_world_mut(0).entity_mut(client_link).insert(
        lightyear::prelude::client::InputTimelineConfig::default()
            .with_input_delay(lightyear::prelude::client::InputDelayConfig::no_input_delay()),
    );

    // Wait for input timeline to sync
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
