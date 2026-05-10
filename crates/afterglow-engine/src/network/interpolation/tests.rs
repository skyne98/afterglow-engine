use super::*;

fn entity(id: u128) -> StableEntityId {
    StableEntityId::from_raw(id)
}

fn sample(x: f32) -> RemoteEntitySample {
    RemoteEntitySample::default()
        .with_field("x", x)
        .with_field("y", x * 2.0)
}

#[test]
fn exact_tick_returns_exact_sample() {
    let mut buffer = RemoteInterpolationBuffer::default();
    buffer.record(entity(1), 10, sample(5.0));

    let result = buffer.sample_at(entity(1), 10.0).unwrap();

    assert_eq!(result.mode, SmoothingMode::Exact);
    assert_eq!(result.fields["x"], 5.0);
}

#[test]
fn interpolates_between_authoritative_samples() {
    let mut buffer = RemoteInterpolationBuffer::default();
    buffer.record(entity(1), 10, sample(0.0));
    buffer.record(entity(1), 12, sample(10.0));

    let result = buffer.sample_at(entity(1), 11.0).unwrap();

    assert_eq!(result.mode, SmoothingMode::Interpolated);
    assert_eq!(result.fields["x"], 5.0);
    assert_eq!(result.fields["y"], 10.0);
}

#[test]
fn sample_for_server_tick_renders_behind_latest_tick() {
    let mut buffer = RemoteInterpolationBuffer::default().with_timing(2, 2);
    buffer.record(entity(1), 8, sample(8.0));
    buffer.record(entity(1), 10, sample(10.0));

    let result = buffer.sample_for_server_tick(entity(1), 12).unwrap();

    assert_eq!(result.tick, 10.0);
    assert_eq!(result.fields["x"], 10.0);
}

#[test]
fn extrapolates_from_last_two_samples_within_limit() {
    let mut buffer = RemoteInterpolationBuffer::default().with_timing(2, 2);
    buffer.record(entity(1), 10, sample(10.0));
    buffer.record(entity(1), 12, sample(14.0));

    let result = buffer.sample_at(entity(1), 13.0).unwrap();

    assert_eq!(result.mode, SmoothingMode::Extrapolated);
    assert_eq!(result.fields["x"], 16.0);
}

#[test]
fn extrapolation_stops_after_configured_limit() {
    let mut buffer = RemoteInterpolationBuffer::default().with_timing(2, 1);
    buffer.record(entity(1), 10, sample(10.0));
    buffer.record(entity(1), 12, sample(14.0));

    assert!(buffer.sample_at(entity(1), 14.0).is_none());
}

#[test]
fn fields_missing_from_one_sample_are_not_blended() {
    let mut buffer = RemoteInterpolationBuffer::default();
    buffer.record(
        entity(1),
        10,
        RemoteEntitySample::default()
            .with_field("x", 0.0)
            .with_field("only_a", 1.0),
    );
    buffer.record(
        entity(1),
        12,
        RemoteEntitySample::default()
            .with_field("x", 10.0)
            .with_field("only_b", 1.0),
    );

    let result = buffer.sample_at(entity(1), 11.0).unwrap();

    assert_eq!(result.fields.len(), 1);
    assert_eq!(result.fields["x"], 5.0);
}

#[test]
fn prune_and_clear_remove_old_samples() {
    let mut buffer = RemoteInterpolationBuffer::default();
    buffer.record(entity(1), 1, sample(1.0));
    buffer.record(entity(1), 2, sample(2.0));
    buffer.record(entity(2), 1, sample(1.0));

    buffer.prune_before(2);
    assert_eq!(buffer.sample_count(entity(1)), 1);
    assert_eq!(buffer.sample_count(entity(2)), 0);

    buffer.clear_entity(entity(1));
    assert_eq!(buffer.sample_count(entity(1)), 0);
}
