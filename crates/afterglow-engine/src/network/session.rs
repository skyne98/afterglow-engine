use crate::{
    core::identity::StableEntityId,
    network::{NetworkPlayerId, PeerId},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct NetworkSession {
    next_player: u64,
    peers: BTreeMap<PeerId, PeerSession>,
    players: BTreeMap<NetworkPlayerId, PlayerSession>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct PeerSession {
    pub peer: PeerId,
    pub platform: PlatformIdentity,
    pub players: Vec<NetworkPlayerId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct PlayerSession {
    pub player: NetworkPlayerId,
    pub peer: PeerId,
    pub avatar: Option<StableEntityId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub enum PlatformIdentity {
    Local,
    Steam { steam_id: u64 },
    Iroh { node_id: String },
    Anonymous { label: String },
}

impl NetworkSession {
    pub fn connect_peer(&mut self, peer: PeerId, platform: PlatformIdentity) -> bool {
        if self.peers.contains_key(&peer) {
            return false;
        }
        self.peers.insert(
            peer,
            PeerSession {
                peer,
                platform,
                players: Vec::new(),
            },
        );
        true
    }

    pub fn disconnect_peer(&mut self, peer: PeerId) -> Vec<NetworkPlayerId> {
        let Some(session) = self.peers.remove(&peer) else {
            return Vec::new();
        };
        for player in &session.players {
            self.players.remove(player);
        }
        session.players
    }

    pub fn add_player(&mut self, peer: PeerId) -> Option<NetworkPlayerId> {
        let peer_session = self.peers.get_mut(&peer)?;
        self.next_player = self.next_player.saturating_add(1);
        let player = NetworkPlayerId(self.next_player);
        peer_session.players.push(player);
        self.players.insert(
            player,
            PlayerSession {
                player,
                peer,
                avatar: None,
            },
        );
        Some(player)
    }

    pub fn bind_avatar(
        &mut self,
        player: NetworkPlayerId,
        avatar: StableEntityId,
    ) -> Option<StableEntityId> {
        let player = self.players.get_mut(&player)?;
        let previous = player.avatar.replace(avatar);
        Some(previous.unwrap_or(avatar))
    }

    pub fn peer(&self, peer: PeerId) -> Option<&PeerSession> {
        self.peers.get(&peer)
    }

    pub fn peer_for_platform(&self, platform: &PlatformIdentity) -> Option<PeerId> {
        self.peers
            .values()
            .find(|session| &session.platform == platform)
            .map(|session| session.peer)
    }

    pub fn player(&self, player: NetworkPlayerId) -> Option<&PlayerSession> {
        self.players.get(&player)
    }

    pub fn player_for_avatar(&self, avatar: StableEntityId) -> Option<NetworkPlayerId> {
        self.players
            .values()
            .find(|player| player.avatar == Some(avatar))
            .map(|player| player.player)
    }

    pub fn owns_player(&self, peer: PeerId, player: NetworkPlayerId) -> bool {
        self.players
            .get(&player)
            .is_some_and(|player_session| player_session.peer == peer)
    }
}

#[cfg(test)]
mod tests;
