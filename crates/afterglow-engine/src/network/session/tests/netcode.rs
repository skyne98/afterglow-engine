use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::time::Duration;

use bevy::prelude::*;

use super::{test_nonce, PlayerIdentity, SessionBackend, SessionConfig, SessionIdentityNonce, SessionStatus, SessionTransport};
use crate::network::lightyear::{
    AfterglowLightyearPlugin, AfterglowLightyearConfig, AfterglowNetcodeConsumerPlugin,
    AfterglowSessionLightyearBridgePlugin, LightyearRole,
};
use crate::network::session::{AfterglowSessionExt, AfterglowSessionPlugin};

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

fn identity(seed: u8, target: &str) -> PlayerIdentity {
    PlayerIdentity::test_native(&test_nonce(), SessionBackend::NonSteam, target, seed)
}

fn lightyear_test_app(role: LightyearRole) -> App {
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

    app.world_mut()
        .insert_resource(SessionIdentityNonce(test_nonce()));
    app
}

fn drive(apps: &mut [&mut App], frames: usize) {
    for _ in 0..frames {
        for app in apps.iter_mut() {
            app.update();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn host_and_client_establish_real_netcode_links_over_provider() {
    let provider_addr = find_tcp_addr();
    let netcode_addr = find_udp_addr();

    let mut host = lightyear_test_app(LightyearRole::Host);

    host.session()
        .host_with_endpoint(
            SessionConfig {
                backend: SessionBackend::NonSteam,
                transport: SessionTransport::DirectUdp {
                    host: netcode_addr.to_string(),
                },
                name: "netcode-test".into(),
                metadata: [("name".into(), "netcode-test".into())].into(),
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

    let mut client = lightyear_test_app(LightyearRole::Client);
    drive(&mut [&mut client], 5);

    client
        .session()
        .search_non_steam(provider_addr, [("name".into(), "netcode-test".into())].into());

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

    let target = code.as_str().to_string();
    client.session().join_non_steam(code, provider_addr, identity(1, &target));

    let mut client_saw_connecting = false;
    let mut client_link_spawned = false;
    let mut host_started = false;
    for i in 0..600 {
        drive(&mut [&mut host, &mut client], 1);

        let client_in = client.world().resource::<SessionStatus>().is_in_session();
        let (connecting, disconnected) = client_connection_state(&mut client);
        let link = client
            .world()
            .resource::<crate::network::lightyear::SessionLightyearLinks>()
            .client_link;
        let host_start = has_server_started(&mut host);
        if i % 20 == 0 {
            eprintln!(
                "iter {} client_in={} connecting={} disconnected={} host_start={} link={:?}",
                i, client_in, connecting, disconnected, host_start, link
            );
        }
        if client_in {
            client_link_spawned |= link.is_some();
            client_saw_connecting |= connecting > 0;
        }
        if host_start {
            host_started = true;
        }
        if client_link_spawned && client_saw_connecting && host_started {
            break;
        }
    }

    assert!(
        client_link_spawned,
        "client should have spawned a netcode link entity"
    );
    assert!(
        client_saw_connecting,
        "client netcode link should have attempted connection"
    );
    assert!(
        host_started,
        "host netcode server should be listening/started"
    );
}

fn client_connection_state(app: &mut App) -> (usize, usize) {
    use lightyear::prelude::client::{Connecting, Disconnected};
    let world = app.world_mut();
    (
        world.query::<&Connecting>().iter(world).count(),
        world.query::<&Disconnected>().iter(world).count(),
    )
}

fn has_server_started(app: &mut App) -> bool {
    use lightyear::prelude::server::Started;
    let world = app.world_mut();
    world.query::<&Started>().iter(world).next().is_some()
}
