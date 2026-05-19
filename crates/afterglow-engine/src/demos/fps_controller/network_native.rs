use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{
    client::{Connect, NetcodeClient, NetcodeConfig as ClientNetcodeConfig},
    server::{
        ClientOf, NetcodeConfig as ServerNetcodeConfig, NetcodeServer, ServerUdpIo, Start, Started,
    },
    *,
};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

use crate::{
    controller::{FirstPersonMotorState, ReplayCommand},
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::AfterglowLightyearConfig,
};

#[cfg(test)]
use super::network_input::integrate_authoritative_state;
use super::{
    FpsDemoConnectionState, FpsDemoInputCommand, FpsDemoNetworkRuntime, FpsDemoNetworkStatus,
    FpsDemoPlayer, FpsDemoPlayerState, FpsDemoRemoteAvatar, fps_demo_input_command,
    network_input::FpsDemoPredictionBuffer,
    network_protocol::{FpsStateChannel, fps_demo_transport},
};

#[path = "network_native_identity.rs"]
mod identity;
#[cfg(test)]
pub(super) use identity::native_host_player_id;
#[cfg(test)]
pub(super) use identity::native_player_client_id;
pub(super) use identity::native_player_id;
use identity::{FPS_DEMO_PRIVATE_KEY, new_native_client_id, protocol_id};

#[derive(Default)]
pub(super) struct FpsDemoNativeLightyear {
    client: Option<NativeEndpoint>,
    server: Option<NativeEndpoint>,
    server_avatars: BTreeMap<StableEntityId, Entity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeEndpoint {
    entity: Entity,
    addr: String,
    player_id: Option<StableEntityId>,
}

#[derive(Component)]
pub(super) struct FpsDemoNativeClient;

#[derive(Component)]
pub(super) struct FpsDemoNativeServer;

#[derive(Component)]
pub(super) struct FpsDemoNativeServerAvatar;

/// Set on a server avatar after its first input command has been processed
/// by the collision-aware controller. Used to skip corrections until the
/// server has a meaningful authoritative state for this player.
#[derive(Component)]
pub(super) struct FpsServerHasProcessedInput;

pub(super) fn configure_native_server_link(
    trigger: On<Add, LinkOf>,
    mut commands: Commands,
    registry: Res<ChannelRegistry>,
) {
    commands.entity(trigger.entity).insert((
        fps_demo_transport(&registry, false, true),
        MessageManager::default(),
        ReplicationSender::new(Duration::ZERO, SendUpdatesMode::SinceLastAck, false),
        MessageReceiver::<FpsDemoInputCommand>::default(),
    ));
}

pub(super) fn update_native_lightyear(
    mut commands: Commands,
    mut status: ResMut<FpsDemoNetworkStatus>,
    config: Res<AfterglowLightyearConfig>,
    registry: Option<Res<ChannelRegistry>>,
    mut prediction: ResMut<FpsDemoPredictionBuffer>,
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
    players: Query<
        (&FpsDemoPlayerState, Option<&ActionState<AfterglowAction>>),
        With<FpsDemoPlayer>,
    >,
    mut clients: Query<
        (
            Option<&Connected>,
            Option<&Linked>,
            Option<&mut MessageSender<FpsDemoInputCommand>>,
        ),
        With<FpsDemoNativeClient>,
    >,
    servers: Query<(Option<&Started>, Option<&Linked>), With<FpsDemoNativeServer>>,
    server_avatar_states: Query<
        (&FpsDemoPlayerState, &FirstPersonMotorState),
        With<FpsDemoNativeServerAvatar>,
    >,
    mut server_links: Query<
        (Entity, &RemoteId, &mut MessageReceiver<FpsDemoInputCommand>),
        With<ClientOf>,
    >,
) {
    let Some(registry) = registry else {
        return;
    };

    match &status.connection {
        FpsDemoConnectionState::Remote(addr) => {
            drop_native_server(&mut commands, &mut runtime);
            ensure_remote_client(
                &mut commands,
                &mut runtime,
                &config,
                registry.as_ref(),
                addr,
            );
            send_visible_player_input(
                &players,
                &mut clients,
                &runtime,
                &mut prediction,
                status.ticks,
            );
        }
        FpsDemoConnectionState::Server(addr) => {
            ensure_native_server(&mut commands, &mut runtime, &config, addr);
            // The host process also runs a local client that connects to the
            // server so the host player's input is processed and its avatar
            // is replicated to all connected clients.
            ensure_remote_client(
                &mut commands,
                &mut runtime,
                &config,
                registry.as_ref(),
                &loopback_addr(addr),
            );
            send_visible_player_input(
                &players,
                &mut clients,
                &runtime,
                &mut prediction,
                status.ticks,
            );
        }
        _ => {
            drop_native_client(&mut commands, &mut runtime);
            drop_native_server(&mut commands, &mut runtime);
        }
    }

    let _native_server_links = apply_native_server_inputs(
        &mut commands,
        &mut runtime,
        &server_avatar_states,
        &mut server_links,
    );
    refresh_native_status(&mut status, &runtime, &mut clients, &servers);
}

fn loopback_addr(addr: &str) -> String {
    // Replace bind-all (0.0.0.0) with loopback so the local client
    // can connect to the server it just started.
    if let Some(rest) = addr.strip_prefix("0.0.0.0:") {
        format!("127.0.0.1:{rest}")
    } else if addr == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        addr.to_string()
    }
}

fn ensure_remote_client(
    commands: &mut Commands,
    runtime: &mut FpsDemoNetworkRuntime,
    config: &AfterglowLightyearConfig,
    registry: &ChannelRegistry,
    addr: &str,
) {
    if runtime
        .native
        .client
        .as_ref()
        .is_some_and(|client| client.addr == addr)
    {
        return;
    }
    drop_native_client(commands, runtime);
    let Ok(server_addr) = addr.parse::<SocketAddr>() else {
        return;
    };
    let client_id = new_native_client_id();
    let player_id = native_player_id(client_id);
    let auth = Authentication::Manual {
        server_addr,
        client_id,
        private_key: FPS_DEMO_PRIVATE_KEY,
        protocol_id: protocol_id(config),
    };
    let Ok(netcode_client) = NetcodeClient::new(auth, ClientNetcodeConfig::default()) else {
        return;
    };
    let entity = commands
        .spawn((
            FpsDemoNativeClient,
            netcode_client,
            UdpIo::default(),
            LocalAddr(SocketAddr::from(([0, 0, 0, 0], 0))),
            fps_demo_transport(registry, true, false),
            MessageManager::default(),
            ReplicationReceiver::default(),
            PredictionManager::default(),
            MessageSender::<FpsDemoInputCommand>::default(),
        ))
        .id();
    commands.trigger(Connect { entity });
    runtime.native.client = Some(NativeEndpoint {
        entity,
        addr: addr.to_string(),
        player_id: Some(player_id),
    });
}

fn ensure_native_server(
    commands: &mut Commands,
    runtime: &mut FpsDemoNetworkRuntime,
    config: &AfterglowLightyearConfig,
    addr: &str,
) {
    if runtime
        .native
        .server
        .as_ref()
        .is_some_and(|server| server.addr == addr)
    {
        return;
    }
    drop_native_server(commands, runtime);
    let Ok(server_addr) = addr.parse::<SocketAddr>() else {
        return;
    };
    let netcode_server = NetcodeServer::new(
        ServerNetcodeConfig::default()
            .with_protocol_id(protocol_id(config))
            .with_key(FPS_DEMO_PRIVATE_KEY),
    );
    let entity = commands
        .spawn((
            FpsDemoNativeServer,
            Server::default(),
            netcode_server,
            ServerUdpIo::default(),
            LocalAddr(server_addr),
            MessageManager::default(),
        ))
        .id();
    commands.trigger(Start { entity });
    runtime.native.server = Some(NativeEndpoint {
        entity,
        addr: addr.to_string(),
        player_id: None,
    });
}

fn drop_native_client(commands: &mut Commands, runtime: &mut FpsDemoNetworkRuntime) {
    if let Some(client) = runtime.native.client.take() {
        commands.entity(client.entity).try_despawn();
    }
}

fn drop_native_server(commands: &mut Commands, runtime: &mut FpsDemoNetworkRuntime) {
    if let Some(server) = runtime.native.server.take() {
        commands.entity(server.entity).try_despawn();
    }
    for (_, entity) in std::mem::take(&mut runtime.native.server_avatars) {
        commands.entity(entity).try_despawn();
    }
}

fn send_visible_player_input(
    players: &Query<
        (&FpsDemoPlayerState, Option<&ActionState<AfterglowAction>>),
        With<FpsDemoPlayer>,
    >,
    clients: &mut Query<
        (
            Option<&Connected>,
            Option<&Linked>,
            Option<&mut MessageSender<FpsDemoInputCommand>>,
        ),
        With<FpsDemoNativeClient>,
    >,
    runtime: &FpsDemoNetworkRuntime,
    prediction: &mut FpsDemoPredictionBuffer,
    tick: u32,
) {
    let Some(client) = runtime.native.client.as_ref() else {
        return;
    };
    let Some(player_id) = client.player_id else {
        return;
    };
    let Ok((_state, action_state)) = players.single() else {
        return;
    };
    let Ok((connected, linked, sender)) = clients.get_mut(client.entity) else {
        return;
    };
    if connected.is_none() || linked.is_none() {
        return;
    }
    if let Some(mut sender) = sender {
        let command = fps_demo_input_command(player_id, tick, action_state);
        prediction.push(command.clone());
        sender.send::<FpsStateChannel>(command);
    }
}

fn apply_native_server_inputs(
    commands: &mut Commands,
    runtime: &mut FpsDemoNetworkRuntime,
    _avatar_states: &Query<
        (&FpsDemoPlayerState, &FirstPersonMotorState),
        With<FpsDemoNativeServerAvatar>,
    >,
    server_links: &mut Query<
        (Entity, &RemoteId, &mut MessageReceiver<FpsDemoInputCommand>),
        With<ClientOf>,
    >,
) {
    let mut updates = Vec::new();
    for (_, remote_id, mut receiver) in server_links.iter_mut() {
        let PeerId::Netcode(client_id) = remote_id.0 else {
            continue;
        };
        let player_id = native_player_id(client_id);
        updates.extend(
            receiver
                .receive()
                .into_iter()
                .map(|update| (player_id, update)),
        );
    }
    for (player_id, update) in updates {
        let initial_state = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
        let initial_motor = super::network_input::motor_from_player_state(&initial_state);
        ensure_native_server_avatar(commands, runtime, player_id, initial_state, initial_motor);
        if let Some(avatar) = runtime.native.server_avatars.get(&player_id).copied() {
            let command_state = super::network_input::first_person_command_state(&update);
            commands.entity(avatar).insert(ReplayCommand(command_state));
            // Mark that this avatar has received its first input so the
            // client-side correction system knows the authoritative state
            // is meaningful (not just the initial spawn position).
            commands.entity(avatar).insert(FpsServerHasProcessedInput);
        }
    }
}

#[cfg(test)]
pub(super) fn native_authoritative_avatar_state(
    current: (FpsDemoPlayerState, FirstPersonMotorState),
    update: &FpsDemoInputCommand,
) -> (FpsDemoPlayerState, FirstPersonMotorState) {
    integrate_authoritative_state(current.0, current.1, update)
}

fn ensure_native_server_avatar(
    commands: &mut Commands,
    runtime: &mut FpsDemoNetworkRuntime,
    stable_id: StableEntityId,
    state: FpsDemoPlayerState,
    motor: FirstPersonMotorState,
) {
    if runtime.native.server_avatars.contains_key(&stable_id) {
        return;
    }
    let translation = state.to_translation();
    let entity = commands
        .spawn((
            FpsDemoNativeServerAvatar,
            FpsDemoRemoteAvatar { stable_id },
            stable_id,
            state,
            motor,
            crate::controller::FirstPersonController::new(),
            crate::physics::PhysicsBody::kinematic(),
            crate::physics::PhysicsCollider::cylinder(
                crate::controller::FirstPersonControllerConfig::default().body_radius,
                crate::controller::FirstPersonControllerConfig::default().standing_height,
            ),
            Transform::from_translation(translation),
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();
    runtime.native.server_avatars.insert(stable_id, entity);
}

fn refresh_native_status(
    status: &mut FpsDemoNetworkStatus,
    runtime: &FpsDemoNetworkRuntime,
    clients: &mut Query<
        (
            Option<&Connected>,
            Option<&Linked>,
            Option<&mut MessageSender<FpsDemoInputCommand>>,
        ),
        With<FpsDemoNativeClient>,
    >,
    servers: &Query<(Option<&Started>, Option<&Linked>), With<FpsDemoNativeServer>>,
) {
    match &status.connection {
        FpsDemoConnectionState::Remote(_) => {
            let Some(client) = runtime.native.client.as_ref() else {
                return;
            };
            let Ok((connected, linked, _)) = clients.get_mut(client.entity) else {
                status.lightyear_links = false;
                return;
            };
            status.lightyear_links = connected.is_some() && linked.is_some();
        }
        FpsDemoConnectionState::Server(_) => {
            let Some(server) = runtime.native.server.as_ref() else {
                return;
            };
            let Ok((started, linked)) = servers.get(server.entity) else {
                status.lightyear_links = false;
                return;
            };
            status.lightyear_links = started.is_some() && linked.is_some();
        }
        _ => {}
    }
}

impl FpsDemoNativeLightyear {
    pub(super) fn local_player_id(&self) -> Option<StableEntityId> {
        self.client.as_ref()?.player_id
    }
}
