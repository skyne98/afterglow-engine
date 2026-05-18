use afterglow_engine::{
    core::{
        AfterglowCorePlugin,
        identity::{ChunkId, ChunkMembership, Persistent, StableEntityId},
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

const CHUNKS: u64 = 1_024;
const ENTITIES_PER_CHUNK: u64 = 64;
const STREAMING_CHUNKS_PER_STEP: u64 = 128;

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BenchPersistedState {
    hp: u16,
    flags: u16,
    seed: u32,
}

fn main() {
    run_persistence_case();
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
