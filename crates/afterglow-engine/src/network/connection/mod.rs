//! Engine-level connection plugin and auth / controlled-entity infrastructure.
//!
//! Provides [`AfterglowConnectionPlugin`] (server/client variants), type
//! aliases, resources, and [`ConnectionEvent`]s that games listen on to
//! spawn/despawn player entities.
//!
//! # Design
//!
//! - Server variant spawns a `NetcodeServer` entity, registers `On<Add,
//!   ClientOf>` observer for synchronous link setup + auth challenge.
//! - Client variant spawns a `NetcodeClient` entity, registers `On<Add,
//!   Connected>` observer for input-timeline configuration.
//! - Challenge-response authentication uses Lightyear messages over the
//!   `ActionsChannel`.

pub mod auth;
pub mod controlled;
pub mod link;
pub(crate) mod readiness;

#[cfg(all(test, feature = "lightyear"))]
mod auth_tests;

pub use auth::{AuthResponse, ChallengeMessage, LocalIdentity};
pub use controlled::{
    ControlledEntityPlugin, MemberLinkMap, PlayerOwned, bind_controlled_entities,
};

use bevy::prelude::*;

pub use crate::network::PlayerId;
use crate::network::lightyear::LightyearLinkConditioner;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Connection lifecycle event.
///
/// Games observe this event to spawn player entities and clean them up on
/// disconnect. The event is triggered via
/// `commands.trigger(ConnectionEvent{..})`. Listen by adding an observer:
/// `app.add_observer(on_connection_event)`.
#[derive(Event, Clone, Debug)]
pub struct ConnectionEvent {
    pub kind: ConnectionEventKind,
    pub player_id: PlayerId,
    pub link_entity: Entity,
}

#[derive(Clone, Debug)]
pub enum ConnectionEventKind {
    Connected,
    Disconnected { reason: String },
}

/// Runtime connection configuration.
#[derive(Resource, Clone, Debug)]
pub struct ConnectionConfig {
    pub tick_rate: u64,
    pub input_delay_ticks: u16,
    pub rebroadcast_inputs: bool,
    pub link_conditioner: Option<LightyearLinkConditioner>,
    /// If `true`, the server challenges each connecting client with a
    /// nonce that must be signed with the client's Ed25519 private key.
    /// If `false`, `ConnectionEvent::Connected` is emitted as soon as the
    /// netcode handshake completes (safe for tests and Steam).
    pub require_auth: bool,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            tick_rate: 60,
            input_delay_ticks: 2,
            rebroadcast_inputs: true,
            link_conditioner: None,
            require_auth: true,
        }
    }
}

/// Netcode configuration (protocol_id + private_key).
///
/// Passed to [`AfterglowConnectionPlugin::server`].
pub struct NetcodeConfig {
    pub protocol_id: u64,
    pub private_key: [u8; 32],
}

/// Server bind address.
#[derive(Resource, Clone, Debug)]
pub struct ServerListenAddr(pub std::net::SocketAddr);

/// Client target address.
#[derive(Resource, Clone, Debug)]
pub struct ServerAddr(pub std::net::SocketAddr);

/// The local player's identifier (derived from [`LocalIdentity`]).
#[derive(Resource, Clone, Debug)]
pub struct LocalPlayerId(pub PlayerId);

// ---------------------------------------------------------------------------
// Internal resources
// ---------------------------------------------------------------------------

/// Stored by the server variant of [`AfterglowConnectionPlugin`] and consumed
/// by the spawn system.
#[derive(Resource)]
pub(crate) struct ServerNetcodeConfig {
    pub protocol_id: u64,
    pub private_key: [u8; 32],
}

#[derive(Resource)]
pub(crate) struct ClientNetcodeConfig {
    pub protocol_id: u64,
    pub private_key: [u8; 32],
}

/// Marker: the server entity has been spawned.
#[derive(Component)]
pub struct ServerSpawned;

/// Marker: the client entity has been spawned.
///
/// Query for entities with this component to find the local client
/// link entity for PreSpawned prediction.
#[derive(Component)]
pub struct ClientSpawned;

// ---------------------------------------------------------------------------
// AfterglowConnectionPlugin
// ---------------------------------------------------------------------------

enum ConnectionMode {
    Server(NetcodeConfig),
    Client(NetcodeConfig),
}

/// Manages a single Lightyear netcode connection, either as server or client.
///
/// **Server variant** (`::server(netcode_config)`):
/// - Spawns a `NetcodeServer` entity bound to [`ServerListenAddr`] on startup.
/// - On each new client connection (`On<Add, ClientOf>`), inserts
///   `ReplicationSender`, adds transport channels, populates [`MemberLinkMap`],
///   and either sends a challenge (if `require_auth`) or emits
///   [`ConnectionEvent::Connected`].
///
/// **Client variant** (`::client()`):
/// - Reads [`ServerAddr`] + [`LocalIdentity`] and spawns a `NetcodeClient` on
///   startup.
/// - On `On<Add, Connected>`, inserts input timeline components for input
///   delay.
/// - On receiving a `ChallengeMessage`, signs the nonce and sends
///   [`AuthResponse`].
pub struct AfterglowConnectionPlugin {
    mode: ConnectionMode,
}

impl AfterglowConnectionPlugin {
    pub fn server(netcode_config: NetcodeConfig) -> Self {
        Self {
            mode: ConnectionMode::Server(netcode_config),
        }
    }

    pub fn client(netcode_config: NetcodeConfig) -> Self {
        Self {
            mode: ConnectionMode::Client(netcode_config),
        }
    }
}

impl Plugin for AfterglowConnectionPlugin {
    fn build(&self, app: &mut App) {
        use lightyear::prelude::*;

        app.init_resource::<ConnectionConfig>()
            .init_resource::<MemberLinkMap>();

        match &self.mode {
            ConnectionMode::Server(cfg) => {
                app.insert_resource(ServerNetcodeConfig {
                    protocol_id: cfg.protocol_id,
                    private_key: cfg.private_key,
                });

                // Register auth messages on ActionsChannel.
                app.register_message::<auth::ChallengeMessage>()
                    .add_direction(NetworkDirection::ServerToClient);
                app.register_message::<auth::AuthResponse>()
                    .add_direction(NetworkDirection::ClientToServer);

                readiness::register_clientof_required_components(app);

                // Synchronous observers for connection lifecycle.
                app.add_observer(auth::on_client_of_added);
                app.add_observer(auth::on_client_disconnected);

                // Auth-response receiver (runs after observer flush).
                app.add_systems(PreUpdate, auth::receive_auth_response);
                app.add_systems(
                    PostUpdate,
                    readiness::ensure_client_replication_senders_ready
                        .before(lightyear::prelude::ReplicationBufferSystems::BeforeBuffer),
                );

                // Controlled entity binding.
                app.add_plugins(ControlledEntityPlugin);

                // Spawn the NetcodeServer (one-shot on startup).
                app.add_systems(Startup, spawn_netcode_server);
            }
            ConnectionMode::Client(netcode_config) => {
                app.insert_resource(ClientNetcodeConfig {
                    protocol_id: netcode_config.protocol_id,
                    private_key: netcode_config.private_key,
                });

                // Register auth messages on ActionsChannel.
                app.register_message::<auth::ChallengeMessage>()
                    .add_direction(NetworkDirection::ServerToClient);
                app.register_message::<auth::AuthResponse>()
                    .add_direction(NetworkDirection::ClientToServer);

                // On netcode connect, configure input delay.
                app.add_observer(link::on_client_connected);

                // Challenge-response receiver.
                app.add_systems(PreUpdate, auth::receive_challenge);

                // Spawn the NetcodeClient (one-shot on startup).
                app.add_systems(Startup, spawn_netcode_client);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Server spawn
// ---------------------------------------------------------------------------

/// Spawns the `NetcodeServer` entity and triggers it to start listening.
fn spawn_netcode_server(
    mut commands: Commands,
    listen_addr: Option<Res<ServerListenAddr>>,
    server_cfg: Option<Res<ServerNetcodeConfig>>,
    server_entities: Query<(), With<ServerSpawned>>,
) {
    if !server_entities.is_empty() {
        return;
    }
    let Some(addr) = listen_addr else {
        return;
    };
    let Some(cfg) = server_cfg else {
        return;
    };

    let config = lightyear::prelude::server::NetcodeConfig {
        protocol_id: cfg.protocol_id,
        private_key: cfg.private_key,
        ..Default::default()
    };
    let server = lightyear::prelude::server::NetcodeServer::new(config);

    let entity = commands
        .spawn((
            lightyear::prelude::server::Server::default(),
            server,
            lightyear::prelude::server::ServerUdpIo::default(),
            lightyear::prelude::LocalAddr(addr.0),
            lightyear::prelude::Link::default(),
            // MessageManager is registered as a required component of
            // ClientOf by the MessagePlugin. The Server entity itself
            // also needs it for its own message handling.
            lightyear::prelude::MessageManager::default(),
            ServerSpawned,
        ))
        .id();

    // Deferred trigger: commands flush spawns the entity first, then
    // this trigger fires the Start observer on it.
    commands
        .entity(entity)
        .trigger(|e| lightyear::prelude::server::Start { entity: e });
}

// ---------------------------------------------------------------------------
// Client spawn
// ---------------------------------------------------------------------------

/// Spawns the `NetcodeClient` entity and triggers it to connect.
fn spawn_netcode_client(
    mut commands: Commands,
    server_addr: Option<Res<ServerAddr>>,
    identity: Option<Res<LocalIdentity>>,
    netcode_config: Option<Res<ClientNetcodeConfig>>,
    channel_registry: Option<Res<lightyear::prelude::ChannelRegistry>>,
    client_entities: Query<(), With<ClientSpawned>>,
) {
    if !client_entities.is_empty() {
        return;
    }
    let Some(addr) = server_addr else {
        return;
    };
    let Some(identity) = identity else {
        return;
    };
    let Some(netcode_config) = netcode_config else {
        return;
    };

    let auth = lightyear::prelude::Authentication::Manual {
        server_addr: addr.0,
        client_id: identity.player_id,
        private_key: netcode_config.private_key,
        protocol_id: netcode_config.protocol_id,
    };

    let config = lightyear::prelude::client::NetcodeConfig::default();

    commands.insert_resource(LocalPlayerId(identity.player_id));

    match lightyear::prelude::client::NetcodeClient::new(auth, config) {
        Ok(client) => {
            let Some(registry) = channel_registry.as_deref() else {
                bevy::log::error!(
                    "ChannelRegistry missing while spawning NetcodeClient; check plugin ordering before starting netcode"
                );
                return;
            };
            let mut transport = lightyear::prelude::Transport::default();
            readiness::add_channels_to_transport(&mut transport, registry);

            // Bind the client UDP socket to an OS-selected local port. Do not
            // insert MessageManager explicitly here: Lightyear registers it as
            // a required component for `Client`, and overwriting it after the
            // message sender/receiver hooks run drops Lightyear's metadata.
            let local_addr = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
            let entity = commands
                .spawn((
                    lightyear::prelude::client::Client::default(),
                    lightyear::prelude::Link::default(),
                ))
                .insert(client)
                .insert(lightyear::prelude::UdpIo::default())
                .insert(lightyear::prelude::LocalAddr(local_addr))
                .insert(lightyear::prelude::ReplicationReceiver::default())
                .insert(lightyear::prelude::PredictionManager::default())
                .insert(transport)
                .insert(ClientSpawned)
                .id();
            commands
                .entity(entity)
                .trigger(|e| lightyear::prelude::client::Connect { entity: e });
        }
        Err(e) => {
            bevy::log::error!("Failed to create NetcodeClient: {:?}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "lightyear")]
    #[test]
    fn spawn_client_inserts_local_player_id_resource() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        app.insert_resource(ServerAddr("127.0.0.1:0".parse().unwrap()));
        app.insert_resource(LocalIdentity::unauthenticated(42));
        app.insert_resource(ClientNetcodeConfig {
            protocol_id: 42,
            private_key: [0u8; 32],
        });

        app.add_systems(Startup, spawn_netcode_client);
        app.update();

        assert!(app.world().contains_resource::<LocalPlayerId>());
        let local_id = app.world().resource::<LocalPlayerId>();
        assert_eq!(local_id.0, 42, "LocalPlayerId should be 42");
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn spawn_client_wires_transport_channels_for_replication_and_input() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::network::AfterglowLightyearConfig {
            role: crate::network::LightyearRole::Client,
            ..Default::default()
        });
        app.add_plugins(crate::network::AfterglowLightyearPlugin);

        app.insert_resource(ServerAddr("127.0.0.1:0".parse().unwrap()));
        app.insert_resource(LocalIdentity::unauthenticated(42));
        app.insert_resource(ClientNetcodeConfig {
            protocol_id: 42,
            private_key: [0u8; 32],
        });
        app.add_systems(Startup, spawn_netcode_client);
        app.finish();
        app.cleanup();
        app.update();

        let mut clients = app
            .world_mut()
            .query_filtered::<&lightyear::prelude::Transport, With<ClientSpawned>>();
        let transport = clients
            .single(app.world())
            .expect("spawn_netcode_client should create one client transport");
        assert!(transport.has_sender::<lightyear::prelude::MetadataChannel>());
        assert!(transport.has_receiver::<lightyear::prelude::MetadataChannel>());
        assert!(transport.has_sender::<lightyear::prelude::UpdatesChannel>());
        assert!(transport.has_receiver::<lightyear::prelude::UpdatesChannel>());
        assert!(transport.has_sender::<lightyear::prelude::ActionsChannel>());
        assert!(transport.has_receiver::<lightyear::prelude::ActionsChannel>());
        assert!(transport.has_sender::<lightyear::input::InputChannel>());
        assert!(transport.has_receiver::<lightyear::input::InputChannel>());
    }
}
