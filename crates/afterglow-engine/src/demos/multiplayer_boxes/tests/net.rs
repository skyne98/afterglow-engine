use std::{
    net::{SocketAddr, TcpListener, UdpSocket},
    time::Duration,
};

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use lightyear::prelude::{client::input::InputSystems, *};

use crate::{
    demos::multiplayer_boxes::{
        movement::{
            DemoInput, add_input_map_to_local_predicted_player, apply_movement, collect_input,
            configure_demo_input_rebroadcast, configure_demo_input_timeline,
        },
        network::register_demo_protocol,
        protocol::{PLAYER_SIZE, PlayerBox},
        scene::{
            MemberToPlayer, PlayerName, attach_predicted_kinematic_physics,
            attach_predicted_player_physics, attach_replicated_kinematic_visuals,
            attach_replicated_player_visuals,
        },
    },
    network::{
        lightyear::{
            AfterglowLightyearConfig, AfterglowLightyearPlugin, AfterglowNetcodeConsumerPlugin,
            AfterglowSessionLightyearBridgePlugin, LightyearRole,
        },
        session::{
            AfterglowSessionExt, AfterglowSessionPlugin, SessionBackend, SessionConfig,
            SessionEvent, SessionIdentityNonce, SessionLeaveReason, SessionStatus,
            SessionTransport, identity::PlayerIdentity,
        },
    },
};

fn find_tcp_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn find_udp_addr() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    addr
}

fn test_nonce() -> [u8; 32] {
    [0xAB; 32]
}

fn identity(seed: u8, target: &str) -> PlayerIdentity {
    PlayerIdentity::demo(&test_nonce(), target, seed)
}

fn build_demo_app(role: LightyearRole) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::input::InputPlugin));

    app.insert_resource(AfterglowLightyearConfig {
        role,
        netcode_private_key: [42u8; 32],
        ..Default::default()
    });

    app.add_plugins((
        AfterglowLightyearPlugin,
        AfterglowSessionPlugin,
        AfterglowSessionLightyearBridgePlugin,
        AfterglowNetcodeConsumerPlugin,
    ));

    let is_host = matches!(role, LightyearRole::Host);

    app.init_resource::<DemoInput>()
        .init_resource::<MemberToPlayer>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

    if is_host {
        app.insert_resource(PlayerName("alice".to_string()));
    } else {
        app.insert_resource(PlayerName(Default::default()));
    }

    register_demo_protocol(&mut app);

    app.world_mut()
        .insert_resource(SessionIdentityNonce(test_nonce()));

    if is_host {
        app.add_systems(Startup, spawn_arena_no_graphics);
        app.add_systems(
            Update,
            (
                configure_demo_input_rebroadcast,
                configure_demo_input_timeline,
                spawn_player_on_member_joined_no_physics,
                despawn_player_on_member_left_no_physics,
            ),
        );
        app.add_systems(FixedUpdate, apply_movement);
        app.add_systems(
            FixedPreUpdate,
            collect_input.in_set(InputSystems::WriteClientInputs),
        );
    } else {
        app.add_systems(
            PreUpdate,
            (
                attach_predicted_player_physics,
                attach_predicted_kinematic_physics,
            )
                .after(ReplicationSystems::Receive),
        );
        app.add_systems(
            Update,
            (
                configure_demo_input_rebroadcast,
                configure_demo_input_timeline,
                attach_replicated_player_visuals,
                attach_replicated_kinematic_visuals,
                add_input_map_to_local_predicted_player.after(attach_replicated_player_visuals),
            ),
        );
        app.add_systems(
            FixedPreUpdate,
            collect_input.in_set(InputSystems::WriteClientInputs),
        );
    }

    // Tests drive apps manually with `App::update()` instead of `App::run()`.
    // Bevy does not run plugin `finish()`/`cleanup()` from `update()`, and
    // Lightyear installs its dynamically-built replication buffer system in
    // `ReplicationSendPlugin::finish()`. Emulate Bevy's runner lifecycle so
    // replication systems are actually present in this manual harness.
    for _ in 0..8 {
        if app.plugins_state() == bevy::app::PluginsState::Ready {
            break;
        }
    }
    app.finish();
    app.cleanup();

    app
}

fn spawn_arena_no_graphics(mut commands: Commands, player_name: Res<PlayerName>) {
    // Spawn a PlayerBox for the host using the simplified no-physics variant.
    spawn_player_box_no_physics(&mut commands, &player_name.0, Vec3::new(-5.0, 0.4, 0.0));
}

fn spawn_player_box_no_physics(commands: &mut Commands, owner: &str, pos: Vec3) -> Entity {
    commands
        .spawn((
            PlayerBox {
                owner: owner.to_string(),
            },
            LinearVelocity::ZERO,
            Transform::from_translation(pos),
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
        ))
        .id()
}

fn spawn_player_on_member_joined_no_physics(
    mut commands: Commands,
    mut map: ResMut<MemberToPlayer>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        let member = match event {
            SessionEvent::MemberJoined { member, .. } => *member,
            _ => continue,
        };
        if map.0.contains_key(&member) {
            continue;
        }
        let owner = member.as_raw().to_string();
        let idx = map.0.len() as f32;
        let pos = Vec3::new(5.0 + idx * 2.0, PLAYER_SIZE, 0.0);
        let entity = spawn_player_box_no_physics(&mut commands, &owner, pos);
        map.0.insert(member, entity);
    }
}

fn despawn_player_on_member_left_no_physics(
    mut commands: Commands,
    mut map: ResMut<MemberToPlayer>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        let (member, reason) = match event {
            SessionEvent::MemberLeft { member, reason, .. } => (*member, reason.clone()),
            _ => continue,
        };
        if reason == SessionLeaveReason::Disconnected || reason == SessionLeaveReason::Left {
            if let Some((_, entity)) = map.0.remove_entry(&member) {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn drive(apps: &mut [&mut App], frames: usize) {
    for _ in 0..frames {
        for app in apps.iter_mut() {
            app.update();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn host_and_client_share_player_boxes_over_real_network() {
    let provider_addr = find_tcp_addr();
    let netcode_addr = find_udp_addr();

    let mut host = build_demo_app(LightyearRole::Host);

    host.session()
        .host_with_endpoint(
            SessionConfig {
                backend: SessionBackend::NonSteam,
                transport: SessionTransport::DirectUdp {
                    host: netcode_addr.to_string(),
                },
                name: "multiplayer-boxes-test".into(),
                metadata: [("name".into(), "multiplayer-boxes-test".into())].into(),
                ..Default::default()
            },
            identity(0, "create"),
            provider_addr,
        )
        .unwrap();

    drive(&mut [&mut host], 60);
    assert!(
        host.world().resource::<SessionStatus>().is_in_session(),
        "host should be in session"
    );

    let mut client = build_demo_app(LightyearRole::Client);
    drive(&mut [&mut client], 5);

    client.session().search_non_steam(
        provider_addr,
        [("name".into(), "multiplayer-boxes-test".into())].into(),
    );

    let mut code = None;
    for _ in 0..80 {
        drive(&mut [&mut host, &mut client], 1);
        let results = client
            .world()
            .resource::<SessionStatus>()
            .last_search_results
            .clone();
        if !results.is_empty() {
            code = Some(results[0].code.clone());
            break;
        }
    }
    let code = code.expect("client should find the host session");

    client
        .session()
        .join_non_steam(code.clone(), provider_addr, identity(1, code.as_str()));

    // Wait for session connection + PlayerBox spawn
    let mut found = false;
    for _ in 0..600 {
        drive(&mut [&mut host, &mut client], 1);

        let client_in = client.world().resource::<SessionStatus>().is_in_session();
        let link = client
            .world()
            .resource::<crate::network::lightyear::SessionLightyearLinks>()
            .client_link;
        let members = host.world().resource::<SessionStatus>().members.len();

        if client_in && link.is_some() && members >= 2 {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "client should connect and host should see 2+ members; client_status={:?} host_status={:?} client_state={:?} host_state={:?} client_pending={:?} client_links={:?} host_links={:?}",
        client.world().resource::<SessionStatus>(),
        host.world().resource::<SessionStatus>(),
        client
            .world()
            .resource::<crate::network::session::AfterglowSessionState>(),
        host.world()
            .resource::<crate::network::session::AfterglowSessionState>(),
        client
            .world()
            .resource::<crate::network::lightyear::PendingNetcodeStartup>(),
        client
            .world()
            .resource::<crate::network::lightyear::SessionLightyearLinks>(),
        host.world()
            .resource::<crate::network::lightyear::SessionLightyearLinks>(),
    );

    // Give time for MemberJoined to be processed
    drive(&mut [&mut host, &mut client], 60);

    // Verify host spawned the remote PlayerBox
    let host_has_remote = !host.world().resource::<MemberToPlayer>().0.is_empty();
    assert!(
        host_has_remote,
        "host should have spawned at least one remote PlayerBox"
    );

    let client_player_count = {
        let mut count = 0;
        for _ in 0..600 {
            drive(&mut [&mut host, &mut client], 1);
            let world = client.world_mut();
            count = world
                .query_filtered::<Entity, With<PlayerBox>>()
                .iter(&world)
                .count();
            if count > 0 {
                break;
            }
        }
        count
    };

    assert!(
        client_player_count > 0,
        "client should observe at least one replicated PlayerBox within 600 frames; \
         replication is not flowing"
    );

    let client_visual_player_count = client
        .world_mut()
        .query_filtered::<Entity, (With<PlayerBox>, With<Mesh3d>)>()
        .iter(client.world())
        .count();
    assert!(
        client_visual_player_count > 0,
        "replicated PlayerBox should get client-side Mesh3d presentation"
    );

    let mut native_input_ready = false;
    for _ in 0..240 {
        drive(&mut [&mut host, &mut client], 1);
        let link = client
            .world()
            .resource::<crate::network::lightyear::SessionLightyearLinks>()
            .client_link;
        let synced = link.is_some_and(|link| {
            client
                .world()
                .get::<IsSynced<InputTimeline>>(link)
                .is_some()
        });
        let has_map = client
            .world_mut()
            .query::<&leafwing_input_manager::input_map::InputMap<crate::input::AfterglowAction>>()
            .iter(client.world())
            .next()
            .is_some();
        let has_buffer = client
            .world_mut()
            .query::<&lightyear::prelude::input::leafwing::LeafwingBuffer<crate::input::AfterglowAction>>()
            .iter(client.world())
            .next()
            .is_some();
        if synced && has_map && has_buffer {
            native_input_ready = true;
            break;
        }
    }
    assert!(
        native_input_ready,
        "native Leafwing input path should be ready"
    );

    // Press W on the client so Leafwing/Lightyear native input drives the
    // predicted local player and streams the action state to the server.
    client
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);

    let mut remote_velocity_seen = false;
    for _ in 0..600 {
        drive(&mut [&mut host, &mut client], 1);
        let velocities: Vec<(String, Vec3)> = host
            .world_mut()
            .query::<(&PlayerBox, &LinearVelocity)>()
            .iter(host.world())
            .map(|(player, velocity)| (player.owner.clone(), velocity.0))
            .collect();
        if velocities
            .iter()
            .any(|(owner, velocity)| owner != "alice" && velocity.z > 0.0)
        {
            remote_velocity_seen = true;
            break;
        }
    }
    assert!(
        remote_velocity_seen,
        "server should eventually consume the client's native Leafwing input"
    );

    // Verify both PlayerBoxes exist on the host with the right owners.
    let host_owners: Vec<&str> = host
        .world_mut()
        .query::<&PlayerBox>()
        .iter(host.world())
        .map(|pb| pb.owner.as_str())
        .collect();
    assert!(
        host_owners.contains(&"alice"),
        "host should have a PlayerBox for alice, got {host_owners:?}"
    );
    assert!(
        host_owners.iter().any(|o| *o != "alice"),
        "host should have a PlayerBox for the remote client, got {host_owners:?}"
    );

    let host_velocities: Vec<(String, Vec3)> = host
        .world_mut()
        .query::<(&PlayerBox, &LinearVelocity)>()
        .iter(host.world())
        .map(|(player, velocity)| (player.owner.clone(), velocity.0))
        .collect();
    assert!(
        host_velocities
            .iter()
            .any(|(owner, velocity)| owner == "alice" && *velocity == Vec3::ZERO),
        "host input should not move alice when no host keys are pressed; got {host_velocities:?}"
    );
    assert!(
        host_velocities
            .iter()
            .any(|(owner, velocity)| owner != "alice" && velocity.z > 0.0),
        "client input should move the remote player's box on the host; got {host_velocities:?}"
    );

    // Check native Lightyear input channel plumbing on host links.
    let input_receiver_links = host
        .world_mut()
        .query::<&Transport>()
        .iter(host.world())
        .filter(|transport| transport.has_receiver::<lightyear::input::InputChannel>())
        .count();
    assert!(
        input_receiver_links > 0,
        "host should have at least one transport receiving native input"
    );
}
