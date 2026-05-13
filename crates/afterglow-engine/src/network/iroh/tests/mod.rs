use super::*;
use crate::network::{
    handshake::{
        HandshakeReport, NetworkBackendKind, NetworkHandshakeConfig, service_control_handshake,
    },
    session::{NetworkSession, PlatformIdentity},
};
use std::time::{Duration, Instant};

mod handshake;
mod transport;

pub(super) fn poll_until(
    transport: &mut IrohTransport,
    matches: impl Fn(&[TransportEvent]) -> bool,
) -> Vec<TransportEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        transport.poll_events(&mut events);
        if matches(&events) {
            return events;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    events
}

pub(super) fn poll_for(transport: &mut IrohTransport, duration: Duration) -> Vec<TransportEvent> {
    let deadline = Instant::now() + duration;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        transport.poll_events(&mut events);
        std::thread::sleep(Duration::from_millis(10));
    }
    events
}

pub(super) fn connect_pair() -> (IrohTransport, IrohTransport) {
    bind_pair(ProtocolVersion::CURRENT, ProtocolVersion::CURRENT)
}

pub(super) fn bind_pair(
    client_protocol: ProtocolVersion,
    server_protocol: ProtocolVersion,
) -> (IrohTransport, IrohTransport) {
    let mut client_config = IrohTransportConfig::local_only();
    client_config.protocol = client_protocol;
    let client = IrohTransport::bind(PeerId(1), client_config).unwrap();
    let mut server_config = IrohTransportConfig::local_only();
    server_config.protocol = server_protocol;
    server_config.next_inbound_peer = 1;
    let server = IrohTransport::bind(PeerId(2), server_config).unwrap();
    (client, server)
}

pub(super) fn wait_for_connection(client: &mut IrohTransport, server: &mut IrohTransport) {
    client.connect(PeerId(2), server.endpoint_addr());
    let client_events = poll_until(client, |events| {
        events.contains(&TransportEvent::Connected(PeerId(2)))
    });
    let server_events = poll_until(server, |events| {
        events.contains(&TransportEvent::Connected(PeerId(1)))
    });
    assert!(client_events.contains(&TransportEvent::Connected(PeerId(2))));
    assert!(server_events.contains(&TransportEvent::Connected(PeerId(1))));
}

pub(super) fn handshake_config(label: &str) -> NetworkHandshakeConfig {
    NetworkHandshakeConfig {
        protocol: ProtocolVersion::CURRENT,
        build_hash: "iroh-test-build".into(),
        content_hash: "iroh-test-content".into(),
        backend: NetworkBackendKind::Iroh,
        identity: PlatformIdentity::Iroh {
            node_id: label.into(),
        },
    }
}

pub(super) fn mismatched_protocol() -> ProtocolVersion {
    ProtocolVersion {
        major: 9,
        minor: 0,
        patch: 0,
    }
}

pub(super) fn handshake_pair(
    client: &mut IrohTransport,
    client_session: &mut NetworkSession,
    client_config: &NetworkHandshakeConfig,
    server: &mut IrohTransport,
    server_session: &mut NetworkSession,
    server_config: &NetworkHandshakeConfig,
) -> (Vec<TransportEvent>, Vec<TransportEvent>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut client_events = Vec::new();
    let mut server_events = Vec::new();
    while Instant::now() < deadline {
        service_control_handshake(client, client_session, client_config, &mut client_events);
        service_control_handshake(server, server_session, server_config, &mut server_events);
        if client_session.peer(PeerId(2)).is_some() && server_session.peer(PeerId(1)).is_some() {
            return (client_events, server_events);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    (client_events, server_events)
}

pub(super) fn poll_service_until(
    transport: &mut IrohTransport,
    session: &mut NetworkSession,
    config: &NetworkHandshakeConfig,
    matches: impl Fn(&[TransportEvent], &NetworkSession, &HandshakeReport) -> bool,
) -> (Vec<TransportEvent>, HandshakeReport) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut app_events = Vec::new();
    let mut last_report = HandshakeReport::default();
    while Instant::now() < deadline {
        last_report = service_control_handshake(transport, session, config, &mut app_events);
        if matches(&app_events, session, &last_report) {
            return (app_events, last_report);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    (app_events, last_report)
}

pub(super) fn packet_payloads(events: &[TransportEvent]) -> Vec<Vec<u8>> {
    events
        .iter()
        .filter_map(|event| match event {
            TransportEvent::Packet(packet) => Some(packet.payload.clone()),
            _ => None,
        })
        .collect()
}
