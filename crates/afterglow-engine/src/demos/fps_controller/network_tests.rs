use super::*;
use crate::console::{DevConsolePlugin, run_console_command};
#[cfg(feature = "lightyear")]
use crate::controller::{FirstPersonController, FirstPersonMotorState, PredictionErrorSmoothing};
#[cfg(feature = "lightyear")]
use crate::input::AfterglowAction;
#[cfg(feature = "lightyear")]
use crate::network::NetworkTransformInterpolationBuffer;
#[cfg(feature = "lightyear")]
use leafwing_input_manager::action_state::ActionState;
#[cfg(feature = "lightyear")]
use network_input::{FpsDemoPredictionBuffer, integrate_authoritative_state};

#[test]
fn remote_launch_records_external_server_without_local_runner() {
    let mut app = App::new();
    app.insert_resource(FpsDemoNetworkConfig::remote("192.0.2.10:8820"));
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    app.finish();
    app.cleanup();

    app.update();

    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(
        status.connection,
        FpsDemoConnectionState::Remote("192.0.2.10:8820".into())
    );
    assert!(!status.local_server_running);
    assert!(!status.lightyear_links);
    assert_eq!(
        app.world()
            .resource::<AfterglowLightyearConfig>()
            .remote_addr,
        "192.0.2.10:8820"
    );
}

#[test]
fn console_remote_connect_replaces_default_local_connection() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, DevConsolePlugin, FpsDemoNetworkPlugin));
    app.finish();
    app.cleanup();
    app.update();

    assert!(run_console_command(app.world_mut(), "connect 203.0.113.7:8820").success);
    app.update();

    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(
        status.connection,
        FpsDemoConnectionState::Remote("203.0.113.7:8820".into())
    );
    assert!(!status.local_server_running);
}

#[test]
fn console_disconnect_tears_down_fps_demo_connection() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, DevConsolePlugin, FpsDemoNetworkPlugin));
    app.finish();
    app.cleanup();
    app.update();

    assert!(run_console_command(app.world_mut(), "disconnect").success);
    app.update();

    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(status.connection, FpsDemoConnectionState::Disconnected);
    assert!(!status.local_server_running);
    assert!(!status.lightyear_links);
}

#[cfg(feature = "lightyear")]
#[test]
fn default_launch_starts_real_lightyear_local_client_server() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    app.finish();
    app.cleanup();

    app.update();

    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(status.connection, FpsDemoConnectionState::Local);
    assert!(status.local_server_running);
    assert!(status.lightyear_links);
    assert!(status.replicated_avatar);
}

#[test]
fn player_state_tracks_scene_player_translation() {
    let mut app = App::new();
    app.insert_resource(FpsDemoNetworkConfig::remote("127.0.0.1:8820"));
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    app.world_mut().spawn((
        FpsDemoPlayer,
        FpsDemoPlayerState::default(),
        Transform::from_xyz(1.25, 2.0, -3.5),
    ));
    app.finish();
    app.cleanup();

    app.update();

    let state = app
        .world_mut()
        .query_filtered::<&FpsDemoPlayerState, With<FpsDemoPlayer>>()
        .single(app.world())
        .unwrap();
    assert_eq!(state.position_mm, [1250, 2000, -3500]);
}

#[test]
fn remote_avatar_visuals_do_not_allocate_assets_without_new_avatars() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_systems(Update, network_visuals::ensure_remote_avatar_visuals);
    app.finish();
    app.cleanup();

    app.update();
    app.update();

    assert_eq!(app.world().resource::<Assets<Mesh>>().iter().count(), 0);
    assert_eq!(
        app.world()
            .resource::<Assets<StandardMaterial>>()
            .iter()
            .count(),
        0
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn native_replicated_states_become_visible_remote_avatars() {
    let mut app = App::new();
    let stable_id = StableEntityId::from_raw(20_000_001);
    app.insert_resource(FpsDemoNetworkStatus {
        connection: FpsDemoConnectionState::Remote("127.0.0.1:50124".into()),
        ..default()
    });
    app.init_non_send_resource::<FpsDemoNetworkRuntime>()
        .add_systems(Update, network_visuals::sync_visible_network_avatars);
    app.world_mut().spawn((
        stable_id,
        FpsDemoPlayerState::from_translation(Vec3::new(2.0, 0.95, -1.0)),
    ));
    app.finish();
    app.cleanup();

    app.update();

    let world = app.world_mut();
    let mut avatars = world.query::<(
        &FpsDemoRemoteAvatar,
        &Transform,
        &NetworkTransformInterpolationBuffer,
    )>();
    let visible = avatars.iter(world).collect::<Vec<_>>();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].0.stable_id, stable_id);
    assert_eq!(visible[0].1.translation, Vec3::new(2.0, 0.95, -1.0));
}

#[cfg(feature = "lightyear")]
#[test]
fn visible_player_state_round_trips_through_lightyear_server() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    app.world_mut().spawn((
        FpsDemoPlayer,
        FpsDemoPlayerState::default(),
        Transform::from_xyz(4.0, 1.0, -2.0),
    ));
    app.finish();
    app.cleanup();

    for _ in 0..4 {
        app.update();
    }

    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert!(status.local_player_round_trip);
    assert!(status.replicated_avatar_count >= 2);
    assert!(status.visible_remote_avatar_count >= 1);

    let remote_count = app
        .world_mut()
        .query::<&FpsDemoRemoteAvatar>()
        .iter(app.world())
        .count();
    assert!(remote_count >= 1);
}

#[cfg(feature = "lightyear")]
#[test]
fn fps_input_command_carries_raw_inputs_without_client_state() {
    let command = FpsDemoInputCommand {
        player: FPS_DEMO_PLAYER_ID,
        tick: 7,
        move_axis: Vec2::new(0.5, 1.0),
        look_axis: Vec2::new(2.0, -1.0),
        jump_held: true,
        crouch_held: false,
        sprint_held: true,
    };

    assert_eq!(command.player, FPS_DEMO_PLAYER_ID);
    assert_eq!(command.tick, 7);
}

#[cfg(feature = "lightyear")]
#[test]
fn authoritative_input_maps_positive_x_to_world_right_at_zero_yaw() {
    let state = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
    let command = FpsDemoInputCommand {
        player: FPS_DEMO_PLAYER_ID,
        tick: 0,
        move_axis: Vec2::new(1.0, 0.0),
        look_axis: Vec2::ZERO,
        jump_held: false,
        crouch_held: false,
        sprint_held: false,
    };

    let (integrated, motor) = integrate_authoritative_state(
        state.clone(),
        network_input::motor_from_player_state(&state),
        &command,
    );

    assert!(
        integrated.position_mm[0] > 0,
        "positive Move.x should strafe right (+X), got {:?}",
        integrated.position_mm
    );
    assert_eq!(integrated.position_mm[2], 4000);
    assert!(motor.side_speed > 0.0);
}

#[cfg(feature = "lightyear")]
#[test]
fn visible_player_input_is_integrated_by_authoritative_server() {
    let mut app = App::new();
    let mut input = ActionState::<AfterglowAction>::default();
    input.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    app.world_mut().spawn((
        FpsDemoPlayer,
        FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0)),
        Transform::from_xyz(0.0, 0.95, 4.0),
        input,
    ));
    app.finish();
    app.cleanup();

    for _ in 0..8 {
        app.update();
    }

    let status = app.world().resource::<FpsDemoNetworkStatus>().clone();
    let server_state = app
        .world()
        .non_send_resource::<FpsDemoNetworkRuntime>()
        .local_player_server_state();
    let client_states = app
        .world_mut()
        .non_send_resource_mut::<FpsDemoNetworkRuntime>()
        .local
        .as_mut()
        .map(|local| local.replicated_avatar_states())
        .unwrap_or_default();
    let server_translation = server_state
        .as_ref()
        .map(FpsDemoPlayerState::to_translation);
    assert!(
        server_translation.is_some_and(|translation| translation.z < 4.0),
        "expected authoritative input to move server player forward from z=4.0; status={:?} server_state={:?} client_states={:?}",
        status,
        server_state,
        client_states
    );
    assert!(status.local_player_round_trip);
}

#[cfg(feature = "lightyear")]
#[test]
fn controlled_player_error_over_2m_triggers_teleport() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    let player = app
        .world_mut()
        .spawn((
            FpsDemoPlayer,
            FirstPersonController::new(),
            FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0)),
            FirstPersonMotorState::default(),
            Transform::from_xyz(0.0, 0.95, 4.0),
        ))
        .id();
    app.finish();
    app.cleanup();
    app.update();

    // Teleport far away — correction should snap back (>2m threshold).
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(5.0, 0.95, 4.0);
    app.update();

    let translation = app.world().get::<Transform>(player).unwrap().translation;
    assert!(
        translation.distance(Vec3::new(0.0, 0.95, 4.0)) < 0.1,
        "expected teleport correction for >2m error, got {translation:?}"
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn owned_player_prediction_is_corrected_when_server_state_diverges() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    let player = app
        .world_mut()
        .spawn((
            FpsDemoPlayer,
            FPS_DEMO_PLAYER_ID,
            FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0)),
            FirstPersonMotorState::default(),
            Transform::from_xyz(0.0, 0.95, 4.0),
        ))
        .id();
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }

    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(10.0, 0.95, 4.0);
    app.update();

    let translation = app.world().get::<Transform>(player).unwrap().translation;
    assert!(
        translation.distance(Vec3::new(0.0, 0.95, 4.0)) < 0.06,
        "expected authoritative correction near server state, got {translation:?}"
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn controlled_player_error_under_2m_snaps_body_and_stores_smoothing() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, FpsDemoNetworkPlugin));
    let mut motor = FirstPersonMotorState::default();
    motor.grounded = true;
    let player = app
        .world_mut()
        .spawn((
            FpsDemoPlayer,
            FPS_DEMO_PLAYER_ID,
            FirstPersonController::new(),
            FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0)),
            motor,
            Transform::from_xyz(0.0, 0.95, 4.0),
        ))
        .id();
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }

    // Move within 2m threshold — body snaps back, camera smoothing set.
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(0.5, 0.95, 4.0);
    app.update();

    assert!(
        app.world()
            .entity(player)
            .contains::<PredictionErrorSmoothing>(),
        "expected PredictionErrorSmoothing component for moderate error"
    );
    let translation = app.world().get::<Transform>(player).unwrap().translation;
    assert!(
        translation.distance(Vec3::new(0.0, 0.95, 4.0)) < 0.1,
        "expected body snapped to server position, got {translation:?}"
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn prediction_buffer_replays_only_unacknowledged_commands() {
    let mut buffer = FpsDemoPredictionBuffer::default();
    buffer.push(FpsDemoInputCommand {
        player: FPS_DEMO_PLAYER_ID,
        tick: 10,
        move_axis: Vec2::new(1.0, 0.0),
        look_axis: Vec2::ZERO,
        jump_held: false,
        crouch_held: false,
        sprint_held: false,
    });
    buffer.push(FpsDemoInputCommand {
        player: FPS_DEMO_PLAYER_ID,
        tick: 11,
        move_axis: Vec2::new(0.0, 1.0),
        look_axis: Vec2::ZERO,
        jump_held: false,
        crouch_held: false,
        sprint_held: false,
    });
    let mut authoritative = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
    authoritative.authoritative_tick = 10;

    let (predicted, motor) = buffer.replay_from_authoritative(authoritative);

    assert_eq!(predicted.authoritative_tick, 11);
    assert_eq!(predicted.position_mm[0], 0);
    assert!(
        predicted.position_mm[2] < 4000,
        "unacknowledged forward input should replay from the server snapshot, got {:?}",
        predicted.position_mm
    );
    assert!(motor.forward_speed > 0.0);
}
