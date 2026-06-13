use super::*;
use crate::network::{lightyear::LightyearRole, session::*};
use std::time::Duration;

fn test_app(role: LightyearRole) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / 60.0);
    app.add_plugins((
        lightyear::prelude::server::ServerPlugins { tick_duration }
            .build()
            .disable::<lightyear::prelude::server::NetcodeServerPlugin>(),
        lightyear::prelude::client::ClientPlugins { tick_duration }
            .build()
            .disable::<lightyear::prelude::client::NetcodeClientPlugin>(),
    ));
    app.init_resource::<PeerMetadata>();

    app.insert_resource(AfterglowLightyearConfig {
        role,
        ..Default::default()
    });
    app.add_plugins(AfterglowSessionPlugin);
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .local_member_id = SessionMemberId::new(1);
    app.add_plugins(AfterglowSessionLightyearBridgePlugin);
    app
}

fn drain_events(app: &mut App) -> Vec<SessionEvent> {
    app.world_mut()
        .resource_mut::<Messages<SessionEvent>>()
        .drain()
        .collect()
}

fn identity_fixture() -> PlayerIdentity {
    PlayerIdentity::Steam {
        steam_id: 1,
        ticket: vec![],
    }
}

fn info_fixture() -> SessionInfo {
    SessionInfo {
        id: SessionId::new(1),
        code: SessionCode::new("ABC-DEF"),
        backend: SessionBackend::NonSteam,
        name: "test".into(),
        owner: SessionMemberId::new(1),
        owner_identity: identity_fixture(),
        member_count: 1,
        max_members: 4,
        visibility: SessionVisibility::Private,
        metadata: Default::default(),
        transport: SessionTransport::Local,
    }
}

mod edge_cases;

#[test]
fn plugin_initializes_resources() {
    let app = test_app(LightyearRole::Client);
    assert!(app.world().contains_resource::<SessionLightyearLinks>());
    assert!(app.world().contains_resource::<PendingNetcodeStartup>());
}

#[test]
fn local_joined_spawns_links_for_host() {
    let mut app = test_app(LightyearRole::Host);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig::default(),
        identity_fixture(),
    ));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_some(), "client link should be spawned");
    assert!(links.server_link.is_some(), "server link should be spawned");
    assert!(
        links.server_entity.is_some(),
        "server entity should be spawned"
    );
    let client_link = links.client_link.unwrap();
    let server_link = links.server_link.unwrap();
    let server_entity = links.server_entity.unwrap();
    assert!(app.world().get_entity(client_link).is_ok());
    assert!(app.world().get_entity(server_link).is_ok());
    assert!(app.world().get_entity(server_entity).is_ok());
    let e = |e: Entity| app.world().entity(e);
    assert!(e(client_link).contains::<Client>() && e(client_link).contains::<Linked>());
    assert!(e(server_link).contains::<LinkOf>() && e(server_link).contains::<ReplicationSender>());
    assert!(e(server_entity).contains::<Server>() && e(server_entity).contains::<Started>());
}

#[test]
fn leave_despawns_links_and_clears_resources() {
    let mut app = test_app(LightyearRole::Host);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig::default(),
        identity_fixture(),
    ));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    let client_link = links.client_link.unwrap();
    let server_link = links.server_link.unwrap();
    let server_entity = links.server_entity.unwrap();

    drain_events(&mut app);
    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_none());
    assert!(links.server_link.is_none());
    assert!(links.server_entity.is_none());

    assert!(app.world().get_entity(client_link).is_err());
    assert!(app.world().get_entity(server_link).is_err());
    assert!(app.world().get_entity(server_entity).is_err());
}

#[test]
fn local_joined_idempotent() {
    let mut app = test_app(LightyearRole::Host);

    let info = SessionInfo {
        transport: SessionTransport::Local,
        ..info_fixture()
    };

    app.world_mut()
        .write_message(SessionEvent::Joined(info.clone()));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    let first_client = links.client_link.unwrap();
    let first_server = links.server_link.unwrap();
    let first_server_entity = links.server_entity.unwrap();

    assert!(app.world().get_entity(first_client).is_ok());

    app.world_mut().write_message(SessionEvent::Joined(info));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    let second_client = links.client_link.unwrap();
    let second_server = links.server_link.unwrap();

    assert_ne!(
        first_client, second_client,
        "new client link should replace old"
    );
    assert_ne!(
        first_server, second_server,
        "new server link should replace old"
    );

    assert!(app.world().get_entity(first_client).is_err());
    assert!(app.world().get_entity(first_server).is_err());
    assert!(app.world().get_entity(first_server_entity).is_err());
}

#[test]
fn direct_udp_joined_writes_pending_params_for_host() {
    let mut app = test_app(LightyearRole::Host);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            transport: SessionTransport::DirectUdp {
                host: "127.0.0.1:8820".into(),
            },
            ..Default::default()
        },
        identity_fixture(),
    ));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_some());
    assert!(pending.server.is_some());

    let client = pending.client.as_ref().unwrap();
    assert_eq!(client.server_addr, SocketAddr::from(([127, 0, 0, 1], 8820)));
    assert_eq!(client.client_id, 1);

    let server = pending.server.as_ref().unwrap();
    assert_eq!(server.bind_addr, SocketAddr::from(([127, 0, 0, 1], 8820)));
}

#[test]
fn direct_udp_joined_writes_pending_params_for_server() {
    let mut app = test_app(LightyearRole::Server);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            transport: SessionTransport::DirectUdp {
                host: "0.0.0.0:8821".into(),
            },
            ..Default::default()
        },
        identity_fixture(),
    ));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_some());
}

#[test]
fn direct_udp_writes_client_only_for_client_role() {
    let mut app = test_app(LightyearRole::Client);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            transport: SessionTransport::DirectUdp {
                host: "127.0.0.1:8820".into(),
            },
            ..Default::default()
        },
        identity_fixture(),
    ));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_some());
    assert!(pending.server.is_none());
}

#[test]
fn invalid_direct_udp_host_does_not_panic_and_clears_pending() {
    let mut app = test_app(LightyearRole::Host);

    {
        let mut pending = app.world_mut().resource_mut::<PendingNetcodeStartup>();
        pending.client = Some(NetcodeClientParams {
            server_addr: SocketAddr::from(([127, 0, 0, 1], 8820)),
            client_id: 1,
            protocol_id: 0,
            private_key: [0u8; 32],
        });
    }

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            transport: SessionTransport::DirectUdp {
                host: "not-a-valid-addr".into(),
            },
            ..Default::default()
        },
        identity_fixture(),
    ));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(
        pending.client.is_none(),
        "pending client should be empty after invalid host"
    );
    assert!(
        pending.server.is_none(),
        "pending server should be empty after invalid host"
    );
}

#[test]
fn leave_clears_pending_netcode_startup() {
    let mut app = test_app(LightyearRole::Host);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            transport: SessionTransport::DirectUdp {
                host: "127.0.0.1:8820".into(),
            },
            ..Default::default()
        },
        identity_fixture(),
    ));
    app.update();

    assert!(
        app.world()
            .resource::<PendingNetcodeStartup>()
            .client
            .is_some()
    );

    drain_events(&mut app);
    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_none());
}

#[test]
fn search_results_does_not_create_links() {
    let mut app = test_app(LightyearRole::Host);

    {
        let mut catalog = app
            .world_mut()
            .resource_mut::<crate::network::session::non_steam::NonSteamSessionCatalog>();
        catalog.seed_session(
            SessionConfig::default(),
            SessionMemberId::new(42),
            identity_fixture(),
        );
    }

    app.world_mut()
        .write_message(SessionRequest::Search(SessionSearch::default()));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_none());
    assert!(links.server_link.is_none());
}

#[test]
fn local_joined_without_lightyear_plugins_does_not_spawn() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(AfterglowSessionPlugin);
    app.add_plugins(AfterglowSessionLightyearBridgePlugin);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig::default(),
        identity_fixture(),
    ));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_none());
    assert!(links.server_link.is_none());
    assert!(links.server_entity.is_none());
}
