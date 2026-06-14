use super::{
    AfterglowSessionState, SessionConfig, SessionConnectionState, SessionError, SessionEvent,
    SessionInfo, SessionRequest, SessionStatus, native_identity_for_create, test_app,
};

#[test]
fn status_starts_idle() {
    let app = test_app();
    let status = app.world().resource::<SessionStatus>();
    assert!(!status.is_in_session());
    assert_eq!(status.state, SessionConnectionState::Idle);
    assert!(status.members.is_empty());
    assert!(status.info.is_none());
}

#[test]
fn create_updates_status_to_joining() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert!(status.is_in_session());
    assert_eq!(status.state, SessionConnectionState::Joining);
    assert!(status.info.is_some());
    assert_eq!(status.member_count(), 1);
    assert_eq!(status.members.len(), 1);
}

#[test]
fn leave_clears_status() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert!(!status.is_in_session());
    assert_eq!(status.state, SessionConnectionState::Idle);
    assert!(status.members.is_empty());
    assert!(status.info.is_none());
}

#[test]
fn error_sets_status_error() {
    let mut app = test_app();
    // Try to leave while not in a session.
    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert!(!status.is_in_session());
    assert_eq!(
        status.state,
        SessionConnectionState::Error(SessionError::NotInSession)
    );
    assert_eq!(status.is_error(), Some(&SessionError::NotInSession));
}

#[test]
fn status_member_events_update_member_list() {
    let mut app = test_app();

    // Owner creates.
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    // Simulate a second player joining by injecting a MemberJoined event.
    let session = app
        .world()
        .resource::<AfterglowSessionState>()
        .current_session
        .unwrap();
    let second_member = super::SessionMemberId::new(99);
    app.world_mut().write_message(SessionEvent::MemberJoined {
        session,
        member: second_member,
    });
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert_eq!(status.member_count(), 2);
    assert!(status.members.contains(&second_member));

    // And leaving removes them.
    app.world_mut().write_message(SessionEvent::MemberLeft {
        session,
        member: second_member,
        reason: super::SessionLeaveReason::Left,
    });
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert_eq!(status.member_count(), 1);
    assert!(!status.members.contains(&second_member));
}

#[test]
fn duplicate_member_joined_event_is_deduplicated() {
    let mut app = test_app();

    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let member = super::SessionMemberId::new(7);
    let session = app
        .world()
        .resource::<AfterglowSessionState>()
        .current_session
        .unwrap();

    app.world_mut()
        .write_message(SessionEvent::MemberJoined { session, member });
    app.update();
    app.world_mut()
        .write_message(SessionEvent::MemberJoined { session, member });
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert_eq!(status.member_count(), 2); // owner + one duplicate-suppressed member
}

#[test]
fn state_helper_is_in_session_matches_status() {
    let mut app = test_app();
    let identity = native_identity_for_create();
    app.world_mut()
        .write_message(SessionRequest::Create(SessionConfig::default(), identity));
    app.update();

    let state = app.world().resource::<AfterglowSessionState>();
    assert!(state.is_in_session());

    app.world_mut().write_message(SessionRequest::Leave);
    app.update();

    let state = app.world().resource::<AfterglowSessionState>();
    assert!(!state.is_in_session());
}

#[test]
fn search_results_update_last_search_results() {
    let mut app = test_app();
    let info = SessionInfo {
        id: super::SessionId::new(42),
        code: super::SessionCode::new("ABC-DEF"),
        backend: super::SessionBackend::NonSteam,
        name: "listed".into(),
        owner: super::SessionMemberId::new(1),
        owner_identity: native_identity_for_create(),
        member_count: 1,
        max_members: 4,
        visibility: super::SessionVisibility::Public,
        metadata: Default::default(),
        transport: super::SessionTransport::Local,
    };
    app.world_mut()
        .write_message(SessionEvent::SearchResults(vec![info.clone()]));
    app.update();

    let status = app.world().resource::<SessionStatus>();
    assert_eq!(status.last_search_results.len(), 1);
    assert_eq!(status.last_search_results[0].code, info.code);
}
