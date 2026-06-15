use super::*;

#[test]
fn join_nonexistent_session_returns_error() {
    let mut app = test_app();
    let identity = native_identity_for_join(SessionId::new(999));

    app.world_mut().write_message(SessionRequest::Join {
        backend: SessionBackend::NonSteam,
        provider: in_process_provider(),
        session: SessionId::new(999),
        identity,
    });
    app.update();

    expect_error(&mut app, SessionError::SessionNotFound);
}

#[test]
fn leave_without_session_returns_error() {
    let mut app = test_app();

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    expect_error(&mut app, SessionError::NotInSession);
}

#[test]
fn leave_for_steam_backend_reports_backend_unavailable_without_clearing_state() {
    let mut app = test_app();
    let session = SessionId::new(777);
    {
        let mut state = app.world_mut().resource_mut::<AfterglowSessionState>();
        state.local_member_id = SessionMemberId::new(42);
        state.current_session = Some(session);
        state.current_backend = Some(SessionBackend::Steam);
    }

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    expect_error(&mut app, SessionError::BackendUnavailable);
    let state = app.world().resource::<AfterglowSessionState>();
    assert_eq!(state.current_session, Some(session));
    assert_eq!(state.current_backend, Some(SessionBackend::Steam));
}

#[test]
fn steam_backend_unavailable() {
    let mut app = test_app();

    app.world_mut().write_message(SessionRequest::Create(
        SessionConfig {
            backend: SessionBackend::Steam,
            ..Default::default()
        },
        steam_identity_for_passthrough(1),
    ));
    app.update();
    expect_error(&mut app, SessionError::BackendUnavailable);

    app.world_mut()
        .write_message(SessionRequest::Search(SessionSearch {
            backend: SessionBackend::Steam,
            ..Default::default()
        }));
    app.update();
    expect_error(&mut app, SessionError::BackendUnavailable);

    app.world_mut().write_message(SessionRequest::Join {
        backend: SessionBackend::Steam,
        provider: ProviderEndpoint::Steam,
        session: SessionId::new(1),
        identity: steam_identity_for_passthrough(1),
    });
    app.update();
    expect_error(&mut app, SessionError::BackendUnavailable);
}

#[test]
fn max_results_zero_returns_empty() {
    let mut app = test_app();
    {
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        catalog.seed_session(
            SessionConfig::default(),
            SessionMemberId::new(999),
            native_identity_for_create(),
        );
    }

    app.world_mut()
        .write_message(SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            max_results: 0,
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
    assert!(results.is_empty());
}

#[test]
fn duplicate_member_join_does_not_duplicate() {
    let mut app = test_app();

    let member_id = SessionMemberId::new(42);
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .local_member_id = member_id;

    let session_id = {
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        let sid = catalog.seed_session(
            SessionConfig {
                max_members: 4,
                ..Default::default()
            },
            SessionMemberId::new(100),
            native_identity_for_create(),
        );
        // The stored identity must use the join target so rejoin verification
        // succeeds.
        let member_identity = PlayerIdentity::test_native(
            &test_nonce(),
            SessionBackend::NonSteam,
            &sid.as_raw().to_string(),
            1,
        );
        catalog.add_member(sid, member_id, member_identity.clone());
        (sid, member_identity)
    };

    app.world_mut().write_message(SessionRequest::Join {
        backend: super::SessionBackend::NonSteam,
        session: session_id.0,
        identity: session_id.1,
        provider: in_process_provider(),
    });
    app.update();
    let batch = drain_messages(&mut app);

    assert!(batch.iter().any(|e| matches!(e, SessionEvent::Joined(_))));

    // MemberJoined is now emitted on every join (including rejoin) so the
    // remote player can re-learn their own member id after a Leave cleared
    // it locally. Dedup is verified at the catalog level, not via the event
    // stream.
    let catalog = app.world().resource::<NonSteamSessionCatalog>();
    let entry = catalog
        .sessions
        .get(&session_id.0)
        .expect("session still exists");
    assert_eq!(
        entry.members.iter().filter(|m| **m == member_id).count(),
        1,
        "rejoin must not duplicate the member in the catalog"
    );
}

#[test]
fn owner_leave_notifies_remaining_members_and_ends() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();
    drain_messages(&mut app);

    let other_a_identity = native_identity_for_create();
    let other_b_identity = native_identity_for_create();
    // Rotate the deterministic key between helpers so the public keys differ.
    // Because the helper always uses the all-zero test key, we intentionally
    // rely on rejoin detection only in tests that exercise it. Here we just
    // need distinct member slots; the catalog helper below creates them.
    let other_a = SessionMemberId::new(200);
    let other_b = SessionMemberId::new(201);
    let session_id = app
        .world()
        .resource::<AfterglowSessionState>()
        .current_session
        .unwrap();
    {
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        catalog.add_member(session_id, other_a, other_a_identity);
        catalog.add_member(session_id, other_b, other_b_identity);
    }

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let batch = drain_messages(&mut app);
    assert!(batch.iter().any(|e| matches!(e, SessionEvent::Left { .. })));
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, SessionEvent::SessionEnded(_)))
    );

    let kicked: Vec<&SessionMemberId> = batch
        .iter()
        .filter_map(|e| {
            if let SessionEvent::MemberLeft {
                member,
                reason: SessionLeaveReason::HostEnded,
                ..
            } = e
            {
                Some(member)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(kicked.len(), 2);
    assert!(kicked.contains(&&other_a));
    assert!(kicked.contains(&&other_b));
}

#[test]
fn stale_missing_session_leave_reports_host_ended() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();
    drain_messages(&mut app);

    let session_id = app
        .world()
        .resource::<AfterglowSessionState>()
        .current_session
        .unwrap();
    app.world_mut()
        .resource_mut::<NonSteamSessionCatalog>()
        .remove_session(session_id);

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let state = app.world().resource::<AfterglowSessionState>();
    assert!(state.current_session.is_none());
    assert_eq!(state.current_backend, None);

    let batch = drain_messages(&mut app);
    assert!(batch.iter().any(|e| matches!(
        e,
        SessionEvent::Left {
            reason: SessionLeaveReason::HostEnded,
            ..
        }
    )));
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, SessionEvent::SessionEnded(_)))
    );
}
