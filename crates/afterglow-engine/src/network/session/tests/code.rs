use super::{
    AfterglowSessionState, NonSteamSessionCatalog, SESSION_CODE_ALPHABET, SESSION_CODE_CHAR_LEN,
    SESSION_CODE_GROUPS, SessionCode, SessionConfig, SessionEvent, SessionMemberId, SessionRequest,
    in_process_provider, native_identity_for_create, native_identity_for_join_by_code,
    native_identity_for_join_by_code_with_seed, test_app,
};

#[test]
fn create_session_generates_memorable_code() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let infos: Vec<super::SessionInfo> = super::drain_messages(&mut app)
        .into_iter()
        .filter_map(|e| {
            if let SessionEvent::Created(info) = e {
                Some(info)
            } else {
                None
            }
        })
        .collect();

    assert_eq!(infos.len(), 1);
    let code = &infos[0].code;
    assert!(
        code.as_str().len() >= SESSION_CODE_CHAR_LEN + (SESSION_CODE_GROUPS.saturating_sub(1)),
        "code should be at least {SESSION_CODE_CHAR_LEN} chars plus separators, got {}",
        code.as_str()
    );
    assert!(
        code.as_str().contains('-'),
        "code should be hyphenated for readability: {}",
        code.as_str()
    );
    assert!(
        code.as_str()
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '-')
    );
}

#[test]
fn created_and_joined_events_share_same_code() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let batch = super::drain_messages(&mut app);
    let created = batch.iter().find_map(|e| match e {
        SessionEvent::Created(info) => Some(info.code.clone()),
        _ => None,
    });
    let joined = batch.iter().find_map(|e| match e {
        SessionEvent::Joined(info) => Some(info.code.clone()),
        _ => None,
    });

    assert_eq!(created, joined);
}

#[test]
fn join_by_code_succeeds_and_sets_state() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let code = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info.code),
            _ => None,
        })
        .expect("should have created code");

    // Simulate a second client by clearing the current session state.
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .current_session = None;

    // Use a different key so the second player is a new member.
    let identity = native_identity_for_join_by_code_with_seed(&code, 1);
    app.world_mut().write_message(SessionRequest::JoinByCode {
        backend: super::SessionBackend::NonSteam,
        provider: in_process_provider(),
        code,
        identity,
    });
    app.update();

    assert!(
        app.world()
            .resource::<AfterglowSessionState>()
            .current_session
            .is_some()
    );

    let batch = super::drain_messages(&mut app);
    assert!(
        batch.iter().any(|e| matches!(e, SessionEvent::Joined(_))),
        "join by code should emit Joined"
    );
    assert!(
        batch
            .iter()
            .any(|e| matches!(e, SessionEvent::MemberJoined { .. })),
        "join by code should emit MemberJoined for the new player"
    );
}

#[test]
fn join_by_missing_code_returns_not_found() {
    let mut app = test_app();
    let code = SessionCode::new("ZZZ-ZZZ");
    let identity = native_identity_for_join_by_code(&code);
    app.world_mut().write_message(SessionRequest::JoinByCode {
        backend: super::SessionBackend::NonSteam,
        provider: in_process_provider(),
        code,
        identity,
    });
    app.update();

    super::expect_error(&mut app, super::SessionError::SessionNotFound);
}

#[test]
fn multiple_sessions_receive_unique_codes() {
    let mut app = test_app();

    let mut codes = std::collections::HashSet::new();
    for _ in 0..10 {
        let identity = native_identity_for_create();
        app.world_mut()
            .write_message(SessionRequest::Create(SessionConfig::default(), identity));
        app.update();

        let code = super::drain_messages(&mut app)
            .into_iter()
            .find_map(|e| match e {
                SessionEvent::Created(info) => Some(info.code),
                _ => None,
            })
            .unwrap();
        assert!(
            codes.insert(code.clone()),
            "duplicate session code generated: {}",
            code.as_str()
        );

        // Simulate a fresh client so the next create is accepted.
        app.world_mut()
            .resource_mut::<AfterglowSessionState>()
            .current_session = None;
    }
}

#[test]
fn session_code_generation_is_deterministic_for_same_counter() {
    let first = SessionCode::generate(1);
    let second = SessionCode::generate(1);
    let different = SessionCode::generate(2);

    assert_eq!(first, second);
    assert_ne!(first, different);
}

#[test]
fn create_avoids_colliding_session_code() {
    let mut app = test_app();

    // Pre-fill the catalog so the first candidate would collide.
    {
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        catalog.used_codes.insert(SessionCode::generate(1));
        catalog.next_code_seed = 1;
    }

    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let code = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info.code),
            _ => None,
        })
        .unwrap();

    assert_ne!(
        code,
        SessionCode::generate(1),
        "create should skip the colliding code and pick the next free one"
    );
}

#[test]
fn code_only_uses_allowed_characters() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let code = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info.code),
            _ => None,
        })
        .unwrap();

    let allowed: std::collections::HashSet<char> = std::str::from_utf8(SESSION_CODE_ALPHABET)
        .unwrap()
        .chars()
        .collect();
    for ch in code.as_str().chars() {
        if ch == '-' {
            continue;
        }
        assert!(
            allowed.contains(&ch),
            "character {ch:?} in {code:?} is not in the session-code alphabet",
        );
    }
}

#[test]
fn code_remains_allocated_after_owner_leave_to_prevent_replay() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let first_code = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info.code),
            _ => None,
        })
        .unwrap();

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();
    super::drain_messages(&mut app);

    let catalog = app.world().resource::<NonSteamSessionCatalog>();
    // Codes are never freed: a freed code could be reassigned to a later
    // session, allowing replay of an old identity proof signed against the
    // same target string. See security note in `handle_leave`.
    assert!(
        catalog.used_codes.contains(&first_code),
        "old session code should remain allocated after owner leaves"
    );
    assert!(
        catalog.sessions.is_empty(),
        "old session should have been removed by owner leave"
    );

    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let second_code = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info.code),
            _ => None,
        })
        .unwrap();

    assert_ne!(first_code, second_code);
    assert!(
        app.world()
            .resource::<NonSteamSessionCatalog>()
            .used_codes
            .contains(&second_code)
    );
}

#[test]
fn join_by_code_while_already_in_session_rejected() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let code = super::drain_messages(&mut app)
        .into_iter()
        .find_map(|e| match e {
            SessionEvent::Created(info) => Some(info.code),
            _ => None,
        })
        .unwrap();

    // Re-join the same session by code while already in it.
    let identity = native_identity_for_join_by_code(&code);
    app.world_mut().write_message(SessionRequest::JoinByCode {
        backend: super::SessionBackend::NonSteam,
        provider: in_process_provider(),
        code,
        identity,
    });
    app.update();

    super::expect_error(&mut app, super::SessionError::AlreadyInSession);
}

#[test]
fn join_by_code_full_session_rejected() {
    let mut app = test_app();
    let code = {
        let identity = native_identity_for_create();
        let mut catalog = app.world_mut().resource_mut::<NonSteamSessionCatalog>();
        let sid = catalog.seed_session(
            SessionConfig {
                max_members: 1,
                ..Default::default()
            },
            SessionMemberId::new(100),
            identity,
        );
        catalog.sessions[&sid].code.clone()
    };

    // Use a different key so the joiner is a new player.
    app.world_mut()
        .resource_mut::<AfterglowSessionState>()
        .current_session = None;

    let identity = native_identity_for_join_by_code_with_seed(&code, 1);
    app.world_mut().write_message(SessionRequest::JoinByCode {
        backend: super::SessionBackend::NonSteam,
        provider: in_process_provider(),
        code,
        identity,
    });
    app.update();

    super::expect_error(&mut app, super::SessionError::SessionFull);
}
