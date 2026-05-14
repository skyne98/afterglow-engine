use afterglow_engine::{
    core::{
        AfterglowCorePlugin,
        identity::{ChunkId, ChunkMembership, Persistent, StableEntityId},
    },
    network::{
        NetworkPlayerId,
        interest::InterestMap,
        replication::{ReplicationWorld, WorldSnapshot},
    },
    persistence::{
        AfterglowPersistencePlugin, ChunkPersistentDelta, PersistenceAppExt, apply_chunk_deltas,
        capture_chunk_deltas,
    },
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const PLAYERS: u64 = 10_000;
const CHUNKS: u64 = 1_024;
const VISIBLE_CHUNKS_PER_PLAYER: u64 = 9;
const ENTITIES_PER_CHUNK: u64 = 64;
const STREAMING_CHUNKS_PER_STEP: u64 = 128;

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BenchPersistedState {
    hp: u16,
    flags: u16,
    seed: u32,
}

fn main() {
    run_interest_case();
    run_persistence_case();
}

fn run_interest_case() {
    let entity_count = CHUNKS * ENTITIES_PER_CHUNK;
    let mut interest = interest_map(entity_count);
    let snapshot = snapshot(entity_count);

    let update_time = measure(1, || {
        move_players_to_next_chunk(&mut interest);
    });
    let chunk_ref_fanout_time = measure(1, || {
        black_box(
            interest
                .snapshot_chunk_ref_fanout((1..=PLAYERS).map(NetworkPlayerId), &snapshot)
                .chunks
                .values()
                .map(Vec::len)
                .sum::<usize>(),
        );
    });
    let chunk_owned_fanout_time = measure(1, || {
        black_box(
            interest
                .snapshot_chunk_fanout((1..=PLAYERS).map(NetworkPlayerId), &snapshot)
                .chunks
                .values()
                .map(Vec::len)
                .sum::<usize>(),
        );
    });
    let filter_batch_time = measure(1, || {
        black_box(
            interest
                .filter_snapshots((1..=PLAYERS).map(NetworkPlayerId), &snapshot)
                .values()
                .map(|snapshot| snapshot.entities.len())
                .sum::<usize>(),
        );
    });
    let legacy_filter_100_players_time = measure(1, || {
        black_box(filter_for_players(&interest, &snapshot, 100));
    });

    println!(
        "streaming_interest players={PLAYERS} chunks={CHUNKS} entities={entity_count} visible_chunks={VISIBLE_CHUNKS_PER_PLAYER} player_chunk_update={} chunk_ref_fanout_snapshot={} chunk_owned_fanout_snapshot={} batch_filter_snapshot_for_all_players={} legacy_filter_snapshot_for_100_players={}",
        fmt(update_time),
        fmt(chunk_ref_fanout_time),
        fmt(chunk_owned_fanout_time),
        fmt(filter_batch_time),
        fmt(legacy_filter_100_players_time),
    );
}

fn run_persistence_case() {
    let mut app = persistence_app();
    spawn_persistent_world(app.world_mut());

    let chunks = (1..=STREAMING_CHUNKS_PER_STEP)
        .map(ChunkId::from_raw)
        .collect::<Vec<_>>();
    let capture_time = measure(1, || {
        black_box(capture_chunks(app.world_mut(), &chunks));
    });

    let deltas = capture_chunks(app.world_mut(), &chunks);
    let mut restore_app = persistence_app();
    let apply_time = measure(1, || {
        black_box(apply_chunks(restore_app.world_mut(), &deltas));
    });

    println!(
        "streaming_persistence chunks_total={CHUNKS} entities_per_chunk={ENTITIES_PER_CHUNK} streaming_chunks={STREAMING_CHUNKS_PER_STEP} captured_entities={} capture_chunks={} apply_chunks={}",
        deltas
            .iter()
            .map(|delta| delta.entities.len())
            .sum::<usize>(),
        fmt(capture_time),
        fmt(apply_time),
    );
}

fn persistence_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
    ))
    .persist_component_as::<BenchPersistedState>("bench.persisted_state.v1");
    app
}

fn spawn_persistent_world(world: &mut World) {
    for chunk_index in 1..=CHUNKS {
        let chunk = ChunkId::from_raw(chunk_index);
        for entity_index in 0..ENTITIES_PER_CHUNK {
            let stable_id =
                StableEntityId::from_raw((chunk_index as u128 * 1_000_000) + entity_index as u128);
            world.spawn((
                Persistent,
                stable_id,
                ChunkMembership::new(chunk),
                BenchPersistedState {
                    hp: (entity_index % 100) as u16,
                    flags: (chunk_index % 16) as u16,
                    seed: (chunk_index as u32) ^ (entity_index as u32),
                },
            ));
        }
    }
}

fn capture_chunks(world: &mut World, chunks: &[ChunkId]) -> Vec<ChunkPersistentDelta> {
    capture_chunk_deltas(world, chunks.iter().copied()).unwrap()
}

fn apply_chunks(world: &mut World, deltas: &[ChunkPersistentDelta]) -> usize {
    apply_chunk_deltas(world, deltas).unwrap().spawned
}

fn snapshot(entity_count: u64) -> WorldSnapshot {
    let mut world = ReplicationWorld::default();
    for entity_index in 1..=entity_count {
        let entity = StableEntityId::from_raw(entity_index as u128);
        world.set_field(entity, "position", entity_index.to_le_bytes());
    }
    world.snapshot(1)
}

fn interest_map(entity_count: u64) -> InterestMap {
    let mut interest = InterestMap::default();
    for entity_index in 1..=entity_count {
        let chunk = ChunkId::from_raw(((entity_index - 1) / ENTITIES_PER_CHUNK) + 1);
        interest.set_entity_chunk(StableEntityId::from_raw(entity_index as u128), chunk);
    }
    for player_index in 1..=PLAYERS {
        interest.set_player_chunks(
            NetworkPlayerId(player_index),
            visible_chunks_for_player(player_index, 0),
        );
    }
    interest
}

fn move_players_to_next_chunk(interest: &mut InterestMap) {
    for player_index in 1..=PLAYERS {
        interest.set_player_chunks(
            NetworkPlayerId(player_index),
            visible_chunks_for_player(player_index, 1),
        );
    }
}

fn visible_chunks_for_player(player_index: u64, offset: u64) -> impl Iterator<Item = ChunkId> {
    let center = ((player_index + offset) % CHUNKS) + 1;
    (0..VISIBLE_CHUNKS_PER_PLAYER).map(move |visible_index| {
        let chunk = ((center + visible_index + CHUNKS - 1) % CHUNKS) + 1;
        ChunkId::from_raw(chunk)
    })
}

fn filter_for_players(
    interest: &InterestMap,
    snapshot: &WorldSnapshot,
    player_count: u64,
) -> usize {
    (1..=player_count)
        .map(|player_index| {
            interest
                .filter_snapshot(NetworkPlayerId(player_index), snapshot)
                .entities
                .len()
        })
        .sum()
}

fn measure(iterations: u32, mut f: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    start.elapsed()
}

fn fmt(duration: Duration) -> String {
    if duration.as_micros() >= 1_000 {
        format!("{:.3}ms", duration.as_secs_f64() * 1_000.0)
    } else {
        format!("{}us", duration.as_micros())
    }
}
