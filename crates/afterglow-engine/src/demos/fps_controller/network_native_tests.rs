use super::{network_input, *};
use lightyear::prelude::{
    LocalAddr, MessageManager, MessageSender, PeerAddr, UdpIo,
    client::NetcodeClient,
    server::{NetcodeServer, ServerUdpIo},
};

#[test]
fn remote_launch_spawns_native_lightyear_client_link() {
    let mut app = App::new();
    app.insert_resource(FpsDemoNetworkConfig::remote("127.0.0.1:8820"));
    app.add_plugins((
        MinimalPlugins,
        crate::network::AfterglowLightyearPlugin,
        FpsDemoNetworkPlugin,
    ));
    app.finish();
    app.cleanup();

    app.update();

    let mut clients = app.world_mut().query::<(
        &NetcodeClient,
        &UdpIo,
        &LocalAddr,
        &PeerAddr,
        &MessageSender<FpsDemoInputCommand>,
    )>();
    assert_eq!(clients.iter(app.world()).count(), 1);
    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(
        status.connection,
        FpsDemoConnectionState::Remote("127.0.0.1:8820".into())
    );
    assert!(!status.local_server_running);
}

#[test]
fn server_launch_spawns_native_lightyear_udp_server() {
    let mut app = App::new();
    app.insert_resource(AfterglowLightyearConfig {
        role: LightyearRole::Server,
        server_addr: "127.0.0.1:0".into(),
        ..default()
    });
    app.insert_resource(FpsDemoNetworkConfig::server("127.0.0.1:0"));
    app.add_plugins((
        MinimalPlugins,
        crate::network::AfterglowLightyearPlugin,
        FpsDemoNetworkPlugin,
    ));
    app.finish();
    app.cleanup();

    app.update();

    let mut servers = app
        .world_mut()
        .query::<(&NetcodeServer, &ServerUdpIo, &LocalAddr, &MessageManager)>();
    assert_eq!(servers.iter(app.world()).count(), 1);
    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(
        status.connection,
        FpsDemoConnectionState::Server("127.0.0.1:0".into())
    );
    assert!(status.local_server_running);
}

#[test]
fn native_player_ids_do_not_collide_with_host_player_id() {
    assert_ne!(
        network_native::native_player_id(10_000_001),
        FPS_DEMO_PLAYER_ID
    );
    assert_ne!(
        network_native::native_player_id(u64::MAX),
        FPS_DEMO_PLAYER_ID
    );
    assert_ne!(
        network_native::native_player_id(u64::MAX),
        network_native::native_host_player_id()
    );
    assert_eq!(
        network_native::native_player_client_id(network_native::native_player_id(42)),
        Some(42)
    );
}

#[test]
fn native_remote_avatar_sync_skips_controlled_player_stable_ids() {
    let mut app = App::new();
    app.insert_resource(FpsDemoNetworkStatus {
        connection: FpsDemoConnectionState::Remote("127.0.0.1:50124".into()),
        ..default()
    });
    app.init_non_send_resource::<FpsDemoNetworkRuntime>()
        .add_systems(Update, network_visuals::sync_visible_network_avatars);
    app.world_mut().spawn((
        FpsDemoPlayer,
        FPS_DEMO_PLAYER_ID,
        FpsDemoPlayerState::from_translation(Vec3::new(4.0, 0.95, 4.0)),
        Transform::from_xyz(4.0, 0.95, 4.0),
    ));
    let replicated_state = app
        .world_mut()
        .spawn((
            FPS_DEMO_PLAYER_ID,
            FpsDemoPlayerState::from_translation(Vec3::new(-2.0, 0.95, -2.0)),
        ))
        .id();
    app.finish();
    app.cleanup();

    app.update();

    let status = app.world().resource::<FpsDemoNetworkStatus>();
    assert_eq!(status.visible_remote_avatar_count, 0);
    let remote_avatars = app
        .world_mut()
        .query::<&FpsDemoRemoteAvatar>()
        .iter(app.world())
        .count();
    assert_eq!(remote_avatars, 0);
    assert!(
        app.world()
            .get::<FpsDemoPlayerState>(replicated_state)
            .is_some()
    );
}

#[test]
fn native_avatar_state_integrates_raw_input_from_server_state() {
    let initial_state = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
    let command = FpsDemoInputCommand {
        player: FPS_DEMO_PLAYER_ID,
        tick: 42,
        move_axis: Vec2::new(1.0, 0.0),
        look_axis: Vec2::ZERO,
        jump_held: false,
        crouch_held: false,
        sprint_held: false,
    };

    let (state, motor) = network_native::native_authoritative_avatar_state(
        (
            initial_state.clone(),
            network_input::motor_from_player_state(&initial_state),
        ),
        &command,
    );

    assert!(state.position_mm[0] > initial_state.position_mm[0]);
    assert_eq!(state.position_mm[2], initial_state.position_mm[2]);
    assert!(motor.side_speed > 0.0);
}
