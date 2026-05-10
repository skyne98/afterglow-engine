use afterglow_engine::{
    core::identity::StableEntityId,
    network::interpolation::{RemoteEntitySample, RemoteInterpolationBuffer},
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(1_024, 8, 4_096);
    run_case(10_000, 4, 512);
    run_case(100_000, 2, 32);
}

fn run_case(entity_count: u128, samples_per_entity: u32, iterations: u32) {
    let buffer = buffer(entity_count, samples_per_entity);
    let latest_tick = samples_per_entity;
    let interpolate_time = measure(iterations, || {
        for entity_index in 1..=entity_count {
            black_box(
                buffer.sample_for_server_tick(StableEntityId::from_raw(entity_index), latest_tick),
            );
        }
    });
    let extrapolate_tick = samples_per_entity as f32 + 1.0;
    let extrapolate_time = measure(iterations, || {
        for entity_index in 1..=entity_count {
            black_box(buffer.sample_at(StableEntityId::from_raw(entity_index), extrapolate_tick));
        }
    });

    println!(
        "interpolation entities={entity_count} samples_per_entity={samples_per_entity} interpolate_all={} extrapolate_all={}",
        fmt(interpolate_time / iterations),
        fmt(extrapolate_time / iterations),
    );
}

fn buffer(entity_count: u128, samples_per_entity: u32) -> RemoteInterpolationBuffer {
    let mut buffer = RemoteInterpolationBuffer::default().with_timing(1, 2);
    for entity_index in 1..=entity_count {
        let entity = StableEntityId::from_raw(entity_index);
        for tick in 1..=samples_per_entity {
            buffer.record(
                entity,
                tick,
                RemoteEntitySample::default()
                    .with_field("x", entity_index as f32 + tick as f32)
                    .with_field("y", tick as f32)
                    .with_field("z", entity_index as f32 * 0.5),
            );
        }
    }
    buffer
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
