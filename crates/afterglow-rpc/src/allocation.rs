//! Opt-in allocation tracking for sealed native hot paths.
//!
//! A binary installs [`TrackingAllocator`] as its global allocator, then wraps
//! deterministic hot-path tests with [`assert_no_alloc`]. Tracking is
//! thread-local so unrelated test threads do not create false positives.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

/// Transparent global-allocator wrapper with opt-in thread-local counting.
pub struct TrackingAllocator<A> {
    inner: A,
}

impl<A> TrackingAllocator<A> {
    /// Wrap an allocator. This is `const` so it can initialize a global static.
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

// SAFETY: every operation delegates to `inner` with the original pointer and
// layout. The wrapper only updates thread-local counters before delegation.
unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: upheld by the caller of GlobalAlloc::alloc.
        unsafe { self.inner.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: upheld by the caller of GlobalAlloc::alloc_zeroed.
        unsafe { self.inner.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: upheld by the caller of GlobalAlloc::realloc.
        unsafe { self.inner.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: upheld by the caller of GlobalAlloc::dealloc.
        unsafe { self.inner.dealloc(ptr, layout) }
    }
}

/// Run `operation` and panic if it allocates on the current thread.
///
/// Tracking must not be nested. The operation's return value is preserved.
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
    assert_eq!(
        allocations, 0,
        "sealed operation allocated {allocations} time(s)"
    );
    value
}

#[cfg(test)]
mod tests {
    use std::alloc::System;

    use super::{TrackingAllocator, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: TrackingAllocator<System> = TrackingAllocator::new(System);

    #[test]
    fn accepts_stack_only_work_and_preserves_the_result() {
        let result = assert_no_alloc(|| {
            let mut values = [1_u32, 2, 3, 4];
            values[2] = 9;
            values.iter().copied().sum::<u32>()
        });
        assert_eq!(result, 16);
    }

    #[test]
    #[should_panic(expected = "sealed operation allocated")]
    fn detects_heap_allocation() {
        assert_no_alloc(|| {
            let value = Box::new(7_u64);
            std::hint::black_box(value);
        });
    }
}
