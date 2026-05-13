use crate::network::{
    DeliveryMode, DisconnectReason, NetChannel, NetworkPacket, NetworkTransport, PacketHeader,
    PeerId, ProtocolVersion, TransportEvent, accepts_unreliable_sequence, decode_transport_packet,
    encode_transport_packet,
};
use bytes::Bytes;
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, presets},
};
use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender, TryRecvError, channel},
    thread::{self, JoinHandle},
};
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};

pub const AFTERGLOW_IROH_ALPN: &[u8] = b"/afterglow/0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrohTransportConfig {
    pub protocol: ProtocolVersion,
    pub alpn: Vec<u8>,
    pub relay_mode: IrohRelayMode,
    pub next_inbound_peer: u64,
    pub max_packet_bytes: usize,
}

impl Default for IrohTransportConfig {
    fn default() -> Self {
        Self {
            protocol: ProtocolVersion::CURRENT,
            alpn: AFTERGLOW_IROH_ALPN.to_vec(),
            relay_mode: IrohRelayMode::N0,
            next_inbound_peer: 1,
            max_packet_bytes: 256 * 1024,
        }
    }
}

impl IrohTransportConfig {
    pub fn local_only() -> Self {
        Self {
            relay_mode: IrohRelayMode::Disabled,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IrohRelayMode {
    N0,
    Disabled,
}

#[derive(Debug, Error)]
pub enum IrohTransportError {
    #[error("failed to start iroh worker: {0}")]
    Startup(String),
    #[error("iroh worker stopped before reporting startup")]
    StartupClosed,
}

pub struct IrohTransport {
    local_peer: PeerId,
    protocol: ProtocolVersion,
    next_sequence: u64,
    delivered_sequences: HashMap<(PeerId, NetChannel), u64>,
    endpoint_addr: EndpointAddr,
    cmd_tx: mpsc::UnboundedSender<IrohCommand>,
    event_rx: Receiver<TransportEvent>,
    worker: Option<JoinHandle<()>>,
}

enum IrohCommand {
    Connect {
        peer: PeerId,
        addr: EndpointAddr,
    },
    Send(NetworkPacket),
    Disconnect {
        peer: PeerId,
        reason: DisconnectReason,
    },
    Shutdown,
}

impl IrohTransport {
    pub fn bind(
        local_peer: PeerId,
        config: IrohTransportConfig,
    ) -> Result<Self, IrohTransportError> {
        let (ready_tx, ready_rx) = channel();
        let (event_tx, event_rx) = channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let thread_config = config.clone();

        let worker = thread::Builder::new()
            .name("afterglow-iroh-transport".into())
            .spawn(move || run_worker(local_peer, thread_config, cmd_rx, event_tx, ready_tx))
            .map_err(|err| IrohTransportError::Startup(err.to_string()))?;

        let endpoint_addr = ready_rx
            .recv()
            .map_err(|_| IrohTransportError::StartupClosed)?
            .map_err(IrohTransportError::Startup)?;

        Ok(Self {
            local_peer,
            protocol: config.protocol,
            next_sequence: 0,
            delivered_sequences: HashMap::new(),
            endpoint_addr,
            cmd_tx,
            event_rx,
            worker: Some(worker),
        })
    }

    pub fn endpoint_addr(&self) -> EndpointAddr {
        self.endpoint_addr.clone()
    }

    pub fn connect(&self, peer: PeerId, addr: EndpointAddr) {
        let _ = self.cmd_tx.send(IrohCommand::Connect { peer, addr });
    }
}

impl Drop for IrohTransport {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(IrohCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl NetworkTransport for IrohTransport {
    fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    fn poll_events(&mut self, out: &mut Vec<TransportEvent>) {
        loop {
            match self.event_rx.try_recv() {
                Ok(event) => match &event {
                    TransportEvent::Connected(peer) => {
                        self.delivered_sequences.retain(|(from, _), _| from != peer);
                        out.push(event);
                    }
                    TransportEvent::Disconnected { peer, .. } => {
                        self.delivered_sequences.retain(|(from, _), _| from != peer);
                        out.push(event);
                    }
                    TransportEvent::Packet(packet) => {
                        if accepts_unreliable_sequence(&mut self.delivered_sequences, packet) {
                            out.push(event);
                        }
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    out.push(TransportEvent::Disconnected {
                        peer: self.local_peer,
                        reason: DisconnectReason::Transport("iroh worker stopped".into()),
                    });
                    break;
                }
            }
        }
    }

    fn send(&mut self, to: PeerId, channel: NetChannel, delivery: DeliveryMode, payload: Vec<u8>) {
        let packet = NetworkPacket {
            from: self.local_peer,
            to,
            header: PacketHeader {
                protocol: self.protocol,
                channel,
                delivery,
                sequence: self.next_sequence,
            },
            payload,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        let _ = self.cmd_tx.send(IrohCommand::Send(packet));
    }

    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason) {
        let _ = self.cmd_tx.send(IrohCommand::Disconnect { peer, reason });
    }
}

fn run_worker(
    local_peer: PeerId,
    config: IrohTransportConfig,
    cmd_rx: mpsc::UnboundedReceiver<IrohCommand>,
    event_tx: Sender<TransportEvent>,
    ready_tx: Sender<Result<EndpointAddr, String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = ready_tx.send(Err(err.to_string()));
            return;
        }
    };
    runtime.block_on(worker_main(local_peer, config, cmd_rx, event_tx, ready_tx));
}

async fn worker_main(
    local_peer: PeerId,
    config: IrohTransportConfig,
    mut cmd_rx: mpsc::UnboundedReceiver<IrohCommand>,
    event_tx: Sender<TransportEvent>,
    ready_tx: Sender<Result<EndpointAddr, String>>,
) {
    let endpoint = match bind_endpoint(&config).await {
        Ok(endpoint) => endpoint,
        Err(err) => {
            let _ = ready_tx.send(Err(err));
            return;
        }
    };
    let endpoint_addr = endpoint.addr();
    let _ = ready_tx.send(Ok(endpoint_addr));

    let state = WorkerState::new(local_peer, config, event_tx);
    let accept_endpoint = endpoint.clone();
    let accept_state = state.clone();
    tokio::spawn(async move {
        accept_loop(accept_endpoint, accept_state).await;
    });

    while let Some(command) = cmd_rx.recv().await {
        match command {
            IrohCommand::Connect { peer, addr } => {
                let endpoint = endpoint.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    connect_peer(endpoint, state, peer, addr).await;
                });
            }
            IrohCommand::Send(packet) => {
                state.send_packet(packet).await;
            }
            IrohCommand::Disconnect { peer, reason } => {
                state.disconnect_peer(peer, reason).await;
            }
            IrohCommand::Shutdown => break,
        }
    }
    endpoint.close().await;
}

async fn bind_endpoint(config: &IrohTransportConfig) -> Result<Endpoint, String> {
    let builder = match config.relay_mode {
        IrohRelayMode::N0 => Endpoint::builder(presets::N0),
        IrohRelayMode::Disabled => Endpoint::builder(presets::N0DisableRelay),
    }
    .alpns(vec![config.alpn.clone()]);
    builder.bind().await.map_err(|err| err.to_string())
}

#[derive(Clone)]
struct WorkerState {
    local_peer: PeerId,
    config: IrohTransportConfig,
    event_tx: Sender<TransportEvent>,
    peers: std::sync::Arc<Mutex<HashMap<PeerId, TrackedConnection>>>,
    next_inbound_peer: std::sync::Arc<Mutex<u64>>,
}

#[derive(Clone)]
struct TrackedConnection {
    stable_id: usize,
    connection: Connection,
}

impl WorkerState {
    fn new(
        local_peer: PeerId,
        config: IrohTransportConfig,
        event_tx: Sender<TransportEvent>,
    ) -> Self {
        Self {
            local_peer,
            next_inbound_peer: std::sync::Arc::new(Mutex::new(config.next_inbound_peer)),
            config,
            event_tx,
            peers: Default::default(),
        }
    }

    async fn insert_connection(&self, peer: PeerId, connection: Connection) {
        let tracked = TrackedConnection {
            stable_id: connection.stable_id(),
            connection: connection.clone(),
        };
        if let Some(replaced) = self.peers.lock().await.insert(peer, tracked) {
            replaced
                .connection
                .close(0u32.into(), b"connection replaced");
        }
        let _ = self.event_tx.send(TransportEvent::Connected(peer));
        spawn_connection_readers(peer, self.clone(), connection);
    }

    async fn send_packet(&self, packet: NetworkPacket) {
        let Some(connection) = self
            .peers
            .lock()
            .await
            .get(&packet.to)
            .map(|tracked| tracked.connection.clone())
        else {
            return;
        };
        let Ok(bytes) = encode_transport_packet(&packet) else {
            return;
        };
        match packet.header.delivery {
            DeliveryMode::Reliable => {
                if let Ok(mut stream) = connection.open_uni().await {
                    let _ = stream.write_all(&bytes).await;
                    let _ = stream.finish();
                }
            }
            DeliveryMode::Unreliable | DeliveryMode::UnreliableSequenced => {
                let _ = connection.send_datagram(Bytes::from(bytes));
            }
        }
    }

    async fn disconnect_peer(&self, peer: PeerId, reason: DisconnectReason) {
        if let Some(tracked) = self.peers.lock().await.remove(&peer) {
            let connection = tracked.connection;
            connection.close(0u32.into(), format!("{reason:?}").as_bytes());
        }
        let _ = self
            .event_tx
            .send(TransportEvent::Disconnected { peer, reason });
    }

    async fn notify_remote_closed(&self, peer: PeerId, stable_id: usize) {
        let removed = {
            let mut peers = self.peers.lock().await;
            if self.peer_has_connection(&peers, peer, stable_id) {
                peers.remove(&peer);
                true
            } else {
                false
            }
        };
        if removed {
            let _ = self.event_tx.send(TransportEvent::Disconnected {
                peer,
                reason: DisconnectReason::Remote,
            });
        }
    }

    async fn emit_packet_if_current(&self, peer: PeerId, stable_id: usize, packet: NetworkPacket) {
        let peers = self.peers.lock().await;
        if self.peer_has_connection(&peers, peer, stable_id) {
            let _ = self.event_tx.send(TransportEvent::Packet(packet));
        }
    }

    fn peer_has_connection(
        &self,
        peers: &HashMap<PeerId, TrackedConnection>,
        peer: PeerId,
        stable_id: usize,
    ) -> bool {
        peers
            .get(&peer)
            .is_some_and(|tracked| tracked.stable_id == stable_id)
    }

    async fn allocate_inbound_peer(&self) -> PeerId {
        let mut next = self.next_inbound_peer.lock().await;
        loop {
            let peer = PeerId(*next);
            *next = next.saturating_add(1);
            if peer != self.local_peer && !self.peers.lock().await.contains_key(&peer) {
                return peer;
            }
        }
    }
}

async fn accept_loop(endpoint: Endpoint, state: WorkerState) {
    while let Some(incoming) = endpoint.accept().await {
        match incoming.await {
            Ok(connection) => {
                let peer = state.allocate_inbound_peer().await;
                state.insert_connection(peer, connection).await;
            }
            Err(err) => {
                let _ = state.event_tx.send(TransportEvent::Disconnected {
                    peer: state.local_peer,
                    reason: DisconnectReason::Transport(err.to_string()),
                });
            }
        }
    }
}

async fn connect_peer(endpoint: Endpoint, state: WorkerState, peer: PeerId, addr: EndpointAddr) {
    match endpoint.connect(addr, &state.config.alpn).await {
        Ok(connection) => state.insert_connection(peer, connection).await,
        Err(err) => {
            let _ = state.event_tx.send(TransportEvent::Disconnected {
                peer,
                reason: DisconnectReason::Transport(err.to_string()),
            });
        }
    }
}

fn spawn_connection_readers(peer: PeerId, state: WorkerState, connection: Connection) {
    let close_conn = connection.clone();
    let close_state = state.clone();
    let stable_id = connection.stable_id();
    tokio::spawn(async move {
        let _ = close_conn.closed().await;
        close_state.notify_remote_closed(peer, stable_id).await;
    });

    let datagram_conn = connection.clone();
    let datagram_state = state.clone();
    tokio::spawn(async move {
        while let Ok(bytes) = datagram_conn.read_datagram().await {
            if let Some(packet) = decode_transport_packet(peer, datagram_state.local_peer, &bytes) {
                datagram_state
                    .emit_packet_if_current(peer, stable_id, packet)
                    .await;
            }
        }
    });

    let stream_state = state.clone();
    tokio::spawn(async move {
        while let Ok(mut stream) = connection.accept_uni().await {
            match stream
                .read_to_end(stream_state.config.max_packet_bytes)
                .await
            {
                Ok(bytes) => {
                    if let Some(packet) =
                        decode_transport_packet(peer, stream_state.local_peer, &bytes)
                    {
                        stream_state
                            .emit_packet_if_current(peer, stable_id, packet)
                            .await;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests;
