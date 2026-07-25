use std::hint::black_box;
use std::time::Instant;

use afterglow_telemetry::{
    ArgumentDescriptor, CaptureConfig, CategoryId, Descriptor, DescriptorId, DescriptorKind,
    MonotonicClock, Recorder, TraceContext,
};

static DESCRIPTORS: [Descriptor; 1] = [Descriptor::new(
    CategoryId(0),
    "bench",
    "instant",
    DescriptorKind::Instant,
    ArgumentDescriptor::NONE,
    ArgumentDescriptor::NONE,
)];

fn main() {
    const ITERATIONS: usize = 1_000_000;
    let disabled = Recorder::new(&DESCRIPTORS, 1, MonotonicClock).unwrap();
    let started = Instant::now();
    for index in 0..ITERATIONS {
        black_box(black_box(&disabled).instant(
            DescriptorId(0),
            TraceContext(index as u64),
            index as u64,
            0,
        ));
    }
    let disabled_ns = started.elapsed().as_nanos() as f64 / ITERATIONS as f64;

    let mut enabled = Recorder::new(&DESCRIPTORS, ITERATIONS, MonotonicClock).unwrap();
    enabled.arm(CaptureConfig::all(1)).unwrap();
    let started = Instant::now();
    for index in 0..ITERATIONS {
        black_box(black_box(&enabled).instant(
            DescriptorId(0),
            TraceContext(index as u64),
            index as u64,
            0,
        ));
    }
    let enabled_ns = started.elapsed().as_nanos() as f64 / ITERATIONS as f64;
    enabled.stop().unwrap();

    println!(
        "disabled={disabled_ns:.2} ns/event enabled={enabled_ns:.2} ns/event records={}",
        enabled.snapshot().unwrap().records.len()
    );
}
