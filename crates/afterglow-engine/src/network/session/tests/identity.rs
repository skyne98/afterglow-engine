use super::{
    AfterglowSessionState, NativeIdentityProof, PlayerIdentity, SessionBackend, SessionConfig,
    SessionEvent, SessionMemberId, SessionRequest, in_process_provider, native_identity_for_create,
    native_identity_for_join, steam_identity_for_passthrough, test_app,
};

#[test]
fn create_session_with_invalid_identity_proof_is_rejected() {
    let mut app = test_app();
    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig::default(),
        PlayerIdentity::Native(NativeIdentityProof {
            public_key: vec![0u8; 32],
            signature: vec![0u8; 64],
        }),
    ));
    app.update();

    super::expect_error(&mut app, super::SessionError::PermissionDenied);
}

#[test]
fn native_identity_rejoin_returns_same_member_id() {
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
    let first_member_id = app
        .world()
        .resource::<AfterglowSessionState>()
        .local_member_id;
    super::drain_messages(&mut app);

    // Simulate a full Leave: clear both current_session and local_member_id,
    // matching what `handle_leave` and `update_session_status` do in
    // production.
    {
        let mut state = app
            .world_mut()
            .resource_mut::<AfterglowSessionState>();
        state.current_session = None;
        state.local_member_id = SessionMemberId::INVALID;
    }

    let identity = native_identity_for_join(session_id);
    app.world_mut().write_message(SessionRequest::Join {
        backend: SessionBackend::NonSteam,
        provider: in_process_provider(),
        session: session_id,
        identity,
    });
    app.update();

    assert_eq!(
        app.world()
            .resource::<AfterglowSessionState>()
            .local_member_id,
        first_member_id
    );

    let batch = super::drain_messages(&mut app);
    assert!(
        batch.iter().any(|e| matches!(e, SessionEvent::Joined(_))),
        "rejoin should emit Joined"
    );
    assert!(
        batch.iter().any(|e| matches!(
            e,
            SessionEvent::MemberJoined { member, .. } if *member == first_member_id
        )),
        "rejoin should emit MemberJoined carrying the rejoiner's existing member id"
    );
}

#[test]
fn invalid_native_signature_is_rejected() {
    let mut app = test_app();

    // Attacker signs with a different nonce than the server expects.
    let attacker_identity =
        PlayerIdentity::test_native(&[1u8; 32], SessionBackend::NonSteam, "create", 0);

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig::default(),
        attacker_identity,
    ));
    app.update();

    super::expect_error(&mut app, super::SessionError::PermissionDenied);
}

#[test]
fn steam_identity_create_passthrough_succeeds() {
    let mut app = test_app();
    let identity = steam_identity_for_passthrough(123456789);
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let state = app.world().resource::<AfterglowSessionState>();
    assert!(state.current_session.is_some());
}

#[test]
fn owner_identity_is_exposed_in_session_info() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig::default(),
        identity.clone(),
    ));
    app.update();

    let info = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info),
            _ => None,
        })
        .unwrap();

    assert_eq!(info.owner_identity, identity);
}
