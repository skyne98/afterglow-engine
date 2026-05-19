use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::core::{
    identity::{ChunkId, ChunkMembership, StableEntityId, StableEntityRegistry},
    schedule::AfterglowSet,
};

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub struct ChunkInterestPeer {
    pub radius: u64,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct PeerChunkInterest {
    interests: BTreeMap<StableEntityId, BTreeSet<ChunkId>>,
}

pub struct ChunkInterestPlugin;

impl Default for ChunkInterestPeer {
    fn default() -> Self {
        Self { radius: 0 }
    }
}

impl ChunkInterestPeer {
    pub const fn with_radius(radius: u64) -> Self {
        Self { radius }
    }
}

impl Plugin for ChunkInterestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PeerChunkInterest>()
            .register_type::<ChunkInterestPeer>()
            .register_type::<PeerChunkInterest>()
            .add_systems(
                Update,
                update_peer_chunk_interest.in_set(AfterglowSet::PreparePersistence),
            );
    }
}

impl PeerChunkInterest {
    pub fn is_interested(&self, peer: StableEntityId, chunk: ChunkId) -> bool {
        self.interests
            .get(&peer)
            .is_some_and(|chunks| chunks.contains(&chunk))
    }

    pub fn peer_chunks(&self, peer: StableEntityId) -> impl Iterator<Item = ChunkId> + '_ {
        self.interests
            .get(&peer)
            .into_iter()
            .flat_map(|chunks| chunks.iter().copied())
    }

    pub fn interested_peers(&self, chunk: ChunkId) -> impl Iterator<Item = StableEntityId> + '_ {
        self.interests
            .iter()
            .filter_map(move |(peer, chunks)| chunks.contains(&chunk).then_some(*peer))
    }

    pub fn interested_entities(
        &self,
        peer: StableEntityId,
        registry: &StableEntityRegistry,
    ) -> Vec<Entity> {
        let mut entities = self
            .peer_chunks(peer)
            .flat_map(|chunk| registry.chunk_entities(chunk).iter().copied())
            .collect::<Vec<_>>();
        entities.sort();
        entities.dedup();
        entities
    }

    pub fn set_peer_chunks(
        &mut self,
        peer: StableEntityId,
        chunks: impl IntoIterator<Item = ChunkId>,
    ) {
        let chunks = chunks
            .into_iter()
            .filter(|chunk| chunk.is_valid())
            .collect::<BTreeSet<_>>();
        if chunks.is_empty() {
            self.interests.remove(&peer);
        } else {
            self.interests.insert(peer, chunks);
        }
    }

    pub fn clear(&mut self) {
        self.interests.clear();
    }
}

fn update_peer_chunk_interest(
    mut interest: ResMut<PeerChunkInterest>,
    peers: Query<(&StableEntityId, &ChunkMembership, &ChunkInterestPeer)>,
) {
    interest.clear();
    for (peer, membership, config) in &peers {
        interest.set_peer_chunks(*peer, chunk_neighborhood(membership.chunk, config.radius));
    }
}

fn chunk_neighborhood(center: ChunkId, radius: u64) -> impl Iterator<Item = ChunkId> {
    let raw = center.as_raw();
    let start = raw.saturating_sub(radius).max(1);
    let end = raw.saturating_add(radius);
    (start..=end).map(ChunkId::from_raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AfterglowCorePlugin;

    const PEER_A: StableEntityId = StableEntityId::from_raw(1_001);
    const PEER_B: StableEntityId = StableEntityId::from_raw(1_002);
    const CHUNK_A: ChunkId = ChunkId::from_raw(10);
    const CHUNK_B: ChunkId = ChunkId::from_raw(20);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin, ChunkInterestPlugin));
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn empty_interest_returns_nothing() {
        let app = app();
        let interest = app.world().resource::<PeerChunkInterest>();

        assert_eq!(interest.peer_chunks(PEER_A).collect::<Vec<_>>(), []);
        assert_eq!(interest.interested_peers(CHUNK_A).collect::<Vec<_>>(), []);
    }

    #[test]
    fn peer_added_to_single_chunk() {
        let mut app = app();
        spawn_peer(&mut app, PEER_A, CHUNK_A, 0);
        app.update();

        let interest = app.world().resource::<PeerChunkInterest>();
        assert!(interest.is_interested(PEER_A, CHUNK_A));
        assert_eq!(interest.peer_chunks(PEER_A).collect::<Vec<_>>(), [CHUNK_A]);
    }

    #[test]
    fn peer_radius_includes_neighboring_chunk_ids() {
        let mut app = app();
        spawn_peer(&mut app, PEER_A, CHUNK_A, 2);
        app.update();

        let chunks = app
            .world()
            .resource::<PeerChunkInterest>()
            .peer_chunks(PEER_A)
            .collect::<Vec<_>>();
        assert_eq!(
            chunks,
            [8, 9, 10, 11, 12]
                .into_iter()
                .map(ChunkId::from_raw)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn chunk_zero_boundary_does_not_create_invalid_interest() {
        let mut app = app();
        spawn_peer(&mut app, PEER_A, ChunkId::from_raw(1), 2);
        app.update();

        let chunks = app
            .world()
            .resource::<PeerChunkInterest>()
            .peer_chunks(PEER_A)
            .collect::<Vec<_>>();
        assert_eq!(
            chunks,
            [1, 2, 3]
                .into_iter()
                .map(ChunkId::from_raw)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn peer_chunk_change_replaces_old_interest() {
        let mut app = app();
        let peer = spawn_peer(&mut app, PEER_A, CHUNK_A, 0);
        app.update();
        app.world_mut()
            .entity_mut(peer)
            .insert(ChunkMembership::new(CHUNK_B));
        app.update();

        let interest = app.world().resource::<PeerChunkInterest>();
        assert!(!interest.is_interested(PEER_A, CHUNK_A));
        assert!(interest.is_interested(PEER_A, CHUNK_B));
    }

    #[test]
    fn despawned_peer_is_removed_from_interest() {
        let mut app = app();
        let peer = spawn_peer(&mut app, PEER_A, CHUNK_A, 0);
        app.update();
        app.world_mut().entity_mut(peer).despawn();
        app.update();

        assert_eq!(
            app.world()
                .resource::<PeerChunkInterest>()
                .peer_chunks(PEER_A)
                .collect::<Vec<_>>(),
            []
        );
    }

    #[test]
    fn interested_peers_are_deterministic_for_shared_chunks() {
        let mut app = app();
        spawn_peer(&mut app, PEER_A, CHUNK_A, 0);
        spawn_peer(&mut app, PEER_B, CHUNK_A, 0);
        app.update();

        assert_eq!(
            app.world()
                .resource::<PeerChunkInterest>()
                .interested_peers(CHUNK_A)
                .collect::<Vec<_>>(),
            [PEER_A, PEER_B]
        );
    }

    #[test]
    fn interested_entities_fans_out_through_stable_registry_chunks() {
        let mut app = app();
        spawn_peer(&mut app, PEER_A, CHUNK_A, 0);
        let visible = app
            .world_mut()
            .spawn((
                StableEntityId::from_raw(2_001),
                ChunkMembership::new(CHUNK_A),
            ))
            .id();
        app.world_mut().spawn((
            StableEntityId::from_raw(2_002),
            ChunkMembership::new(CHUNK_B),
        ));
        app.update();

        let interest = app.world().resource::<PeerChunkInterest>();
        let registry = app.world().resource::<StableEntityRegistry>();
        assert!(
            interest
                .interested_entities(PEER_A, registry)
                .contains(&visible)
        );
    }

    fn spawn_peer(app: &mut App, peer: StableEntityId, chunk: ChunkId, radius: u64) -> Entity {
        app.world_mut()
            .spawn((
                peer,
                ChunkMembership::new(chunk),
                ChunkInterestPeer::with_radius(radius),
            ))
            .id()
    }
}
