use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

pub struct TrackingAllocator<A> {
    inner: A,
}

impl<A> TrackingAllocator<A> {
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }
}

fn record_allocation() {
    TRACKING.with(|tracking| {
        if tracking.get() {
            ALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
        }
    });
}

// SAFETY: Each operation uses the original pointer and layout with the inner
// allocator. The wrapper only changes thread-local counters.
unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller obeys the GlobalAlloc::alloc contract.
        unsafe { self.inner.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller obeys the GlobalAlloc::alloc_zeroed contract.
        unsafe { self.inner.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller obeys the GlobalAlloc::realloc contract.
        unsafe { self.inner.realloc(pointer, layout, new_size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The caller obeys the GlobalAlloc::dealloc contract.
        unsafe { self.inner.dealloc(pointer, layout) }
    }
}

pub fn assert_no_alloc<T>(operation: impl FnOnce() -> T) -> T {
    TRACKING.with(|tracking| {
        assert!(
            !tracking.replace(true),
            "allocation tracking scopes cannot be nested"
        );
    });
    ALLOCATIONS.with(|count| count.set(0));

    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TRACKING.with(|tracking| tracking.set(false));
        }
    }

    let reset = Reset;
    let value = operation();
    let allocations = ALLOCATIONS.with(Cell::get);
    drop(reset);
    assert_eq!(allocations, 0, "operation allocated {allocations} time(s)");
    value
}
