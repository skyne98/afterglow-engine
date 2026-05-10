use afterglow_engine::{
    core::identity::{ChunkId, StableEntityId},
    network::{
        NetworkPlayerId,
        interest::InterestMap,
        replication::{ReplicationWorld, WorldDelta},
    },
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(128, 8, 128, 8, 64);
    run_case(1_024, 8, 128, 8, 64);
    run_case(8_192, 8, 64, 8, 64);
    run_case(100_000, 8, 16, 100, 1_000);
    run_case(100_000, 8, 4, 8, 64);
}

fn run_case(
    entity_count: u128,
    field_count: u32,
    iterations: u32,
    change_step: usize,
    remove_step: usize,
) {
    let mut baseline_world = world(entity_count, field_count, 0);
    baseline_world.clear_changes();
    let baseline = baseline_world.snapshot(1);
    let mut changed_world = baseline_world.clone();
    mutate_subset(
        &mut changed_world,
        entity_count,
        field_count,
        change_step,
        remove_step,
    );
    let delta = changed_world.delta_since(&baseline, 2);
    let dirty_delta = changed_world.dirty_delta_since(&baseline, 2);
    let interest = interest(entity_count);
    let player = NetworkPlayerId(1);
    let mut baseline_client = ReplicationWorld::default();
    baseline_client.apply_snapshot(&baseline);

    let snapshot_time = measure(iterations, || {
        black_box(changed_world.snapshot(2));
    });
    let delta_time = measure(iterations, || {
        black_box(changed_world.delta_since(&baseline, 2));
    });
    let dirty_delta_time = measure(iterations, || {
        black_box(changed_world.dirty_delta_since(&baseline, 2));
    });
    let snapshot_apply_time = measure(iterations, || {
        let mut client = ReplicationWorld::default();
        client.apply_snapshot(&baseline);
        black_box(client);
    });
    let dirty_apply_time = measure(iterations, || {
        baseline_client.apply_delta(&dirty_delta);
        black_box(&baseline_client);
    });
    let filter_snapshot_time = measure(iterations, || {
        black_box(interest.filter_snapshot(player, &baseline));
    });
    let filter_dirty_delta_time = measure(iterations, || {
        black_box(interest.filter_delta(player, &dirty_delta));
    });
    let visible_snapshot = interest.filter_snapshot(player, &baseline);
    let visible_dirty_delta = interest.filter_delta(player, &dirty_delta);

    println!(
        "replication entities={entity_count} fields={field_count} changed={} removed={} visible_snapshot={} visible_dirty_delta={} snapshot={} full_delta={} dirty_delta={} snapshot_apply={} dirty_apply={} interest_snapshot={} interest_dirty_delta={}",
        dirty_delta.changes.len(),
        dirty_delta.removed.len(),
        visible_snapshot.entities.len(),
        delta_entity_count(&visible_dirty_delta),
        fmt(snapshot_time / iterations),
        fmt(delta_time / iterations),
        fmt(dirty_delta_time / iterations),
        fmt(snapshot_apply_time / iterations),
        fmt(dirty_apply_time / iterations),
        fmt(filter_snapshot_time / iterations),
        fmt(filter_dirty_delta_time / iterations),
    );
    black_box(delta);
}

fn world(entity_count: u128, field_count: u32, seed: u8) -> ReplicationWorld {
    let mut world = ReplicationWorld::default();
    for entity_index in 1..=entity_count {
        let entity = StableEntityId::from_raw(entity_index);
        for field_index in 0..field_count {
            let value = [
                seed,
                (entity_index & 0xff) as u8,
                field_index as u8,
                ((entity_index >> 8) & 0xff) as u8,
            ];
            world.set_field(entity, format!("field_{field_index:02}"), value);
        }
    }
    world
}

fn mutate_subset(
    world: &mut ReplicationWorld,
    entity_count: u128,
    field_count: u32,
    change_step: usize,
    remove_step: usize,
) {
    for entity_index in (1..=entity_count).step_by(change_step) {
        let entity = StableEntityId::from_raw(entity_index);
        world.set_field(entity, "field_00", [9, 9, 9, 9]);
        if field_count > 1 {
            world.remove_field(entity, "field_01");
        }
    }
    for entity_index in (16..=entity_count).step_by(remove_step) {
        world.remove_entity(StableEntityId::from_raw(entity_index));
    }
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

fn delta_entity_count(delta: &WorldDelta) -> usize {
    delta.changes.len() + delta.removed.len()
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
