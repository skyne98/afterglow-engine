use crate::network::{
    NetworkPlayerId, PeerId,
    interest::InterestMap,
    replication::{ReplicationWorld, WorldSnapshot},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplicationSaveData {
    pub tick: u32,
    pub snapshot: WorldSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconnectBaseline {
    pub peer: PeerId,
    pub player: NetworkPlayerId,
    pub snapshot: WorldSnapshot,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconnectBaselineStore {
    baselines: BTreeMap<(PeerId, NetworkPlayerId), ReconnectBaseline>,
}

impl ReplicationSaveData {
    pub fn from_world(world: &ReplicationWorld, tick: u32) -> Self {
        Self {
            tick,
            snapshot: world.snapshot(tick),
        }
    }

    pub fn restore_world(&self) -> ReplicationWorld {
        let mut world = ReplicationWorld::default();
        world.apply_snapshot(&self.snapshot);
        world
    }

    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl ReconnectBaseline {
    pub fn from_snapshot(peer: PeerId, player: NetworkPlayerId, snapshot: WorldSnapshot) -> Self {
        Self {
            peer,
            player,
            snapshot,
        }
    }

    pub fn filtered(
        peer: PeerId,
        player: NetworkPlayerId,
        snapshot: &WorldSnapshot,
        interest: &InterestMap,
    ) -> Self {
        Self {
            peer,
            player,
            snapshot: interest.filter_snapshot(player, snapshot),
        }
    }
}

impl ReconnectBaselineStore {
    pub fn insert(&mut self, baseline: ReconnectBaseline) -> Option<ReconnectBaseline> {
        self.baselines
            .insert((baseline.peer, baseline.player), baseline)
    }

    pub fn get(&self, peer: PeerId, player: NetworkPlayerId) -> Option<&ReconnectBaseline> {
        self.baselines.get(&(peer, player))
    }

    pub fn remove(&mut self, peer: PeerId, player: NetworkPlayerId) -> Option<ReconnectBaseline> {
        self.baselines.remove(&(peer, player))
    }

    pub fn clear_peer(&mut self, peer: PeerId) {
        self.baselines
            .retain(|(baseline_peer, _), _| *baseline_peer != peer);
    }

    pub fn len(&self) -> usize {
        self.baselines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.baselines.is_empty()
    }
}

#[cfg(test)]
mod tests;
