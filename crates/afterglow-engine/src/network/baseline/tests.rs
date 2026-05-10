use super::*;
use crate::{
    core::identity::{ChunkId, StableEntityId},
    network::replication::{EntitySnapshot, FieldValue},
};

fn world() -> ReplicationWorld {
    let mut world = ReplicationWorld::default();
    world.set_field(StableEntityId::from_raw(1), "hp", [100]);
    world.set_field(StableEntityId::from_raw(2), "door_open", [1]);
    world
}

#[test]
fn save_data_roundtrips_json_and_restores_world() {
    let world = world();
    let save = ReplicationSaveData::from_world(&world, 42);
    let bytes = save.to_json().unwrap();

    let decoded = ReplicationSaveData::from_json(&bytes).unwrap();
    let restored = decoded.restore_world();

    assert_eq!(decoded.tick, 42);
    assert_eq!(
        restored
            .entity(StableEntityId::from_raw(1))
            .unwrap()
            .field("hp"),
        Some([100].as_slice())
    );
    assert_eq!(restored.snapshot(42), world.snapshot(42));
}

#[test]
fn reconnect_baseline_can_be_filtered_by_interest() {
    let snapshot = world().snapshot(10);
    let peer = PeerId(7);
    let player = NetworkPlayerId(3);
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(StableEntityId::from_raw(1), ChunkId::from_raw(1));
    interest.set_entity_chunk(StableEntityId::from_raw(2), ChunkId::from_raw(2));
    interest.set_player_chunks(player, [ChunkId::from_raw(1)]);

    let baseline = ReconnectBaseline::filtered(peer, player, &snapshot, &interest);

    assert_eq!(baseline.peer, peer);
    assert_eq!(baseline.player, player);
    assert_eq!(baseline.snapshot.tick, 10);
    assert_eq!(baseline.snapshot.entities.len(), 1);
    assert_eq!(
        baseline.snapshot.entities[0].entity,
        StableEntityId::from_raw(1)
    );
}

#[test]
fn reconnect_store_replaces_and_clears_peer_baselines() {
    let peer = PeerId(1);
    let player = NetworkPlayerId(1);
    let mut store = ReconnectBaselineStore::default();
    let first = ReconnectBaseline::from_snapshot(
        peer,
        player,
        WorldSnapshot {
            tick: 1,
            entities: Vec::new(),
        },
    );
    let second = ReconnectBaseline::from_snapshot(
        peer,
        player,
        WorldSnapshot {
            tick: 2,
            entities: vec![EntitySnapshot {
                entity: StableEntityId::from_raw(5),
                fields: vec![FieldValue {
                    name: "hp".into(),
                    value: vec![5],
                }],
            }],
        },
    );

    assert!(store.insert(first).is_none());
    assert_eq!(
        store.insert(second),
        Some(ReconnectBaseline {
            peer,
            player,
            snapshot: WorldSnapshot {
                tick: 1,
                entities: Vec::new(),
            },
        })
    );
    assert_eq!(store.get(peer, player).unwrap().snapshot.tick, 2);
    store.clear_peer(peer);
    assert!(store.is_empty());
}
