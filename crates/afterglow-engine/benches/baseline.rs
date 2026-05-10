use afterglow_engine::{
    core::identity::{ChunkId, StableEntityId},
    network::{
        NetworkPlayerId, PeerId,
        baseline::{ReconnectBaseline, ReplicationSaveData},
        interest::InterestMap,
        replication::ReplicationWorld,
    },
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(1_024, 8, 128);
    run_case(10_000, 8, 32);
    run_case(100_000, 4, 4);
}

fn run_case(entity_count: u128, field_count: u32, iterations: u32) {
    let world = world(entity_count, field_count);
    let save = ReplicationSaveData::from_world(&world, 10);
    let bytes = save.to_json().unwrap();
    let interest = interest(entity_count);
    let player = NetworkPlayerId(1);

    let save_json_time = measure(iterations, || {
        black_box(save.to_json().unwrap());
    });
    let load_json_time = measure(iterations, || {
        let save = ReplicationSaveData::from_json(&bytes).unwrap();
        black_box(save.restore_world());
    });
    let reconnect_time = measure(iterations, || {
        black_box(ReconnectBaseline::filtered(
            PeerId(1),
            player,
            &save.snapshot,
            &interest,
        ));
    });

    println!(
        "baseline entities={entity_count} fields={field_count} json_bytes={} save_json={} load_restore={} reconnect_filtered={}",
        bytes.len(),
        fmt(save_json_time / iterations),
        fmt(load_json_time / iterations),
        fmt(reconnect_time / iterations),
    );
}

fn world(entity_count: u128, field_count: u32) -> ReplicationWorld {
    let mut world = ReplicationWorld::default();
    for entity_index in 1..=entity_count {
        let entity = StableEntityId::from_raw(entity_index);
        for field_index in 0..field_count {
            world.set_field(
                entity,
                format!("field_{field_index:02}"),
                [field_index as u8; 4],
            );
        }
    }
    world.clear_changes();
    world
}

fn interest(entity_count: u128) -> InterestMap {
    let mut interest = InterestMap::default();
    for entity_index in 1..=entity_count {
        let chunk = ChunkId::from_raw(((entity_index - 1) / 1_000 + 1) as u64);
        interest.set_entity_chunk(StableEntityId::from_raw(entity_index), chunk);
    }
    interest.set_player_chunks(NetworkPlayerId(1), (1_u64..=8).map(ChunkId::from_raw));
    interest
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
