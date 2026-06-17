use super::{LightyearTestRig, TransportConfig};
use afterglow_engine::network::{AfterglowLightyearConfig, LightyearRole};
use bevy::prelude::*;
use lightyear::{
    crossbeam::CrossbeamIo,
    prelude::{
        server::{ClientOf, Started},
        *,
    },
};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Mutex,
    time::Duration,
};

const PROTOCOL_ID: u64 = 0x1234_5678_9ABC_DEF0;
const PRIVATE_KEY: [u8; 32] = [42; 32];
const FIRST_DYNAMIC_UDP_PORT: u16 = 40_000;
const LAST_DYNAMIC_UDP_PORT: u16 = 60_999;
static NEXT_DYNAMIC_UDP_PORT: Mutex<u16> = Mutex::new(FIRST_DYNAMIC_UDP_PORT);

impl LightyearTestRig {
    /// Create a new test rig with Crossbeam transport (in-memory).
    ///
    /// - `client_count`: how many simulated clients (must be >= 1)
    /// - `plugins`: called on each app before Lightyear plugins are added (for
    ///   injecting engine plugins like AfterglowFirstPersonControllerPlugin)
    /// - `register_protocol`: called on each app with its role. Register
    ///   channels, messages, and components here. The callback runs after
    ///   Lightyear plugins but before `app.finish()`, so component registration
    ///   (`register_component`) and message registration (`register_message`)
    ///   work correctly.
    pub fn new(
        client_count: usize,
        plugins: impl Fn(&mut App),
        register_protocol: impl Fn(&mut App, LightyearRole),
    ) -> Self {
        Self::new_with_transport(
            client_count,
            plugins,
            register_protocol,
            TransportConfig::Crossbeam,
        )
    }

    /// Create a test rig with an explicit transport selection.
    pub fn new_with_transport(
        client_count: usize,
        plugins: impl Fn(&mut App),
        register_protocol: impl Fn(&mut App, LightyearRole),
        transport: TransportConfig,
    ) -> Self {
        assert!(
            client_count > 0,
            "LightyearTestRig needs at least one client"
        );
        match transport {
            TransportConfig::Crossbeam => {
                Self::new_crossbeam(client_count, plugins, register_protocol)
            }
            TransportConfig::Udp { server_port } => {
                Self::new_udp(client_count, plugins, register_protocol, server_port)
            }
        }
    }

    fn new_crossbeam(
        client_count: usize,
        plugins: impl Fn(&mut App),
        register_protocol: impl Fn(&mut App, LightyearRole),
    ) -> Self {
        let mut server_app = lightyear_app(LightyearRole::Server, &plugins, &register_protocol);
        let mut client_apps: Vec<App> = (0..client_count)
            .map(|_| lightyear_app(LightyearRole::Client, &plugins, &register_protocol))
            .collect();

        let server_id = server_app
            .world_mut()
            .spawn((Server::default(), Started))
            .id();
        server_app.update();

        let mut client_links = Vec::new();
        let mut server_links = Vec::new();

        for (i, client_app) in client_apps.iter_mut().enumerate() {
            let peer_id = PeerId::Local(i as u64 + 1);
            let (client_io, server_io) = CrossbeamIo::new_pair();

            let client_link = spawn_client_link(client_app, peer_id, client_io);
            let server_link = spawn_server_link(&mut server_app, server_id, peer_id, server_io);

            client_app.update();
            client_app.update();
            server_app.update();

            client_links.push(client_link);
            server_links.push(server_link);
        }

        Self {
            server_app,
            client_apps,
            server_links,
            client_links,
            current_tick: 0,
            entity_map: HashMap::new(),
            input_delay_ticks: 0,
            tick_rate: 60,
            retention_window_ticks: 0,
            pending_inputs: Vec::new(),
        }
    }

    fn new_udp(
        client_count: usize,
        plugins: impl Fn(&mut App),
        register_protocol: impl Fn(&mut App, LightyearRole),
        server_port: u16,
    ) -> Self {
        let actual_port = if server_port == 0 {
            allocate_dynamic_udp_port()
        } else {
            server_port
        };

        let mut server_app = lightyear_app_udp(LightyearRole::Server, &plugins, &register_protocol);
        let mut client_apps: Vec<App> = (0..client_count)
            .map(|_| lightyear_app_udp(LightyearRole::Client, &plugins, &register_protocol))
            .collect();

        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), actual_port);
        server_app.world_mut().spawn((
            server::NetcodeServer::new(
                server::NetcodeConfig::default()
                    .with_protocol_id(PROTOCOL_ID)
                    .with_key(PRIVATE_KEY),
            ),
            server::ServerUdpIo::default(),
            LocalAddr(bind_addr),
        ));
        server_app.update();

        let mut client_links = Vec::new();
        for i in 0..client_count {
            let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), actual_port);
            let transport = client_transport(&client_apps[i]);
            let client_entity = client_apps[i]
                .world_mut()
                .spawn((
                    Client::default(),
                    client::NetcodeClient::new(
                        Authentication::Manual {
                            server_addr,
                            client_id: i as u64 + 1,
                            private_key: PRIVATE_KEY,
                            protocol_id: PROTOCOL_ID,
                        },
                        client::NetcodeConfig::default(),
                    )
                    .expect("failed to create NetcodeClient"),
                    UdpIo::default(),
                    LocalAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)),
                    MessageManager::default(),
                    ReplicationReceiver::default(),
                    PredictionManager::default(),
                    transport,
                ))
                .id();
            client_links.push(client_entity);
        }

        Self {
            server_app,
            client_apps,
            server_links: Vec::new(),
            client_links,
            current_tick: 0,
            entity_map: HashMap::new(),
            input_delay_ticks: 0,
            tick_rate: 60,
            retention_window_ticks: 0,
            pending_inputs: Vec::new(),
        }
    }
}

fn allocate_dynamic_udp_port() -> u16 {
    let mut next = NEXT_DYNAMIC_UDP_PORT
        .lock()
        .expect("UDP port allocator should not be poisoned");

    let range_len = LAST_DYNAMIC_UDP_PORT - FIRST_DYNAMIC_UDP_PORT + 1;

    // Apply a process-specific offset on first call to spread parallel test
    // processes across the port range instead of all starting at 40_000.
    // This reduces the risk of cross-process port collisions during
    // concurrent `cargo test` invocations without adding a true lock.
    if *next == FIRST_DYNAMIC_UDP_PORT {
        *next = FIRST_DYNAMIC_UDP_PORT + (std::process::id() as u16) % range_len;
    }

    for _ in FIRST_DYNAMIC_UDP_PORT..=LAST_DYNAMIC_UDP_PORT {
        let candidate = *next;
        *next = if candidate >= LAST_DYNAMIC_UDP_PORT {
            FIRST_DYNAMIC_UDP_PORT
        } else {
            candidate + 1
        };

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), candidate);
        if std::net::UdpSocket::bind(addr).is_ok() {
            return candidate;
        }
    }
    panic!(
        "no available UDP port in dynamic test range {FIRST_DYNAMIC_UDP_PORT}-{LAST_DYNAMIC_UDP_PORT}"
    );
}

fn lightyear_app(
    role: LightyearRole,
    plugins: impl Fn(&mut App),
    register_protocol: impl Fn(&mut App, LightyearRole),
) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(afterglow_engine::core::AfterglowCorePlugin);
    app.insert_resource(AfterglowLightyearConfig {
        role,
        ..Default::default()
    });
    plugins(&mut app);
    add_crossbeam_lightyear_plugins(&mut app, role);
    app.init_resource::<PeerMetadata>();
    register_protocol(&mut app, role);
    app.finish();
    app.cleanup();
    app
}

fn lightyear_app_udp(
    role: LightyearRole,
    plugins: impl Fn(&mut App),
    register_protocol: impl Fn(&mut App, LightyearRole),
) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(afterglow_engine::core::AfterglowCorePlugin);
    app.insert_resource(AfterglowLightyearConfig {
        role,
        ..Default::default()
    });
    plugins(&mut app);
    let tick_duration = Duration::from_secs_f64(1.0 / 60.0);
    match role {
        LightyearRole::Client => {
            app.add_plugins(lightyear::prelude::client::ClientPlugins { tick_duration });
        }
        LightyearRole::Server => {
            app.add_plugins(lightyear::prelude::server::ServerPlugins { tick_duration });
            app.add_observer(add_udp_replication_sender);
        }
        LightyearRole::Host => {
            app.add_plugins((
                lightyear::prelude::server::ServerPlugins { tick_duration },
                lightyear::prelude::client::ClientPlugins { tick_duration },
            ));
            app.add_observer(add_udp_replication_sender);
        }
    }
    app.init_resource::<PeerMetadata>();
    register_protocol(&mut app, role);
    app.finish();
    app.cleanup();
    app
}

fn add_crossbeam_lightyear_plugins(app: &mut App, role: LightyearRole) {
    let tick_duration = Duration::from_secs_f64(1.0 / 60.0);
    match role {
        LightyearRole::Client => {
            app.add_plugins(
                lightyear::prelude::client::ClientPlugins { tick_duration }
                    .build()
                    .disable::<lightyear::prelude::client::NetcodeClientPlugin>(),
            );
        }
        LightyearRole::Server => {
            app.add_plugins(
                lightyear::prelude::server::ServerPlugins { tick_duration }
                    .build()
                    .disable::<lightyear::prelude::server::NetcodeServerPlugin>(),
            );
        }
        LightyearRole::Host => {
            app.add_plugins((
                lightyear::prelude::server::ServerPlugins { tick_duration }
                    .build()
                    .disable::<lightyear::prelude::server::NetcodeServerPlugin>(),
                lightyear::prelude::client::ClientPlugins { tick_duration }
                    .build()
                    .disable::<lightyear::prelude::client::NetcodeClientPlugin>(),
            ));
        }
    }
}

fn spawn_client_link(app: &mut App, local_id: PeerId, io: CrossbeamIo) -> Entity {
    let transport = client_transport(app);
    app.world_mut()
        .spawn((
            Client::default(),
            LocalId(local_id),
            RemoteId(PeerId::Server),
            Connected,
            Link::default(),
            Linked,
            io,
            transport,
            MessageManager::default(),
            ReplicationReceiver::default(),
            PredictionManager::default(),
        ))
        .id()
}

fn spawn_server_link(app: &mut App, server: Entity, remote_id: PeerId, io: CrossbeamIo) -> Entity {
    let transport = server_transport(app);
    app.world_mut()
        .spawn((
            LinkOf { server },
            ClientOf,
            LocalId(PeerId::Server),
            RemoteId(remote_id),
            Connected,
            Link::default(),
            Linked,
            io,
            transport,
            MessageManager::default(),
            ReplicationSender::new(Duration::ZERO, SendUpdatesMode::SinceLastAck, false),
        ))
        .id()
}

fn add_udp_replication_sender(
    trigger: On<Add, LinkOf>,
    mut commands: Commands,
    client_links: Query<(), With<Client>>,
    existing_senders: Query<(), With<ReplicationSender>>,
) {
    if client_links.get(trigger.entity).is_ok() || existing_senders.get(trigger.entity).is_ok() {
        return;
    }

    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::new(
            Duration::ZERO,
            SendUpdatesMode::SinceLastAck,
            false,
        ));
}

fn client_transport(app: &App) -> Transport {
    let registry = app.world().resource::<ChannelRegistry>();
    let mut transport = Transport::default();
    transport.add_sender_from_registry::<MetadataChannel>(registry);
    transport.add_receiver_from_registry::<MetadataChannel>(registry);
    transport.add_sender_from_registry::<UpdatesChannel>(registry);
    transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    add_input_channel_if_registered(&mut transport, registry);
    transport
}

fn server_transport(app: &App) -> Transport {
    let registry = app.world().resource::<ChannelRegistry>();
    let mut transport = Transport::default();
    transport.add_sender_from_registry::<MetadataChannel>(registry);
    transport.add_receiver_from_registry::<MetadataChannel>(registry);
    transport.add_sender_from_registry::<UpdatesChannel>(registry);
    transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    add_input_channel_if_registered(&mut transport, registry);
    transport
}

fn add_input_channel_if_registered(transport: &mut Transport, registry: &ChannelRegistry) {
    use lightyear_transport::channel::registry::ChannelKind;
    if registry
        .settings(ChannelKind::of::<lightyear::input::InputChannel>())
        .is_some()
    {
        transport.add_sender_from_registry::<lightyear::input::InputChannel>(registry);
        transport.add_receiver_from_registry::<lightyear::input::InputChannel>(registry);
    }
}
