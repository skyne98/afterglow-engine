use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use super::{
    native_identity_for_create, native_identity_for_join_by_code_with_seed, test_app, test_nonce,
    AfterglowSessionState, NonSteamSessionClient, NonSteamSessionProvider, ProviderEndpoint,
    SessionBackend, SessionConfig, SessionEvent, SessionRequest, SessionSearch,
    SessionIdentityNonce,
};

fn find_test_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn send_raw_request(stream: &mut TcpStream, request: &SessionRequest) {
    let bytes = postcard::to_allocvec(request).unwrap();
    stream.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
    stream.write_all(&bytes).unwrap();
    stream.flush().unwrap();
}

fn read_raw_event(stream: &mut TcpStream) -> Option<SessionEvent> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).ok()?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).ok()?;
    postcard::from_bytes(&payload).ok()
}

fn drive_app(app: &mut bevy::app::App, frames: usize) {
    for _ in 0..frames {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn provider_accepts_create_and_returns_session_event() {
    let mut app = test_app();
    let addr = find_test_addr();
    app.world_mut().insert_resource(NonSteamSessionProvider::new(addr).unwrap());
    drive_app(&mut app, 3);

    let mut client = TcpStream::connect(addr).unwrap();
    send_raw_request(
        &mut client,
        &SessionRequest::Create(SessionConfig::default(), native_identity_for_create()),
    );

    drive_app(&mut app, 5);

    let events: Vec<SessionEvent> = std::iter::from_fn(|| read_raw_event(&mut client))
        .take(2)
        .collect();
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Created(_))));
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Joined(_))));
}

#[test]
fn provider_search_by_ip_lists_sessions_and_join_by_code_works() {
    let mut app = test_app();
    let addr = find_test_addr();
    app.world_mut().insert_resource(NonSteamSessionProvider::new(addr).unwrap());
    drive_app(&mut app, 3);

    // Host creates a session.
    let mut host = TcpStream::connect(addr).unwrap();
    send_raw_request(
        &mut host,
        &SessionRequest::Create(
            SessionConfig {
                name: "test-room".into(),
                metadata: [("name".into(), "test-room".into())].into(),
                ..Default::default()
            },
            native_identity_for_create(),
        ),
    );

    drive_app(&mut app, 5);

    let mut code = None;
    while let Some(event) = read_raw_event(&mut host) {
        if let SessionEvent::Created(info) = &event {
            code = Some(info.code.clone());
        }
        if code.is_some() {
            break;
        }
    }
    let code = code.expect("host should have received Created");

    // Client searches the provider by IP.
    let mut joiner = TcpStream::connect(addr).unwrap();
    send_raw_request(
        &mut joiner,
        &SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::Udp(addr),
            metadata: [("name".into(), "test-room".into())].into(),
            require_open_slot: true,
            max_results: 10,
        }),
    );

    drive_app(&mut app, 5);

    let results = read_raw_event(&mut joiner);
    let results = match results {
        Some(SessionEvent::SearchResults(r)) => r,
        other => panic!("expected SearchResults, got {:?}", other),
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].code, code);

    // Client joins by code. Use a different seed so it is a distinct member.
    let identity = native_identity_for_join_by_code_with_seed(&code, 1);
    send_raw_request(
        &mut joiner,
        &SessionRequest::JoinByCode {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::Udp(addr),
            code: code.clone(),
            identity,
        },
    );

    drive_app(&mut app, 20);

    let joined = read_raw_event(&mut joiner);
    assert!(
        matches!(joined, Some(SessionEvent::Joined(_))),
        "joiner should receive Joined, got {:?}",
        joined
    );

    // Drain the host's own Create/Joined events first.
    read_raw_event(&mut host);

    let member_joined = read_raw_event(&mut host);
    assert!(
        matches!(member_joined, Some(SessionEvent::MemberJoined { .. })),
        "host should receive MemberJoined, got {:?}",
        member_joined
    );
}

#[test]
fn bevy_client_sends_request_and_receives_event() {
    let mut provider_app = test_app();
    let addr = find_test_addr();
    provider_app
        .world_mut()
        .insert_resource(NonSteamSessionProvider::new(addr).unwrap());
    drive_app(&mut provider_app, 3);

    let mut client_app = test_app();
    client_app
        .world_mut()
        .insert_resource(SessionIdentityNonce(test_nonce()));
    drive_app(&mut client_app, 2);

    client_app
        .world_mut()
        .resource_mut::<NonSteamSessionClient>()
        .send_request(
            &ProviderEndpoint::Udp(addr),
            &SessionRequest::Create(SessionConfig::default(), native_identity_for_create()),
        )
        .unwrap();

    let mut found = false;
    for _ in 0..60 {
        provider_app.update();
        client_app.update();
        if super::drain_messages(&mut client_app)
            .iter()
            .any(|e| matches!(e, SessionEvent::Created(_)))
        {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(found, "client app should receive Created event");

    // One more update so ApplyEffects can update AfterglowSessionState.
    client_app.update();
    assert!(
        client_app
            .world()
            .resource::<AfterglowSessionState>()
            .current_session
            .is_some(),
        "client should be in a session"
    );
}
