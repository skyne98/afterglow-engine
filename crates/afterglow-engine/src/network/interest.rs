use crate::{
    core::identity::{ChunkId, StableEntityId},
    network::{
        NetworkPlayerId,
        replication::{EntityDelta, EntitySnapshot, WorldDelta, WorldSnapshot},
    },
};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct InterestMap {
    entity_chunks: BTreeMap<StableEntityId, ChunkId>,
    player_chunks: BTreeMap<NetworkPlayerId, BTreeSet<ChunkId>>,
}

impl InterestMap {
    pub fn set_entity_chunk(&mut self, entity: StableEntityId, chunk: ChunkId) {
        self.entity_chunks.insert(entity, chunk);
    }

    pub fn remove_entity(&mut self, entity: StableEntityId) {
        self.entity_chunks.remove(&entity);
    }

    pub fn set_player_chunks(
        &mut self,
        player: NetworkPlayerId,
        chunks: impl IntoIterator<Item = ChunkId>,
    ) {
        self.player_chunks
            .insert(player, chunks.into_iter().collect());
    }

    pub fn visible_chunks(&self, player: NetworkPlayerId) -> Option<&BTreeSet<ChunkId>> {
        self.player_chunks.get(&player)
    }

    pub fn can_see_entity(&self, player: NetworkPlayerId, entity: StableEntityId) -> bool {
        let Some(entity_chunk) = self.entity_chunks.get(&entity) else {
            return false;
        };
        self.player_chunks
            .get(&player)
            .is_some_and(|chunks| chunks.contains(entity_chunk))
    }

    pub fn filter_snapshot(
        &self,
        player: NetworkPlayerId,
        snapshot: &WorldSnapshot,
    ) -> WorldSnapshot {
        WorldSnapshot {
            tick: snapshot.tick,
            entities: snapshot
                .entities
                .iter()
                .filter(|entity| self.can_see_entity(player, entity.entity))
                .cloned()
                .collect::<Vec<EntitySnapshot>>(),
        }
    }

    pub fn filter_delta(&self, player: NetworkPlayerId, delta: &WorldDelta) -> WorldDelta {
        WorldDelta {
            from_tick: delta.from_tick,
            to_tick: delta.to_tick,
            changes: delta
                .changes
                .iter()
                .filter(|entity| self.can_see_entity(player, entity.entity))
                .cloned()
                .collect::<Vec<EntityDelta>>(),
            removed: delta
                .removed
                .iter()
                .copied()
                .filter(|entity| self.can_see_entity(player, *entity))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
