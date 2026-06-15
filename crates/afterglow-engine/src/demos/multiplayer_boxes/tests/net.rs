use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use lightyear::prelude::*;

use crate::demos::multiplayer_boxes::movement::{client_send_input, collect_input, ensure_message_receivers, ensure_message_sender, server_receive_input, apply_movement, DemoInput};
use crate::demos::multiplayer_boxes::protocol::{MoveInput, MoveInputMsg, PlayerBox, PLAYER_SIZE, PLAYER_SPEED};
use crate::demos::multiplayer_boxes::scene::{MemberToPlayer, PlayerName};
use crate::demos::multiplayer_boxes::network::register_demo_protocol;
use crate::network::lightyear::{
    AfterglowLightyearConfig, AfterglowLightyearPlugin, AfterglowNetcodeConsumerPlugin,
    AfterglowSessionLightyearBridgePlugin, LightyearRole,
};
use crate::network::session::{
    AfterglowSessionExt, AfterglowSessionPlugin, SessionBackend, SessionConfig,
    SessionEvent, SessionIdentityNonce, SessionLeaveReason, SessionStatus, SessionTransport,
};
use crate::network::session::identity::PlayerIdentity;

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
    app.add_plugins(MinimalPlugins);

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

    app.init_resource::<PlayerName>()
        .init_resource::<DemoInput>()
        .init_resource::<MemberToPlayer>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

    register_demo_protocol(&mut app);

    app.world_mut()
        .insert_resource(SessionIdentityNonce(test_nonce()));

    let is_host = matches!(role, LightyearRole::Host);

    if is_host {
        app.add_systems(
            Update,
            (
                collect_input,
                spawn_player_on_member_joined_no_physics,
                despawn_player_on_member_left_no_physics,
            ),
        );
        app.add_systems(
            FixedUpdate,
            (apply_movement, server_receive_input, ensure_message_receivers),
        );
    } else {
        app.add_systems(
            Update,
            (collect_input,),
        );
        app.add_systems(
            FixedUpdate,
            (ensure_message_sender, client_send_input),
        );
    }

    app
}

fn spawn_player_box_no_physics(
    commands: &mut Commands,
    owner: &str,
    pos: Vec3,
) -> Entity {
    commands
        .spawn((
            PlayerBox {
                owner: owner.to_string(),
            },
            MoveInput {
                direction: Vec2::ZERO,
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

    client
        .session()
        .search_non_steam(provider_addr, [("name".into(), "multiplayer-boxes-test".into())].into());

    let mut code = None;
    for _ in 0..80 {
        drive(&mut [&mut host, &mut client], 1);
        let results = client.world().resource::<SessionStatus>().last_search_results.clone();
        if !results.is_empty() {
            code = Some(results[0].code.clone());
            break;
        }
    }
    let code = code.expect("client should find the host session");

    client.session().join_non_steam(code.clone(), provider_addr, identity(1, code.as_str()));

    // Wait for session connection + PlayerBox spawn
    let mut found = false;
    for _ in 0..600 {
        drive(&mut [&mut host, &mut client], 1);

        let client_in = client.world().resource::<SessionStatus>().is_in_session();
        let link = client.world().resource::<crate::network::lightyear::SessionLightyearLinks>().client_link;
        let members = host.world().resource::<SessionStatus>().members.len();

        if client_in && link.is_some() && members >= 2 {
            found = true;
            break;
        }
    }
    assert!(found, "client should connect and host should see 2+ members");

    // Give time for MemberJoined to be processed
    drive(&mut [&mut host, &mut client], 60);

    // Verify host spawned the remote PlayerBox
    let host_map = host.world().resource::<MemberToPlayer>();
    assert!(
        !host_map.0.is_empty(),
        "host should have spawned at least one remote PlayerBox"
    );

    // Verify at least one non-host PlayerBox exists on the host
    let host_non_host_players: Vec<&PlayerBox> = host
        .world_mut()
        .query::<&PlayerBox>()
        .iter(host.world())
        .filter(|pb| pb.owner != "alice")
        .collect();
    assert!(
        !host_non_host_players.is_empty(),
        "host should have a non-alice PlayerBox for the remote client"
    );

    // Now verify the message flow works by checking that the host's client link
    // entity has a MessageReceiver (added by ensure_message_receivers).
    let host_receivers = host
        .world_mut()
        .query::<&MessageReceiver<MoveInputMsg>>()
        .iter(host.world())
        .count();
    assert!(
        host_receivers > 0,
        "host should have at least one MessageReceiver<MoveInputMsg>"
    );

    // Set client DemoInput and drive frames
    client
        .world_mut()
        .resource_mut::<DemoInput>()
        .0 = Vec2::new(0.0, 1.0);

    for _ in 0..120 {
        drive(&mut [&mut host, &mut client], 1);
    }

    // Check that the remote client's PlayerBox on the host has the expected velocity.
    let host_players: Vec<(&PlayerBox, &LinearVelocity)> = host
        .world_mut()
        .query::<(&PlayerBox, &LinearVelocity)>()
        .iter(host.world())
        .collect();

    let expected_z = PLAYER_SPEED;
    let has_moved = host_players
        .iter()
        .any(|(_, vel)| (vel.z - expected_z).abs() < 0.001);
    assert!(
        has_moved,
        "at least one PlayerBox on host should have LinearVelocity.z = {expected_z}, \
         got: {:?}",
        host_players.iter().map(|(_, vel)| vel.0).collect::<Vec<_>>()
    );
}
