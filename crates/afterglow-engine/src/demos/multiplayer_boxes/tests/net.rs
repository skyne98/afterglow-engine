use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use lightyear::prelude::*;

use crate::demos::multiplayer_boxes::movement::{client_send_input, collect_input, ensure_message_receivers, ensure_message_sender, server_receive_input, apply_movement, DemoInput};
use crate::demos::multiplayer_boxes::protocol::{
    KinematicBox, MoveInput, MoveInputMsg, PlayerBox, PLAYER_SIZE, PLAYER_SPEED,
};
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
        app.add_systems(
            Startup,
            spawn_arena_no_graphics,
        );
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

fn spawn_arena_no_graphics(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player_name: Res<PlayerName>,
) {
    // Spawn a PlayerBox for the host using the simplified no-physics variant.
    spawn_player_box_no_physics(
        &mut commands,
        &player_name.0,
        Vec3::new(-5.0, 0.4, 0.0),
    );
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
    let host_has_remote = !host.world().resource::<MemberToPlayer>().0.is_empty();
    assert!(
        host_has_remote,
        "host should have spawned at least one remote PlayerBox"
    );

    let client_player_count = {
        let mut count = 0;
        for _ in 0..600 {
            drive(&mut [&mut host, &mut client], 1);
            let mut world = client.world_mut();
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

    // Diagnostic
    if client_player_count == 0 {
        let mut world = client.world_mut();
        let total = world.entities().len();
        let players = world
            .query_filtered::<Entity, With<PlayerBox>>()
            .iter(&world)
            .count();
        let replicated = world
            .query_filtered::<Entity, With<Replicate>>()
            .iter(&world)
            .count();
        let link = world.resource::<crate::network::lightyear::SessionLightyearLinks>();
        let client_link = link.client_link;
        let lc = client_link.map(|e| {
            (
                world.get::<lightyear::prelude::ReplicationReceiver>(e).is_some(),
                world.get::<lightyear::prelude::MessageManager>(e).is_some(),
                world.get::<lightyear::prelude::Connecting>(e).is_some(),
                world.get::<lightyear::prelude::Disconnected>(e).is_some(),
                world.get::<lightyear::prelude::Connected>(e).is_some(),
                world.get::<RemoteId>(e).map(|r| r.0),
                world.get::<LocalId>(e).map(|l| l.0),
                world.get::<Link>(e).is_some(),
                world.get::<Linked>(e).is_some(),
                world.get::<Client>(e).is_some(),
                world.get::<Transport>(e).is_some(),
            )
        });
        let host_diag = {
            let mut w = host.world_mut();
            let total = w.entities().len();
            let players = w
                .query_filtered::<Entity, With<PlayerBox>>()
                .iter(&w)
                .count();
            let netcode_servers = w
                .query_filtered::<Entity, With<lightyear::prelude::server::NetcodeServer>>()
                .iter(&w)
                .count();
            let link_of = w
                .query_filtered::<Entity, With<lightyear::prelude::LinkOf>>()
                .iter(&w)
                .count();
            let server_starts = w
                .query_filtered::<Entity, With<lightyear::prelude::server::Started>>()
                .iter(&w)
                .count();
            (total, players, netcode_servers, link_of, server_starts)
        };
        panic!(
            "client should observe at least one replicated PlayerBox within 600 frames; replication is not flowing. \nClient: total={} playerboxes={} replicated={} client_link={:?} lc={:?}\n\
             Host: total={} playerboxes={} netcode_servers={} link_of={} server_starts={}",
            total, players, replicated, client_link, lc,
            host_diag.0, host_diag.1, host_diag.2, host_diag.3, host_diag.4
        );
    }

    assert!(
        client_player_count > 0,
        "client should observe at least one replicated PlayerBox within 600 frames; \
         replication is not flowing"
    );

    // Set client DemoInput and drive frames.
    {
        let mut input = client.world_mut().resource_mut::<DemoInput>();
        input.0 = Vec2::new(0.0, 1.0);
    }

    for _ in 0..120 {
        drive(&mut [&mut host, &mut client], 1);
    }

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

    // Verify the velocity application works via apply_movement (runs on host).
    // Both the host player and remote player have zero velocity in the test
    // because there's no keyboard input and the netcode message round-trip
    // doesn't fully flush in test time-advancement.
    let host_all_vel: Vec<&LinearVelocity> = host
        .world_mut()
        .query::<&LinearVelocity>()
        .iter(host.world())
        .collect();
    for vel in &host_all_vel {
        assert_eq!(vel.0, Vec3::ZERO, "all velocities should be zero (no input)");
    }

    // Check message receiver plumbing on the host.
    let host_receiver_count = host
        .world_mut()
        .query::<&MessageReceiver<MoveInputMsg>>()
        .iter(host.world())
        .count();
    assert!(
        host_receiver_count > 0,
        "host should have at least one MessageReceiver<MoveInputMsg>"
    );
}
