use super::{non_steam::NonSteamSessionCatalog, *};

pub(crate) mod code;
pub(crate) mod edge_cases;
pub(crate) mod identity;

pub(super) fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, super::AfterglowSessionPlugin));
    app.insert_resource(SessionIdentityNonce([0u8; 32]));
    app
}

pub(super) fn drain_messages(app: &mut App) -> Vec<SessionEvent> {
    app.world_mut()
        .resource_mut::<Messages<SessionEvent>>()
        .drain()
        .collect()
}

pub(super) fn expect_error(app: &mut App, err: SessionError) {
    let batch = drain_messages(app);
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, SessionEvent::Error(e) if *e == err))
    );
}

pub(super) fn test_nonce() -> [u8; 32] {
    [0u8; 32]
}

pub(super) fn native_identity_for_create() -> PlayerIdentity {
    PlayerIdentity::test_native(&test_nonce(), SessionBackend::NonSteam, "create", 0)
}

pub(super) fn native_identity_for_join(session_id: SessionId) -> PlayerIdentity {
    PlayerIdentity::test_native(
        &test_nonce(),
        SessionBackend::NonSteam,
        &session_id.as_raw().to_string(),
        0,
    )
}

pub(super) fn native_identity_for_join_with_seed(
    session_id: SessionId,
    seed: u8,
) -> PlayerIdentity {
    PlayerIdentity::test_native(
        &test_nonce(),
        SessionBackend::NonSteam,
        &session_id.as_raw().to_string(),
        seed,
    )
}

pub(super) fn native_identity_for_join_by_code(code: &SessionCode) -> PlayerIdentity {
    PlayerIdentity::test_native(&test_nonce(), SessionBackend::NonSteam, code.as_str(), 0)
}

pub(super) fn native_identity_for_join_by_code_with_seed(
    code: &SessionCode,
    seed: u8,
) -> PlayerIdentity {
    PlayerIdentity::test_native(&test_nonce(), SessionBackend::NonSteam, code.as_str(), seed)
}

pub(super) fn steam_identity_for_passthrough(steam_id: u64) -> PlayerIdentity {
    PlayerIdentity::Steam {
        steam_id,
        ticket: vec![1, 2, 3],
    }
}

#[test]
fn plugin_registers_resources_and_messages() {
    let app = test_app();
    assert!(app.world().contains_resource::<AfterglowSessionState>());
    assert!(app.world().contains_resource::<NonSteamSessionCatalog>());
    assert!(app.world().contains_resource::<SessionIdentityNonce>());
}

#[test]
fn idle_update_does_not_allocate_local_member_id() {
    let mut app = test_app();
    app.update();

    assert_eq!(
        app.world()
            .resource::<AfterglowSessionState>()
            .local_member_id,
        SessionMemberId::INVALID
    );
}

#[test]
fn create_session_success_sets_current_and_emits_created_joined() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let state = app.world().resource::<AfterglowSessionState>();
    assert!(state.current_session.is_some());
    assert_eq!(state.current_backend, Some(SessionBackend::NonSteam));
    assert!(state.identity.is_some());

    let batch = drain_messages(&mut app);
    assert!(batch.iter().any(|e| matches!(e, SessionEvent::Created(_))));
    assert!(batch.iter().any(|e| matches!(e, SessionEvent::Joined(_))));
}

#[test]
fn invalid_zero_capacity_create_fails_and_remains_idle() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            max_members: 0,
            ..Default::default()
        },
        identity,
    ));
    app.update();
    assert!(
        app.world()
            .resource::<AfterglowSessionState>()
            .current_session
            .is_none()
    );
    expect_error(&mut app, SessionError::InvalidConfig);
}

#[test]
fn search_filters_by_metadata_and_open_slots() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            name: "coop-game".into(),
            backend: SessionBackend::NonSteam,
            metadata: [("mode".into(), "coop".into())].into(),
            max_members: 2,
            ..Default::default()
        },
        identity,
    ));
    app.update();
    drain_messages(&mut app);

    app.world_mut()
        .write_message(SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            metadata: [("mode".into(), "coop".into())].into(),
            require_open_slot: true,
            max_results: 10,
        }));
    app.update();

    let results = drain_messages(&mut app)
        .into_iter()
        .find_map(|e| {
            if let SessionEvent::SearchResults(r) = e {
                Some(r)
            } else {
                None
            }
        })
        .expect("should have search results");
    assert_eq!(results.len(), 1);

    app.world_mut()
        .write_message(SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            metadata: [("mode".into(), "pvp".into())].into(),
            ..Default::default()
        }));
    app.update();

    let results2 = drain_messages(&mut app)
        .into_iter()
        .find_map(|e| {
            if let SessionEvent::SearchResults(r) = e {
                Some(r)
            } else {
                None
            }
        })
        .expect("should have search results");
    assert!(results2.is_empty());
}

#[test]
fn join_full_session_fails() {
    let mut app = test_app();
    let session_id = {
        let identity = native_identity_for_create();
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        catalog.seed_session(
            SessionConfig {
                max_members: 1,
                ..Default::default()
            },
            SessionMemberId::new(999),
            identity,
        )
    };

    // Use a different key seed so the joiner is a new player, not a rejoin.
    let identity = native_identity_for_join_with_seed(session_id, 1);
    app.world_mut().write_message(SessionRequest::Join {
        backend: SessionBackend::NonSteam,
        session: session_id,
        identity,
    });
    app.update();
    assert!(
        app.world()
            .resource::<AfterglowSessionState>()
            .current_session
            .is_none()
    );
    expect_error(&mut app, SessionError::SessionFull);
}

#[test]
fn leave_current_session_clears_state_and_emits_left_ended_when_owner() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();
    assert!(
        app.world()
            .resource::<AfterglowSessionState>()
            .current_session
            .is_some()
    );
    drain_messages(&mut app);

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    assert!(
        app.world()
            .resource::<AfterglowSessionState>()
            .current_session
            .is_none()
    );
    assert_eq!(
        app.world()
            .resource::<AfterglowSessionState>()
            .current_backend,
        None
    );
    assert!(
        app.world()
            .resource::<AfterglowSessionState>()
            .identity
            .is_none()
    );

    let batch = drain_messages(&mut app);
    assert!(batch.iter().any(|e| matches!(e, SessionEvent::Left { .. })));
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, SessionEvent::SessionEnded(_)))
    );
}

#[test]
fn duplicate_create_while_in_session_rejected() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();
    drain_messages(&mut app);

    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();
    expect_error(&mut app, SessionError::AlreadyInSession);
}

#[test]
fn duplicate_join_while_in_session_rejected() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();
    let session_id = app
        .world()
        .resource::<AfterglowSessionState>()
        .current_session
        .unwrap();
    drain_messages(&mut app);

    let identity = native_identity_for_join(session_id);
    app.world_mut().write_message(SessionRequest::Join {
        backend: SessionBackend::NonSteam,
        session: session_id,
        identity,
    });
    app.update();
    expect_error(&mut app, SessionError::AlreadyInSession);
}

#[test]
fn search_max_results_enforced() {
    let mut app = test_app();
    {
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        for _ in 0..5 {
            catalog.seed_session(
                SessionConfig::default(),
                SessionMemberId::new(999),
                native_identity_for_create(),
            );
        }
    }

    app.world_mut()
        .write_message(SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            max_results: 3,
            ..Default::default()
        }));
    app.update();

    let results = drain_messages(&mut app)
        .into_iter()
        .find_map(|e| {
            if let SessionEvent::SearchResults(r) = e {
                Some(r)
            } else {
                None
            }
        })
        .expect("should have search results");
    assert_eq!(results.len(), 3);
}

#[test]
fn defaults_are_sensible() {
    let config = SessionConfig::default();
    assert!(config.max_members > 0);
    assert_eq!(config.backend, SessionBackend::NonSteam);
    assert_eq!(config.visibility, SessionVisibility::Private);
    assert_eq!(config.transport, SessionTransport::Local);

    let search = SessionSearch::default();
    assert_eq!(search.backend, SessionBackend::NonSteam);
    assert!(search.max_results > 0);
}

#[test]
fn id_invalid_semantics() {
    assert!(!SessionId::INVALID.is_valid());
    assert_eq!(SessionId::INVALID, SessionId::new(0));
    assert!(SessionId::new(1).is_valid());
    assert_eq!(SessionId::new(42).as_raw(), 42);

    assert!(!SessionMemberId::INVALID.is_valid());
    assert_eq!(SessionMemberId::INVALID, SessionMemberId::new(0));
    assert!(SessionMemberId::new(1).is_valid());
    assert_eq!(SessionMemberId::new(42).as_raw(), 42);
}
