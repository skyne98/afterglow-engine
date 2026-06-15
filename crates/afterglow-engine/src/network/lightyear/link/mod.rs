//! Session-to-Lightyear bridge.
//!
//! Provides [`AfterglowSessionLightyearBridgePlugin`] — an opt-in plugin that
//! reads [`SessionEvent`] messages produced by the session layer and maps them
//! to Lightyear link lifecycle.
//!
//! # Transport Mapping
//!
//! | SessionTransport | Bridge Action |
//! |---|---|
//! | `SessionTransport::Local` | Spawn in-process Crossbeam Lightyear link entities for client and server. Requires Lightyear plugins (crossbeam) to be installed. |
//! | `SessionTransport::DirectUdp { host }` | Parse `host` as a [`SocketAddr`]; write [`NetcodeClientParams`] (and [`NetcodeServerParams`] if the configured role is `Host` or `Server`) into [`PendingNetcodeStartup`]. No link entities are spawned — a separate consumer should drain the pending params and establish UDP/netcode links. |
//!
//! # Ordering
//!
//! The bridge runs in [`AfterglowSessionSet::ApplyEffects`].
//! It creates link entities or pending startup params for later systems to
//! consume; Lightyear may observe newly spawned link entities on the following
//! frame depending on its own internal `PreUpdate` ordering.

use std::{net::SocketAddr, time::Duration};

use bevy::prelude::*;
use lightyear::{
    crossbeam::CrossbeamIo,
    prelude::{server::*, *},
};

#[cfg(feature = "lightyear")]
use lightyear::prelude::{
    Authentication as NetcodeAuthentication, LocalAddr, UdpIo,
};
#[cfg(feature = "lightyear")]
use lightyear::prelude::client::{Connect, NetcodeClient, NetcodeConfig as ClientNetcodeConfig};
#[cfg(feature = "lightyear")]
use lightyear::prelude::server::{NetcodeConfig as ServerNetcodeConfig, NetcodeServer, Start};

use crate::network::{
    lightyear::{AfterglowLightyearConfig, LightyearRole},
    session::{AfterglowSessionSet, AfterglowSessionState, SessionEvent, SessionTransport},
};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Tracks in-process Lightyear link entities created for a local session.
///
/// Set when processing a [`SessionEvent::Created`] or [`SessionEvent::Joined`]
/// with [`SessionTransport::Local`]. Cleared on [`SessionEvent::Left`] or
/// [`SessionEvent::SessionEnded`].
#[derive(Resource, Clone, Debug, Default)]
pub struct SessionLightyearLinks {
    /// The client link entity spawned for a local session.
    pub client_link: Option<Entity>,
    /// The server link entity spawned for a local session.
    pub server_link: Option<Entity>,
    /// The server entity (`Server::default()` + `Started`) spawned alongside
    /// the server link. Tracked so it can be despawned on cleanup.
    pub server_entity: Option<Entity>,
}

/// Pending Netcode startup parameters written when a `DirectUdp` session event
/// is processed.
///
/// Consumers (e.g. a netcode-connection system) should drain this resource and
/// establish UDP/netcode links. The bridge does not spawn UDP links directly.
#[derive(Resource, Clone, Debug, Default)]
pub struct PendingNetcodeStartup {
    pub client: Option<NetcodeClientParams>,
    pub server: Option<NetcodeServerParams>,
}

/// Parameters for starting a Netcode client connection.
#[derive(Clone, Debug)]
pub struct NetcodeClientParams {
    pub server_addr: SocketAddr,
    pub client_id: u64,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
}

/// Parameters for starting a Netcode server.
#[derive(Clone, Debug)]
pub struct NetcodeServerParams {
    pub bind_addr: SocketAddr,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
}

// ---------------------------------------------------------------------------
// Transport helper
// ---------------------------------------------------------------------------

fn transport_from_registry(registry: &ChannelRegistry) -> Transport {
    let mut transport = Transport::default();
    transport.add_sender_from_registry::<MetadataChannel>(registry);
    transport.add_receiver_from_registry::<MetadataChannel>(registry);
    transport.add_sender_from_registry::<UpdatesChannel>(registry);
    transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    transport
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Opt-in plugin that maps session events to Lightyear link lifecycle.
///
/// # Initialization
///
/// Initializes [`SessionLightyearLinks`] and [`PendingNetcodeStartup`]
/// resources. Adds its private bridge system to
/// [`AfterglowSessionSet::ApplyEffects`].
///
/// # When to Add
///
/// Add this plugin *after* Lightyear plugins are installed (e.g. via
/// [`AfterglowLightyearPlugin`]) and *after*
/// [`AfterglowSessionPlugin`](crate::network::session::AfterglowSessionPlugin).
/// It is not included in
/// [`AfterglowNetworkPlugin`](crate::network::AfterglowNetworkPlugin).
pub struct AfterglowSessionLightyearBridgePlugin;

impl Plugin for AfterglowSessionLightyearBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SessionLightyearLinks>()
            .init_resource::<PendingNetcodeStartup>()
            .add_systems(
                PreUpdate,
                handle_session_lightyear_links
                    .in_set(AfterglowSessionSet::ApplyEffects)
                    .after(crate::network::session::status::update_session_status),
            );
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_session_lightyear_links(
    mut commands: Commands,
    mut links: ResMut<SessionLightyearLinks>,
    mut pending: ResMut<PendingNetcodeStartup>,
    session_state: Option<Res<AfterglowSessionState>>,
    config: Option<Res<AfterglowLightyearConfig>>,
    registry: Option<Res<ChannelRegistry>>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        match event {
            SessionEvent::Created(info) | SessionEvent::Joined(info) => {
                // Clear stale state from any previous transport.
                if let Some(entity) = links.client_link.take() {
                    commands.entity(entity).despawn();
                }
                if let Some(entity) = links.server_link.take() {
                    commands.entity(entity).despawn();
                }
                if let Some(entity) = links.server_entity.take() {
                    commands.entity(entity).despawn();
                }
                pending.client = None;
                pending.server = None;

                match &info.transport {
                    SessionTransport::Local => {
                        let registry = match registry.as_ref() {
                            Some(r) => r,
                            // ChannelRegistry absent — Lightyear plugins not
                            // installed. Skip spawning to avoid panics.
                            None => continue,
                        };

                        let server_entity = commands.spawn((Server::default(), Started)).id();

                        let (client_io, server_io) = CrossbeamIo::new_pair();
                        let client_transport = transport_from_registry(registry);
                        let server_transport = transport_from_registry(registry);

                        let local_id = PeerId::Local(1);

                        let client_link = commands
                            .spawn((
                                Client::default(),
                                LocalId(local_id),
                                RemoteId(PeerId::Server),
                                Connected,
                                Link::default(),
                                Linked,
                                client_io,
                                client_transport,
                                MessageManager::default(),
                                ReplicationReceiver::default(),
                                PredictionManager::default(),
                            ))
                            .id();

                        let server_link = commands
                            .spawn((
                                LinkOf {
                                    server: server_entity,
                                },
                                // Lightyear marks the server-side link as the
                                // connected client of the server entity.
                                ClientOf,
                                LocalId(PeerId::Server),
                                RemoteId(local_id),
                                Connected,
                                Link::default(),
                                Linked,
                                server_io,
                                server_transport,
                                MessageManager::default(),
                                ReplicationSender::new(
                                    Duration::ZERO,
                                    SendUpdatesMode::SinceLastAck,
                                    false,
                                ),
                            ))
                            .id();

                        links.client_link = Some(client_link);
                        links.server_link = Some(server_link);
                        links.server_entity = Some(server_entity);
                    }
                    SessionTransport::DirectUdp { host } => {
                        if let Ok(server_addr) = host.parse::<SocketAddr>() {
                            let cfg = config.as_deref().cloned().unwrap_or_default();
                            let protocol_id = cfg.protocol_id;
                            let private_key = cfg.netcode_private_key;

                            let client_id = session_state
                                .as_deref()
                                .and_then(|state| {
                                    u64::try_from(state.local_member_id.as_raw()).ok()
                                })
                                .filter(|&id| id != 0);

                            if matches!(cfg.role, LightyearRole::Host | LightyearRole::Client)
                                && let Some(cid) = client_id
                            {
                                pending.client = Some(NetcodeClientParams {
                                    server_addr,
                                    client_id: cid,
                                    protocol_id,
                                    private_key,
                                });
                            }

                            if matches!(cfg.role, LightyearRole::Host | LightyearRole::Server) {
                                pending.server = Some(NetcodeServerParams {
                                    bind_addr: server_addr,
                                    protocol_id,
                                    private_key,
                                });
                            }
                        }
                        // Invalid host: stale state cleared above; no panic,
                        // no error event emitted.
                    }
                }
            }
            SessionEvent::Left { .. } | SessionEvent::SessionEnded(_) => {
                if let Some(entity) = links.client_link.take() {
                    commands.entity(entity).despawn();
                }
                if let Some(entity) = links.server_link.take() {
                    commands.entity(entity).despawn();
                }
                if let Some(entity) = links.server_entity.take() {
                    commands.entity(entity).despawn();
                }
                pending.client = None;
                pending.server = None;
            }
            // Ignored: SearchResults, MemberJoined, MemberLeft, Error
            _ => {}
        }
    }
}

/// Opt-in plugin that drains [`PendingNetcodeStartup`] and spawns real
/// Lightyear netcode link entities with UDP transport.
///
/// Add this after `AfterglowSessionLightyearBridgePlugin` (and after Lightyear
/// client/server plugins) when you want netcode sessions to automatically open
/// UDP sockets. The bridge itself only writes pending params; this consumer is
/// responsible for acting on them.
#[derive(Default)]
pub struct AfterglowNetcodeConsumerPlugin;

impl Plugin for AfterglowNetcodeConsumerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            consume_pending_netcode_startup
                .in_set(AfterglowSessionSet::ApplyEffects)
                .after(handle_session_lightyear_links),
        );
    }
}

/// Drains [`PendingNetcodeStartup`] and spawns real Lightyear netcode link
/// entities with UDP transport.
fn consume_pending_netcode_startup(
    mut commands: Commands,
    mut pending: ResMut<PendingNetcodeStartup>,
    mut links: ResMut<SessionLightyearLinks>,
    registry: Option<Res<ChannelRegistry>>,
) {
    if pending.client.is_none() && pending.server.is_none() {
        return;
    }

    let any_local_client = pending.client.take();
    let any_local_server = pending.server.take();

    if let Some(params) = any_local_client {
        if let Some(entity) = links.client_link.take() {
            commands.entity(entity).despawn();
        }

        let local_addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let auth = NetcodeAuthentication::Manual {
            server_addr: params.server_addr,
            client_id: params.client_id,
            private_key: params.private_key,
            protocol_id: params.protocol_id,
        };

        match NetcodeClient::new(auth, ClientNetcodeConfig::default()) {
            Ok(client) => {
                let registry = registry.as_deref().cloned().unwrap_or_default();
                let mut transport = Transport::default();
                let registry = &registry;
                transport.add_sender_from_registry::<MetadataChannel>(registry);
                transport.add_receiver_from_registry::<MetadataChannel>(registry);
                transport.add_sender_from_registry::<UpdatesChannel>(registry);
                transport.add_receiver_from_registry::<UpdatesChannel>(registry);
                transport.add_sender_from_registry::<ActionsChannel>(registry);
                transport.add_receiver_from_registry::<ActionsChannel>(registry);

                // Per `docs/subject/lightyear.md` § 4.2:
                //   Client entity requires Client, LocalId, RemoteId,
                //   Link, Linked, Transport, MessageManager,
                //   ReplicationReceiver, PredictionManager.
                // `Connected` is added by the NetcodeClientPlugin's
                // observer when the handshake completes.
                let entity = commands
                    .spawn((
                        Client::default(),
                        LocalId(PeerId::Local(1)),
                        RemoteId(PeerId::Server),
                        Link::default(),
                        client,
                        UdpIo::default(),
                        LocalAddr(local_addr),
                        MessageManager::default(),
                        ReplicationReceiver::default(),
                        PredictionManager::default(),
                        transport,
                    ))
                    .id();
                commands.entity(entity).trigger(|e| Connect { entity: e });
                links.client_link = Some(entity);
            }
            Err(e) => {
                bevy::log::warn!("failed to create NetcodeClient: {:?}", e);
            }
        }
    }

    if let Some(params) = any_local_server {
        if let Some(entity) = links.server_link.take() {
            commands.entity(entity).despawn();
        }

        let config = ServerNetcodeConfig {
            protocol_id: params.protocol_id,
            private_key: params.private_key,
            ..Default::default()
        };
        let server = NetcodeServer::new(config);
        // Per `docs/subject/lightyear.md` § 4.2:
        //   Server entity requires Server, Link, Transport, MessageManager.
        // `Started` is added by the NetcodeServerPlugin's observer when the
        // server finishes binding.
        let entity = commands
            .spawn((
                Server::default(),
                server,
                lightyear::prelude::server::ServerUdpIo::default(),
                LocalAddr(params.bind_addr),
                Link::default(),
                MessageManager::default(),
            ))
            .id();
        commands.entity(entity).trigger(|e| Start { entity: e });
        links.server_link = Some(entity);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
