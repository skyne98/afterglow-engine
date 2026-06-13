use super::*;

fn udp_fixture() -> SessionInfo {
    SessionInfo {
        transport: SessionTransport::DirectUdp {
            host: "127.0.0.1:8820".into(),
        },
        ..info_fixture()
    }
}

#[test]
fn direct_udp_client_id_matches_local_member() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .local_member_id = SessionMemberId::new(42);
    let info = SessionInfo {
        owner: SessionMemberId::new(7),
        ..udp_fixture()
    };
    app.world_mut().write_message(SessionEvent::Joined(info));
    app.update();
    let client = app
        .world()
        .resource::<PendingNetcodeStartup>()
        .client
        .as_ref()
        .unwrap();
    assert_eq!(client.client_id, 42);
}

#[test]
fn zero_local_member_suppresses_client_params_but_allows_server() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .local_member_id = SessionMemberId::INVALID;
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_some());
}

#[test]
fn out_of_range_local_member_suppresses_client_params() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .local_member_id = SessionMemberId::new(u128::from(u64::MAX) + 1);
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_some());
}

#[test]
fn transport_switch_direct_udp_to_local_clears_pending() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    assert!(
        app.world()
            .resource::<PendingNetcodeStartup>()
            .client
            .is_some()
    );
    drain_events(&mut app);
    let info = SessionInfo {
        transport: SessionTransport::Local,
        ..info_fixture()
    };
    app.world_mut().write_message(SessionEvent::Joined(info));
    app.update();
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_none());
    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_some());
    assert!(links.server_link.is_some());
    assert!(links.server_entity.is_some());
}

#[test]
fn transport_switch_local_to_direct_udp_despawns_entities() {
    let mut app = test_app(LightyearRole::Host);
    let info = SessionInfo {
        transport: SessionTransport::Local,
        ..info_fixture()
    };
    app.world_mut().write_message(SessionEvent::Joined(info));
    app.update();
    let links = app.world().resource::<SessionLightyearLinks>();
    let old_client = links.client_link.unwrap();
    let old_server = links.server_link.unwrap();
    drain_events(&mut app);
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    assert!(app.world().get_entity(old_client).is_err());
    assert!(app.world().get_entity(old_server).is_err());
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_some());
    assert!(pending.server.is_some());
    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_none());
    assert!(links.server_link.is_none());
    assert!(links.server_entity.is_none());
}

#[test]
fn session_ended_clears_pending_params() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    assert!(
        app.world()
            .resource::<PendingNetcodeStartup>()
            .client
            .is_some()
    );
    drain_events(&mut app);
    app.world_mut()
        .write_message(SessionEvent::SessionEnded(SessionId::new(1)));
    app.update();
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_none());
}

#[test]
fn config_private_key_used_in_direct_udp_params() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .resource_mut::<AfterglowLightyearConfig>()
        .netcode_private_key = [0xAB; 32];
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    let p = app.world().resource::<PendingNetcodeStartup>();
    assert_eq!(p.client.as_ref().unwrap().private_key, [0xAB; 32]);
    assert_eq!(p.server.as_ref().unwrap().private_key, [0xAB; 32]);
}

#[test]
fn same_frame_joined_local_then_direct_udp() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .write_message(SessionEvent::Joined(SessionInfo {
            transport: SessionTransport::Local,
            ..info_fixture()
        }));
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.update();
    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_none());
    assert!(links.server_link.is_none());
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_some());
    assert!(pending.server.is_some());
}

#[test]
fn same_frame_joined_direct_udp_then_local() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .write_message(SessionEvent::Joined(udp_fixture()));
    app.world_mut()
        .write_message(SessionEvent::Joined(SessionInfo {
            transport: SessionTransport::Local,
            ..info_fixture()
        }));
    app.update();
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_none());
    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_some());
    assert!(links.server_link.is_some());
}

// ---------------------------------------------------------------------------
// Edge-review gap tests
// ---------------------------------------------------------------------------

#[test]
fn created_local_spawns_links() {
    let mut app = test_app(LightyearRole::Host);
    let info = SessionInfo {
        transport: SessionTransport::Local,
        ..info_fixture()
    };
    app.world_mut().write_message(SessionEvent::Created(info));
    app.update();
    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(
        links.client_link.is_some(),
        "Created(Local) should spawn client link"
    );
    assert!(
        links.server_link.is_some(),
        "Created(Local) should spawn server link"
    );
    assert!(
        links.server_entity.is_some(),
        "Created(Local) should spawn server entity"
    );
    let e = |e: Entity| app.world().entity(e);
    assert!(e(links.client_link.unwrap()).contains::<Client>());
    assert!(e(links.server_link.unwrap()).contains::<LinkOf>());
    assert!(e(links.server_entity.unwrap()).contains::<Server>());
}

#[test]
fn created_direct_udp_writes_pending_params() {
    let mut app = test_app(LightyearRole::Host);
    app.world_mut()
        .write_message(SessionEvent::Created(udp_fixture()));
    app.update();
    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(
        pending.client.is_some(),
        "Created(DirectUdp) should write client params"
    );
    assert!(
        pending.server.is_some(),
        "Created(DirectUdp) should write server params"
    );
    let client = pending.client.as_ref().unwrap();
    assert_eq!(client.server_addr, SocketAddr::from(([127, 0, 0, 1], 8820)));
    assert_eq!(client.client_id, 1);
}

#[test]
fn missing_afterglow_session_state_suppresses_client_params_but_allows_server() {
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
        role: LightyearRole::Host,
        ..Default::default()
    });
    // Register only the message resource so the bridge can run without the
    // session-state resource or the non-Steam provider system.
    app.add_message::<SessionEvent>();
    app.add_plugins(AfterglowSessionLightyearBridgePlugin);

    app.world_mut()
        .write_message(SessionEvent::Created(udp_fixture()));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(
        pending.client.is_none(),
        "no AfterglowSessionState → no client_id → client params suppressed"
    );
    assert!(
        pending.server.is_some(),
        "server params should still be written even without session state"
    );
}

#[test]
fn missing_afterglow_lightyear_config_falls_back_to_default() {
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
    app.add_plugins(AfterglowSessionPlugin);
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .local_member_id = SessionMemberId::new(1);
    app.add_plugins(AfterglowSessionLightyearBridgePlugin);
    // AfterglowLightyearConfig intentionally omitted — bridge uses default.

    app.world_mut()
        .write_message(SessionEvent::Created(udp_fixture()));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(
        pending.client.is_some(),
        "default role Client should write client params"
    );
    assert!(
        pending.server.is_none(),
        "default role Client should NOT write server params"
    );
    let client = pending.client.as_ref().unwrap();
    assert_eq!(client.protocol_id, 0, "default protocol_id is 0");
    assert_eq!(
        client.private_key, [0u8; 32],
        "default private_key is [0u8; 32]"
    );
}

#[test]
fn session_ended_after_local_despawns_links_and_clears_fields() {
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
    app.world_mut()
        .write_message(SessionEvent::SessionEnded(SessionId::new(1)));
    app.update();

    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(links.client_link.is_none());
    assert!(links.server_link.is_none());
    assert!(links.server_entity.is_none());

    assert!(app.world().get_entity(client_link).is_err());
    assert!(app.world().get_entity(server_link).is_err());
    assert!(app.world().get_entity(server_entity).is_err());

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_none());
}

#[test]
fn created_then_joined_same_frame_local_final_state() {
    let mut app = test_app(LightyearRole::Host);
    let info = SessionInfo {
        transport: SessionTransport::Local,
        ..info_fixture()
    };

    app.world_mut()
        .write_message(SessionEvent::Created(info.clone()));
    app.world_mut().write_message(SessionEvent::Joined(info));
    app.update();

    let pending = app.world().resource::<PendingNetcodeStartup>();
    assert!(pending.client.is_none());
    assert!(pending.server.is_none());

    let links = app.world().resource::<SessionLightyearLinks>();
    assert!(
        links.client_link.is_some(),
        "final state should have client link (from Joined)"
    );
    assert!(
        links.server_link.is_some(),
        "final state should have server link (from Joined)"
    );
    assert!(
        links.server_entity.is_some(),
        "final state should have server entity (from Joined)"
    );
}
