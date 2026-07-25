use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use afterglow_telemetry::{
    ArgumentDescriptor, CaptureConfig, CategoryId, Clock, Descriptor, DescriptorId, DescriptorKind,
    MetricBank, MetricDescriptor, MetricId, MetricKind, Recorder, TraceContext, Unit,
};

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

struct TrackingAllocator;

// SAFETY: every operation delegates unchanged to the system allocator.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        TRACKING.with(|tracking| {
            if tracking.get() {
                ALLOCATIONS.with(|count| count.set(count.get() + 1));
            }
        });
        // SAFETY: delegated with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the original pointer and layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn assert_no_alloc(operation: impl FnOnce()) {
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    operation();
    TRACKING.with(|tracking| tracking.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

struct FixedClock(Cell<u64>);

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        let now = self.0.get();
        self.0.set(now + 1);
        now
    }
}

static TRACE_DESCRIPTORS: [Descriptor; 1] = [Descriptor::new(
    CategoryId(0),
    "test",
    "event",
    DescriptorKind::Instant,
    ArgumentDescriptor::NONE,
    ArgumentDescriptor::NONE,
)];

static METRIC_DESCRIPTORS: [MetricDescriptor; 1] = [MetricDescriptor::new(
    CategoryId(0),
    "test",
    "count",
    MetricKind::Counter,
    Unit::Count,
)];

#[test]
fn sealed_recording_and_metrics_allocate_nothing() {
    let mut recorder = Recorder::new(&TRACE_DESCRIPTORS, 8, FixedClock(Cell::new(1))).unwrap();
    let metrics = MetricBank::new(&METRIC_DESCRIPTORS);

    assert_no_alloc(|| {
        let _ = recorder.instant(DescriptorId(0), TraceContext(1), 2, 3);
        let _ = metrics.counter_add(MetricId(0), 1);
    });

    recorder.arm(CaptureConfig::all(1)).unwrap();
    assert_no_alloc(|| {
        let _ = recorder.instant(DescriptorId(0), TraceContext(1), 2, 3);
        let _ = metrics.counter_add(MetricId(0), 1);
    });
}
