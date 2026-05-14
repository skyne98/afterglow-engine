use crate::{
    core::identity::{ChunkId, StableEntityId},
    network::{
        NetworkPlayerId,
        replication::{EntityDelta, EntitySnapshot, WorldDelta, WorldSnapshot},
    },
};
use bevy::prelude::*;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

const PARALLEL_FANOUT_ENTITY_THRESHOLD: usize = 262_144;

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct InterestMap {
    entity_chunks: BTreeMap<StableEntityId, ChunkId>,
    removed_entity_chunks: BTreeMap<StableEntityId, ChunkId>,
    player_chunks: BTreeMap<NetworkPlayerId, BTreeSet<ChunkId>>,
    cleanup_routed_removals: bool,
}

impl Default for InterestMap {
    fn default() -> Self {
        Self {
            entity_chunks: BTreeMap::new(),
            removed_entity_chunks: BTreeMap::new(),
            player_chunks: BTreeMap::new(),
            cleanup_routed_removals: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkSnapshotFanout {
    pub tick: u32,
    pub chunks: BTreeMap<ChunkId, Vec<EntitySnapshot>>,
    pub chunk_players: BTreeMap<ChunkId, Vec<NetworkPlayerId>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkSnapshotRefFanout<'a> {
    pub tick: u32,
    pub chunks: BTreeMap<ChunkId, Vec<&'a EntitySnapshot>>,
    pub chunk_players: BTreeMap<ChunkId, Vec<NetworkPlayerId>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkDeltaFanout {
    pub from_tick: u32,
    pub to_tick: u32,
    pub chunks: BTreeMap<ChunkId, ChunkDeltaPayload>,
    pub chunk_players: BTreeMap<ChunkId, Vec<NetworkPlayerId>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkDeltaPayload {
    pub changes: Vec<EntityDelta>,
    pub removed: Vec<StableEntityId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkDeltaRefFanout<'a> {
    pub from_tick: u32,
    pub to_tick: u32,
    pub chunks: BTreeMap<ChunkId, ChunkDeltaRefPayload<'a>>,
    pub chunk_players: BTreeMap<ChunkId, Vec<NetworkPlayerId>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkDeltaRefPayload<'a> {
    pub changes: Vec<&'a EntityDelta>,
    pub removed: Vec<StableEntityId>,
}

impl InterestMap {
    pub fn set_cleanup_routed_removals(&mut self, enabled: bool) {
        self.cleanup_routed_removals = enabled;
    }

    pub fn set_entity_chunk(&mut self, entity: StableEntityId, chunk: ChunkId) {
        self.entity_chunks.insert(entity, chunk);
    }

    pub fn remove_entity(&mut self, entity: StableEntityId) {
        if let Some(chunk) = self.entity_chunks.remove(&entity) {
            self.removed_entity_chunks.insert(entity, chunk);
        }
    }

    pub fn clear_removed_entities(&mut self, entities: impl IntoIterator<Item = StableEntityId>) {
        for entity in entities {
            self.removed_entity_chunks.remove(&entity);
        }
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

    pub fn filter_snapshots(
        &self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        snapshot: &WorldSnapshot,
    ) -> BTreeMap<NetworkPlayerId, WorldSnapshot> {
        let mut snapshots = BTreeMap::new();
        let chunk_players =
            self.index_players_by_chunk(players, &mut snapshots, |_| WorldSnapshot {
                tick: snapshot.tick,
                entities: Vec::new(),
            });

        for entity in &snapshot.entities {
            let Some(chunk) = self.entity_chunks.get(&entity.entity) else {
                continue;
            };
            let Some(players) = chunk_players.get(chunk) else {
                continue;
            };
            for player in players {
                snapshots
                    .get_mut(player)
                    .expect("indexed player output should exist")
                    .entities
                    .push(entity.clone());
            }
        }

        snapshots
    }

    pub fn snapshot_chunk_fanout(
        &self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        snapshot: &WorldSnapshot,
    ) -> ChunkSnapshotFanout {
        let fanout = self.snapshot_chunk_ref_fanout(players, snapshot);
        ChunkSnapshotFanout {
            tick: fanout.tick,
            chunks: fanout
                .chunks
                .into_iter()
                .map(|(chunk, entities)| (chunk, entities.into_iter().cloned().collect()))
                .collect(),
            chunk_players: fanout.chunk_players,
        }
    }

    pub fn snapshot_chunk_ref_fanout<'a>(
        &self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        snapshot: &'a WorldSnapshot,
    ) -> ChunkSnapshotRefFanout<'a> {
        let chunk_players = self.chunk_players(players);
        let requested_chunks = chunk_players.keys().copied().collect::<BTreeSet<_>>();
        let mut chunks = requested_chunks
            .iter()
            .map(|chunk| (*chunk, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        if snapshot.entities.len() >= PARALLEL_FANOUT_ENTITY_THRESHOLD {
            let mut entities = snapshot
                .entities
                .par_iter()
                .filter_map(|entity| {
                    let chunk = *self.entity_chunks.get(&entity.entity)?;
                    requested_chunks.contains(&chunk).then_some((chunk, entity))
                })
                .collect::<Vec<_>>();
            entities.par_sort_unstable_by_key(|(chunk, entity)| (*chunk, entity.entity));

            for (chunk, entity) in entities {
                chunks
                    .get_mut(&chunk)
                    .expect("requested chunk output should exist")
                    .push(entity);
            }
        } else {
            for entity in &snapshot.entities {
                let Some(chunk) = self.entity_chunks.get(&entity.entity) else {
                    continue;
                };
                if let Some(entities) = chunks.get_mut(chunk) {
                    entities.push(entity);
                }
            }
        }

        ChunkSnapshotRefFanout {
            tick: snapshot.tick,
            chunks,
            chunk_players,
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
                .filter(|entity| {
                    self.chunk_for_removed_entity(*entity).is_some_and(|chunk| {
                        self.player_chunks
                            .get(&player)
                            .is_some_and(|chunks| chunks.contains(&chunk))
                    })
                })
                .collect(),
        }
    }

    pub fn filter_deltas(
        &mut self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        delta: &WorldDelta,
    ) -> BTreeMap<NetworkPlayerId, WorldDelta> {
        let mut deltas = BTreeMap::new();
        let chunk_players = self.index_players_by_chunk(players, &mut deltas, |_| WorldDelta {
            from_tick: delta.from_tick,
            to_tick: delta.to_tick,
            changes: Vec::new(),
            removed: Vec::new(),
        });

        for entity in &delta.changes {
            let Some(chunk) = self.entity_chunks.get(&entity.entity) else {
                continue;
            };
            let Some(players) = chunk_players.get(chunk) else {
                continue;
            };
            for player in players {
                deltas
                    .get_mut(player)
                    .expect("indexed player output should exist")
                    .changes
                    .push(entity.clone());
            }
        }
        for entity in &delta.removed {
            let Some(chunk) = self.chunk_for_removed_entity(*entity) else {
                continue;
            };
            let Some(players) = chunk_players.get(&chunk) else {
                continue;
            };
            for player in players {
                deltas
                    .get_mut(player)
                    .expect("indexed player output should exist")
                    .removed
                    .push(*entity);
            }
        }

        let routed = deltas
            .values()
            .flat_map(|delta| delta.removed.iter().copied())
            .collect::<Vec<_>>();
        self.cleanup_routed_removals_if_enabled(routed);
        deltas
    }

    pub fn delta_chunk_fanout(
        &mut self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        delta: &WorldDelta,
    ) -> ChunkDeltaFanout {
        let fanout = self.delta_chunk_ref_fanout(players, delta);
        ChunkDeltaFanout {
            from_tick: fanout.from_tick,
            to_tick: fanout.to_tick,
            chunks: fanout
                .chunks
                .into_iter()
                .map(|(chunk, payload)| {
                    (
                        chunk,
                        ChunkDeltaPayload {
                            changes: payload.changes.into_iter().cloned().collect(),
                            removed: payload.removed,
                        },
                    )
                })
                .collect(),
            chunk_players: fanout.chunk_players,
        }
    }

    pub fn delta_chunk_ref_fanout<'a>(
        &mut self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        delta: &'a WorldDelta,
    ) -> ChunkDeltaRefFanout<'a> {
        let chunk_players = self.chunk_players(players);
        let requested_chunks = chunk_players.keys().copied().collect::<BTreeSet<_>>();
        let mut chunks = requested_chunks
            .iter()
            .map(|chunk| (*chunk, ChunkDeltaRefPayload::default()))
            .collect::<BTreeMap<_, _>>();
        if delta.changes.len() + delta.removed.len() >= PARALLEL_FANOUT_ENTITY_THRESHOLD {
            let mut changes = delta
                .changes
                .par_iter()
                .filter_map(|entity| {
                    let chunk = *self.entity_chunks.get(&entity.entity)?;
                    requested_chunks.contains(&chunk).then_some((chunk, entity))
                })
                .collect::<Vec<_>>();
            let mut removed = delta
                .removed
                .par_iter()
                .filter_map(|entity| {
                    let chunk = self.chunk_for_removed_entity(*entity)?;
                    requested_chunks
                        .contains(&chunk)
                        .then_some((chunk, *entity))
                })
                .collect::<Vec<_>>();

            changes.par_sort_unstable_by_key(|(chunk, entity)| (*chunk, entity.entity));
            removed.par_sort_unstable_by_key(|(chunk, entity)| (*chunk, *entity));

            for (chunk, entity) in changes {
                chunks
                    .get_mut(&chunk)
                    .expect("requested chunk output should exist")
                    .changes
                    .push(entity);
            }
            for (chunk, entity) in removed {
                chunks
                    .get_mut(&chunk)
                    .expect("requested chunk output should exist")
                    .removed
                    .push(entity);
            }
        } else {
            for entity in &delta.changes {
                let Some(chunk) = self.entity_chunks.get(&entity.entity) else {
                    continue;
                };
                if let Some(payload) = chunks.get_mut(chunk) {
                    payload.changes.push(entity);
                }
            }
            for entity in &delta.removed {
                let Some(chunk) = self.chunk_for_removed_entity(*entity) else {
                    continue;
                };
                if let Some(payload) = chunks.get_mut(&chunk) {
                    payload.removed.push(*entity);
                }
            }
        }

        let routed = chunks
            .values()
            .flat_map(|payload| payload.removed.iter().copied())
            .collect::<Vec<_>>();
        self.cleanup_routed_removals_if_enabled(routed);

        ChunkDeltaRefFanout {
            from_tick: delta.from_tick,
            to_tick: delta.to_tick,
            chunks,
            chunk_players,
        }
    }

    fn chunk_players(
        &self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
    ) -> BTreeMap<ChunkId, Vec<NetworkPlayerId>> {
        let mut chunk_players = BTreeMap::<ChunkId, Vec<NetworkPlayerId>>::new();
        for player in unique_players(players) {
            if let Some(chunks) = self.player_chunks.get(&player) {
                for chunk in chunks {
                    chunk_players.entry(*chunk).or_default().push(player);
                }
            }
        }
        chunk_players
    }

    fn index_players_by_chunk<T>(
        &self,
        players: impl IntoIterator<Item = NetworkPlayerId>,
        outputs: &mut BTreeMap<NetworkPlayerId, T>,
        mut init_output: impl FnMut(NetworkPlayerId) -> T,
    ) -> BTreeMap<ChunkId, Vec<NetworkPlayerId>> {
        let mut chunk_players = BTreeMap::<ChunkId, Vec<NetworkPlayerId>>::new();
        for player in unique_players(players) {
            outputs.insert(player, init_output(player));
            if let Some(chunks) = self.player_chunks.get(&player) {
                for chunk in chunks {
                    chunk_players.entry(*chunk).or_default().push(player);
                }
            }
        }
        chunk_players
    }

    fn chunk_for_removed_entity(&self, entity: StableEntityId) -> Option<ChunkId> {
        self.removed_entity_chunks
            .get(&entity)
            .or_else(|| self.entity_chunks.get(&entity))
            .copied()
    }

    fn cleanup_routed_removals_if_enabled(
        &mut self,
        entities: impl IntoIterator<Item = StableEntityId>,
    ) {
        if self.cleanup_routed_removals {
            self.clear_removed_entities(entities);
        }
    }
}

fn unique_players(players: impl IntoIterator<Item = NetworkPlayerId>) -> BTreeSet<NetworkPlayerId> {
    players.into_iter().collect()
}

#[cfg(test)]
mod tests;
