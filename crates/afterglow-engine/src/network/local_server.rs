use crate::{
    input::{LocalPlayers, PlayerCommandQueue},
    network::{
        PeerId,
        authority::ServerCommandBuffer,
        session::{NetworkSession, PlatformIdentity},
    },
};
use bevy::prelude::*;

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct LocalServerConfig {
    pub enabled: bool,
    pub peer: PeerId,
    pub platform: PlatformIdentity,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct LocalServerState {
    owned_peer: Option<PeerId>,
}

impl LocalServerConfig {
    pub fn single_player() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

impl Default for LocalServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            peer: PeerId(0),
            platform: PlatformIdentity::Local,
        }
    }
}

pub fn sync_local_server_session(
    config: Res<LocalServerConfig>,
    local_players: Option<ResMut<LocalPlayers>>,
    mut state: ResMut<LocalServerState>,
    mut session: ResMut<NetworkSession>,
) {
    let Some(mut local_players) = local_players else {
        return;
    };
    if !config.enabled {
        clear_owned_local_peer(&mut local_players, &mut state, &mut session);
        return;
    }
    if state.owned_peer.is_some_and(|peer| peer != config.peer) {
        clear_owned_local_peer(&mut local_players, &mut state, &mut session);
    }
    local_players.peer = Some(config.peer);
    if session.peer(config.peer).is_none() {
        session.connect_peer(config.peer, config.platform.clone());
    }
    state.owned_peer = Some(config.peer);

    for player in local_players.players() {
        if session.player(*player).is_none() {
            session.add_player_with_id(config.peer, *player);
        }
    }

    let owned_players = session
        .peer(config.peer)
        .map(|peer| peer.players.clone())
        .unwrap_or_default();
    for player in owned_players {
        if !local_players.players().contains(&player) {
            session.remove_player(player);
        }
    }
}

fn clear_owned_local_peer(
    local_players: &mut LocalPlayers,
    state: &mut LocalServerState,
    session: &mut NetworkSession,
) {
    let Some(peer) = state.owned_peer.take() else {
        return;
    };
    if local_players.peer == Some(peer) {
        local_players.peer = None;
    }
    session.disconnect_peer(peer);
}

pub fn submit_local_player_commands(
    config: Res<LocalServerConfig>,
    local_players: Option<Res<LocalPlayers>>,
    commands: Option<Res<PlayerCommandQueue>>,
    session: Res<NetworkSession>,
    mut authority: ResMut<ServerCommandBuffer>,
) {
    if !config.enabled {
        return;
    }
    let (Some(local_players), Some(commands)) = (local_players, commands) else {
        return;
    };
    let peer = local_players.peer.unwrap_or(config.peer);
    for command in commands.commands() {
        authority.submit(peer, command.clone(), &session);
    }
}

#[cfg(test)]
mod tests;
