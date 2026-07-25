//! Handle-based shared arena for zero-copy payload exchange.
//!
//! A [`Handle`] is a region in shared memory — the payload unit that crosses the
//! ring instead of bytes. On native, "shared memory" is an [`Arena`] slot that
//! the caller and worker access in place by raw pointer (one address space);
//! on web, payloads still cross as bytes (separate wasm memories make true
//! zero-copy impossible there). The ring carries only the 16-byte `Handle`.
//!
//! `Handle` is `Serialize`/`Deserialize` (postcard) — it is a normal arg, not a
//! new wire format. Native bulk methods take/return `Handle`; web methods take/
//! return bytes. Both go through the same `Transport::call`.
//!
//! # Ownership protocol
//!
//! A slot is exclusively leased to exactly one side at a time, enforced by an
//! atomic lease state + generation counter (not the borrow checker — handles are
//! `Copy` because they cross the ring). The lease state machine:
//!
//! ```text
//! Free --acquire--> WriteLeased --handoff--> ReadLeased --read--> Reading --drop--> Free
//! ```
//!
//! [`WriteGuard`] is the RAII write lease; [`WriteGuard::handoff`] consumes it
//! and produces a `Handle` for the reader. [`ReadGuard`] is the RAII read
//! lease; dropping it frees the slot. A stale `generation` (slot reused after
//! release) is rejected, never silently aliased.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::Arc;
use std::cell::UnsafeCell;

use crate::Handle;

/// Per-slot lease state for the ownership state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LeaseState {
    /// Free, available for `acquire`.
    Free = 0,
    /// Leased to a writer ([`WriteGuard`] outstanding).
    WriteLeased = 1,
    /// Writer handed off; a `Handle` is in flight to the reader.
    ReadLeased = 2,
    /// A [`ReadGuard`] or external V8 view is outstanding.
    Reading = 3,
}

impl LeaseState {
    const fn from_u8(v: u8) -> Self {
        match v {
            1 => LeaseState::WriteLeased,
            2 => LeaseState::ReadLeased,
            3 => LeaseState::Reading,
            _ => LeaseState::Free,
        }
    }
}

type Lease = LeaseState;

/// One arena slot: a fixed-size byte region + lease state + generation.
struct Slot {
    state: AtomicU8,
    generation: AtomicU32,
}

impl Slot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(Lease::Free as u8),
            generation: AtomicU32::new(0),
        }
    }
    fn lease(&self) -> Lease {
        Lease::from_u8(self.state.load(Ordering::Acquire))
    }
    fn lease_generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }
}

/// A fixed-capacity, generational-slot shared arena for zero-copy payload
/// exchange between a caller thread and a worker thread.
///
/// Slots are a fixed `slot_size` (sized to the largest bulk payload, e.g. a VT
/// page). [`Arena::acquire`] returns a [`WriteGuard`] (exclusive write lease);
/// [`WriteGuard::handoff`] produces a [`Handle`] for the reader; [`Arena::read`]
/// returns a [`ReadGuard`] (exclusive read lease). Guards are RAII — slots are
/// never double-leased; a stale `generation` is rejected.
///
/// `Send + Sync`: the data area is `UnsafeCell`-backed; mutation happens only
/// under the lease state machine, which guarantees exclusive access per slot.
pub struct Arena {
    region: u32,
    slot_size: usize,
    slots: Vec<Slot>,
    data: UnsafeCell<Box<[u8]>>,
}

// SAFETY: `data` is `UnsafeCell`; access is mediated by the per-slot atomic
// lease state, which guarantees exclusive access to each slot's byte range.
// `region`/`slot_size`/`slots` are immutable after construction.
unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

impl Arena {
    /// Construct an arena of `slot_count` slots, each `slot_size` bytes, under
    /// arena instance id `region`. Total footprint is `slot_count * slot_size`.
    pub fn new(region: u32, slot_count: usize, slot_size: usize) -> Arc<Self> {
        assert!(slot_count >= 1 && slot_size >= 1, "arena must be non-empty");
        let total = slot_count.checked_mul(slot_size).expect("arena size overflow");
        let slots = (0..slot_count).map(|_| Slot::new()).collect::<Vec<_>>();
        Arc::new(Self {
            region,
            slot_size,
            slots,
            data: UnsafeCell::new(vec![0u8; total].into_boxed_slice()),
        })
    }

    /// Slot size in bytes.
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Number of slots.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Current lease state of a slot (diagnostic / test access).
    pub fn slot_state(&self, slot: usize) -> LeaseState {
        self.slots
            .get(slot)
            .map(|s| LeaseState::from_u8(s.state.load(Ordering::Acquire)))
            .unwrap_or(LeaseState::Free)
    }

    /// Acquire any free slot for writing. Returns `None` if all slots are
    /// leased. The guard grants exclusive write access until `handoff` or drop.
    pub fn acquire(self: &Arc<Self>) -> Option<WriteGuard<'_>> {
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.lease() != Lease::Free {
                continue;
            }
            let generation = slot.lease_generation();
            if slot
                .state
                .compare_exchange(
                    Lease::Free as u8,
                    Lease::WriteLeased as u8,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Some(WriteGuard {
                    arena: self,
                    slot: i as u32,
                    generation: generation,
                });
            }
        }
        None
    }

    /// Read a handed-off slot. Returns `None` if the slot is not `ReadLeased`,
    /// the generation is stale, or a read is already outstanding. The guard
    /// grants exclusive read access until drop (which frees the slot).
    pub fn read(self: &Arc<Self>, handle: Handle) -> Option<ReadGuard<'_>> {
        let i = handle.slot as usize;
        if i >= self.slots.len() || handle.region != self.region {
            return None;
        }
        let slot = &self.slots[i];
        if slot.lease_generation() != handle.generation {
            return None; // stale handle into a reused slot
        }
        if slot.lease() != Lease::ReadLeased {
            return None;
        }
        if slot
            .state
            .compare_exchange(
                Lease::ReadLeased as u8,
                Lease::Reading as u8,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            Some(ReadGuard {
                arena: self,
                slot: handle.slot,
                generation: handle.generation,
                length: handle.length,
            })
        } else {
            None
        }
    }

    /// Lease a handed-off slot for an external reader (e.g. a V8 `ArrayBuffer`
    /// backed by the slot's memory). Like [`read`](Self::read) but returns the
    /// raw slot pointer + length instead of a Rust guard; the caller must later
    /// call [`release_read`](Self::release_read) (typically from the backing
    /// store's deleter) to free the slot. Returns `None` if the slot is not
    /// `ReadLeased`, the generation is stale, or a read is already outstanding.
    pub fn lease_read(&self, handle: Handle) -> Option<(*const u8, usize)> {
        let i = handle.slot as usize;
        if i >= self.slots.len() || handle.region != self.region {
            return None;
        }
        let slot = &self.slots[i];
        if slot.lease_generation() != handle.generation {
            return None;
        }
        if slot.lease() != Lease::ReadLeased {
            return None;
        }
        if slot
            .state
            .compare_exchange(
                Lease::ReadLeased as u8,
                Lease::Reading as u8,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            // SAFETY: the slot is now Reading (exclusively leased to this
            // reader); `handle.length` was clamped to `slot_size` at handoff.
            let ptr = unsafe { self.slot_ptr(handle.slot) };
            Some((ptr, handle.length as usize))
        } else {
            None
        }
    }

    /// Release a slot leased via [`lease_read`](Self::lease_read). Transitions
    /// `Reading` -> `Free` and advances the generation. Idempotent-ish: a no-op
    /// if the slot is no longer `Reading` (e.g. already released).
    pub fn release_read(&self, handle: Handle) {
        self.finish_read(handle.slot, handle.generation);
    }

    /// # Safety (internal)
    /// `slot` must be a valid index; the caller must hold an exclusive lease
    /// for this slot (guaranteed by the guard that calls this).
    unsafe fn slot_ptr(&self, slot: u32) -> *mut u8 {
        // SAFETY: the guard holds the exclusive WriteLeased/Reading lease for
        // `slot`; `data` is `UnsafeCell`-backed and accessed only under that lease.
        let base: *const u8 = unsafe { (*self.data.get()).as_ptr() };
        // SAFETY: caller guarantees `slot` is valid and exclusively leased.
        unsafe { base.add(slot as usize * self.slot_size) as *mut u8 }
    }

    /// # Safety (internal)
    /// Called by [`WriteGuard::drop`] when the guard is dropped without
    /// `handoff` — the slot returns to `Free` and the generation advances so a
    /// stale handle is rejected.
    fn cancel_write(&self, slot: u32, generation: u32) {
        let s = &self.slots[slot as usize];
        // Only transition if still WriteLeased (a concurrent handoff can't run —
        // the guard owns the lease exclusively).
        let _ = s.state.compare_exchange(
            Lease::WriteLeased as u8,
            Lease::Free as u8,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        // Advance generation so any handle captured before a cancelled write is
        // rejected on read.
        s.generation.store(generation.wrapping_add(1), Ordering::Release);
    }

    /// Called by [`ReadGuard::drop`] — the slot returns to `Free` and the
    /// generation advances.
    fn finish_read(&self, slot: u32, generation: u32) {
        let s = &self.slots[slot as usize];
        let _ = s.state.compare_exchange(
            Lease::Reading as u8,
            Lease::Free as u8,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        s.generation.store(generation.wrapping_add(1), Ordering::Release);
    }
}

/// RAII write lease on an arena slot. Grants exclusive `&mut [u8]` access until
/// [`handoff`](Self::handoff) (produces a `Handle` for the reader) or drop
/// (cancels the lease).
pub struct WriteGuard<'a> {
    arena: &'a Arena,
    slot: u32,
    generation: u32,
}

impl WriteGuard<'_> {
    /// The slot index this guard leases.
    pub fn slot(&self) -> u32 {
        self.slot
    }
    /// The lease generation.
    pub fn generation(&self) -> u32 {
        self.generation
    }
    /// The writable byte region for this slot.
    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: the guard holds the exclusive WriteLeased lease for `slot`.
        let ptr = unsafe { self.arena.slot_ptr(self.slot) };
        // SAFETY: exclusive access; slot_size is the valid range.
        unsafe { std::slice::from_raw_parts_mut(ptr, self.arena.slot_size) }
    }

    /// Hand the slot off to a reader. Consumes the write lease and produces a
    /// [`Handle`] carrying `valid_len` (the bytes the caller wrote, ≤ slot size).
    /// The slot transitions to `ReadLeased`; [`Arena::read`] with the returned
    /// handle grants the reader exclusive access.
    pub fn handoff(self, valid_len: u32) -> Handle {
        let len = valid_len.min(self.arena.slot_size as u32);
        let s = &self.arena.slots[self.slot as usize];
        // Transition WriteLeased -> ReadLeased. Exclusive (we hold the lease).
        let prev = s.state.compare_exchange(
            Lease::WriteLeased as u8,
            Lease::ReadLeased as u8,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        debug_assert!(
            prev.is_ok(),
            "WriteGuard::handoff on a slot not WriteLeased (lease corruption)"
        );
        let handle = Handle {
            region: self.arena.region,
            slot: self.slot,
            length: len,
            generation: self.generation,
        };
        // Prevent the Drop from cancelling the lease.
        std::mem::forget(self);
        handle
    }
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        // Dropped without `handoff`: cancel the lease, advance the generation.
        self.arena.cancel_write(self.slot, self.generation);
    }
}

/// RAII read lease on an arena slot. Grants exclusive `&[u8]` access (the
/// `valid_len` bytes the writer handed off) until drop, which frees the slot.
pub struct ReadGuard<'a> {
    arena: &'a Arena,
    slot: u32,
    generation: u32,
    length: u32,
}

impl ReadGuard<'_> {
    /// The readable bytes for this lease (`valid_len` from `handoff`).
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the guard holds the exclusive Reading lease for `slot`.
        let ptr = unsafe { self.arena.slot_ptr(self.slot) };
        // SAFETY: exclusive access; `length` ≤ slot_size (clamped at handoff).
        unsafe { std::slice::from_raw_parts(ptr, self.length as usize) }
    }
}

impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        self.arena.finish_read(self.slot, self.generation);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn handle_is_postcard_round_trippable() {
        let h = Handle { region: 7, slot: 3, length: 64, generation: 99 };
        let bytes = crate::encode(&h).unwrap();
        let back: Handle = crate::decode(&bytes).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn write_handoff_read_round_trip_is_zero_copy() {
        let arena = Arena::new(0, 2, 128);
        let mut w = arena.acquire().unwrap();
        let payload = b"hello arena";
        w.bytes()[..payload.len()].copy_from_slice(payload);
        let handle = w.handoff(payload.len() as u32);

        let r = arena.read(handle).unwrap();
        assert_eq!(&r.bytes()[..payload.len()], payload);
        drop(r);

        // Slot is free again; re-acquire reuses it with a new generation.
        let w2 = arena.acquire().unwrap();
        assert_eq!(w2.slot, handle.slot);
    }

    #[test]
    fn dropped_write_guard_cancels_and_advances_generation() {
        let arena = Arena::new(0, 1, 64);
        let w = arena.acquire().unwrap();
        let slot = w.slot;
        let generation = w.generation;
        drop(w); // cancelled, not handed off

        // The slot is free; generation advanced.
        let w2 = arena.acquire().unwrap();
        assert_eq!(w2.slot, slot);
        assert_eq!(w2.generation, generation.wrapping_add(1));
    }

    #[test]
    fn stale_handle_rejected_after_reuse() {
        let arena = Arena::new(0, 1, 64);
        let mut w = arena.acquire().unwrap();
        let handle = w.handoff(4);
        // Read + release (slot freed, generation advances).
        let r = arena.read(handle).unwrap();
        drop(r);

        // Re-acquire + handoff a new payload into the same slot.
        let mut w2 = arena.acquire().unwrap();
        w2.bytes()[..3].copy_from_slice(b"new");
        let new_handle = w2.handoff(3);
        assert_eq!(new_handle.slot, handle.slot);
        assert_ne!(new_handle.generation, handle.generation);

        // The OLD handle is stale and must be rejected, not aliased onto "new".
        assert!(arena.read(handle).is_none());
        // The new handle reads the new payload.
        let r = arena.read(new_handle).unwrap();
        assert_eq!(&r.bytes()[..3], b"new");
    }

    #[test]
    fn read_rejected_while_write_leased() {
        let arena = Arena::new(0, 1, 64);
        let w = arena.acquire().unwrap();
        // Slot is WriteLeased; no handle exists yet. A fabricated handle to a
        // WriteLeased slot is rejected.
        let fake = Handle { region: 0, slot: w.slot, length: 4, generation: w.generation };
        assert!(arena.read(fake).is_none());
    }

    #[test]
    fn double_read_rejected() {
        let arena = Arena::new(0, 1, 64);
        let mut w = arena.acquire().unwrap();
        let handle = w.handoff(4);
        let r1 = arena.read(handle).unwrap();
        // A second read of the same handle is rejected (already Reading).
        assert!(arena.read(handle).is_none());
        drop(r1);
        // After release, the stale handle is rejected (generation advanced).
        assert!(arena.read(handle).is_none());
    }

    #[test]
    fn wrong_region_rejected() {
        let arena = Arena::new(0, 1, 64);
        let mut w = arena.acquire().unwrap();
        let mut h = w.handoff(4);
        h.region = 999;
        assert!(arena.read(h).is_none());
    }

    #[test]
    fn acquire_returns_none_when_full() {
        let arena = Arena::new(0, 1, 64);
        let _w = arena.acquire().unwrap();
        assert!(arena.acquire().is_none());
    }

    #[test]
    fn arena_is_send_sync_across_threads() {
        // Proves `Arc<Arena>` can cross threads and a handed-off handle can be
        // read on another thread (the worker-thread pattern).
        let arena = Arena::new(0, 1, 256);
        let mut w = arena.acquire().unwrap();
        let payload = b"cross-thread payload";
        w.bytes()[..payload.len()].copy_from_slice(payload);
        let handle = w.handoff(payload.len() as u32);

        let arena2 = arena.clone();
        let t = std::thread::spawn(move || {
            let r = arena2.read(handle).unwrap();
            assert_eq!(&r.bytes()[..payload.len()], payload);
        });
        t.join().unwrap();
    }
}

/// A lock-free SPSC bounded queue of [`Handle`]s for worker↔worker comms
/// (e.g. asset loader → texture transcoder, physics → audio). One producer, one
/// consumer; direct Rust-to-Rust, no renderer hop, no copy (the payload lives
/// in the shared [`Arena`]; only the 16-byte `Handle` crosses).
///
/// `push` returns `Err(handle)` if full (the caller retries or back-pressure
/// drops); `pop` returns `None` if empty. Indices are monotonically wrapping
/// `u32`s; the queue is full when `tail - head == capacity` and empty when
/// `tail == head`. Native-only — web workers have separate memories and route
/// through the main thread.
pub struct HandleQueue {
    slots: Box<[UnsafeCell<Handle>]>,
    head: AtomicU32,
    tail: AtomicU32,
    capacity: u32,
}

unsafe impl Send for HandleQueue {}
unsafe impl Sync for HandleQueue {}

impl HandleQueue {
    /// Construct a queue holding up to `capacity` handles.
    pub fn new(capacity: usize) -> Arc<Self> {
        assert!(capacity >= 1, "HandleQueue capacity must be >= 1");
        let slots = (0..capacity)
            .map(|_| UnsafeCell::new(Handle {
                region: 0,
                slot: 0,
                length: 0,
                generation: 0,
            }))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Arc::new(Self {
            slots,
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            capacity: capacity as u32,
        })
    }

    /// Push a handle. Returns `Err(handle)` if the queue is full.
    pub fn push(&self, handle: Handle) -> Result<(), Handle> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= self.capacity {
            return Err(handle); // full
        }
        let slot = (tail % self.capacity) as usize;
        // SAFETY: SPSC producer; `slot` is not yet visible (tail not published)
        // and cannot alias the consumer, which only touches slots `< head`.
        unsafe {
            *self.slots[slot].get() = handle;
        }
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Pop a handle. Returns `None` if the queue is empty.
    pub fn pop(&self) -> Option<Handle> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None; // empty
        }
        let slot = (head % self.capacity) as usize;
        // SAFETY: SPSC consumer; `head` was published by the producer (Release on
        // tail), so the slot's bytes are visible and stable until we advance head.
        let handle = unsafe { *self.slots[slot].get() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(handle)
    }

    /// Number of handles currently enqueued.
    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        tail.wrapping_sub(head) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod handle_queue_tests {
    use super::*;

    #[test]
    fn push_pop_round_trip() {
        let q = HandleQueue::new(4);
        assert!(q.is_empty());
        q.push(Handle { region: 0, slot: 1, length: 8, generation: 0 }).unwrap();
        q.push(Handle { region: 0, slot: 2, length: 16, generation: 1 }).unwrap();
        assert_eq!(q.len(), 2);
        let h1 = q.pop().unwrap();
        let h2 = q.pop().unwrap();
        assert_eq!(h1.slot, 1);
        assert_eq!(h2.slot, 2);
        assert!(q.is_empty());
    }

    #[test]
    fn push_returns_err_when_full() {
        let q = HandleQueue::new(2);
        q.push(Handle { region: 0, slot: 0, length: 0, generation: 0 }).unwrap();
        q.push(Handle { region: 0, slot: 1, length: 0, generation: 0 }).unwrap();
        let overflow = Handle { region: 0, slot: 2, length: 0, generation: 0 };
        let err = q.push(overflow).unwrap_err();
        assert_eq!(err.slot, 2);
    }

    #[test]
    fn wraparound_stress() {
        let q = HandleQueue::new(4);
        let mut seq = 0u32;
        for _ in 0..1000 {
            // Fill to capacity, drain, repeat — exercises index wraparound.
            for _ in 0..4 {
                q.push(Handle { region: 0, slot: seq, length: 0, generation: seq })
                    .unwrap();
                seq = seq.wrapping_add(1);
            }
            for _ in 0..4 {
                assert!(q.pop().is_some());
            }
            assert!(q.is_empty());
        }
    }

    #[test]
    fn spsc_across_threads() {
        // One producer thread, one consumer thread; the consumer sees every
        // pushed handle exactly once (the worker↔worker pattern).
        let q = HandleQueue::new(16);
        let qc = q.clone();
        let total = 2000u32;
        let producer = std::thread::spawn(move || {
            for i in 0..total {
                // Spin-push (back-pressure) — fine for the test.
                let mut h = Handle { region: 0, slot: i, length: 0, generation: i };
                while let Err(returned) = q.push(h) {
                    h = returned;
                    std::hint::spin_loop();
                }
            }
        });
        let consumer = std::thread::spawn(move || {
            let mut seen = 0u32;
            while seen < total {
                if let Some(h) = qc.pop() {
                    assert_eq!(h.slot, seen);
                    assert_eq!(h.generation, seen);
                    seen += 1;
                } else {
                    std::hint::spin_loop();
                }
            }
            seen
        });
        producer.join().unwrap();
        let seen = consumer.join().unwrap();
        assert_eq!(seen, total);
    }
}

/// A generic, thread-safe, generational slot map. Workers insert resources
/// (textures, buffers, ...); the renderer looks them up by handle. A stale
/// `SlotHandle` (slot reused after `remove`/`take`) is rejected, never silently
/// aliased. Native-only — the GPU resource table is the primary user; web
/// crosses bytes and has no shared GPU device.
pub struct SlotMap<T> {
    slots: std::sync::Mutex<Vec<Entry<T>>>,
    free_list: std::sync::Mutex<Vec<u32>>,
}

struct Entry<T> {
    generation: u32,
    value: Option<T>,
}

/// A handle into a [`SlotMap`]: an index + the generation it was created at.
/// `Serialize`/`Deserialize` so it can cross the ring like a `Handle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SlotHandle {
    pub id: u32,
    pub generation: u32,
}

impl<T: Send + 'static> SlotMap<T> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            slots: std::sync::Mutex::new(Vec::new()),
            free_list: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Insert a value; returns a handle carrying the slot's current generation.
    pub fn insert(&self, value: T) -> SlotHandle {
        let mut free = self.free_list.lock().unwrap();
        if let Some(id) = free.pop() {
            let mut slots = self.slots.lock().unwrap();
            let slot = &mut slots[id as usize];
            slot.generation = slot.generation.wrapping_add(1);
            slot.value = Some(value);
            SlotHandle { id, generation: slot.generation }
        } else {
            let mut slots = self.slots.lock().unwrap();
            let id = slots.len() as u32;
            slots.push(Entry { generation: 0, value: Some(value) });
            SlotHandle { id, generation: 0 }
        }
    }

    /// Take the value out of a slot if the handle's generation is current.
    /// Returns `None` for a stale handle or an already-taken slot.
    pub fn take(&self, handle: SlotHandle) -> Option<T> {
        let mut slots = self.slots.lock().unwrap();
        let slot = slots.get_mut(handle.id as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.value.take().inspect(|_| {
            // Generation advances so a stale handle (same id+gen) is rejected;
            // the slot returns to the free list for reuse.
            slot.generation = slot.generation.wrapping_add(1);
            self.free_list.lock().unwrap().push(handle.id);
        })
    }

    /// Remove a value by handle (drops it). Returns whether the handle was
    /// current (i.e. a value was removed).
    pub fn remove(&self, handle: SlotHandle) -> bool {
        self.take(handle).is_some()
    }
}

impl<T: Send + 'static + Clone> SlotMap<T> {
    /// Clone the value at a handle without removing it. Returns `None` for a
    /// stale or empty slot.
    pub fn get_cloned(&self, handle: SlotHandle) -> Option<T> {
        let slots = self.slots.lock().unwrap();
        let slot = slots.get(handle.id as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.value.clone()
    }
}

impl<T: Send + 'static> Default for SlotMap<T> {
    fn default() -> Self {
        Self {
            slots: std::sync::Mutex::new(Vec::new()),
            free_list: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod slot_map_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn insert_take_round_trip() {
        let map: Arc<SlotMap<u32>> = SlotMap::new();
        let h0 = map.insert(10);
        let h1 = map.insert(20);
        assert_eq!(map.take(h0), Some(10));
        assert_eq!(map.take(h1), Some(20));
        // Already taken.
        assert_eq!(map.take(h0), None);
    }

    #[test]
    fn stale_handle_rejected_after_reuse() {
        let map: Arc<SlotMap<u32>> = SlotMap::new();
        let h = map.insert(1);
        assert!(map.remove(h));
        // Slot reused for a new value with an advanced generation.
        let h2 = map.insert(2);
        assert_eq!(h2.id, h.id);
        assert_ne!(h2.generation, h.generation);
        // The old handle is stale.
        assert_eq!(map.take(h), None);
        assert_eq!(map.take(h2), Some(2));
    }

    #[test]
    fn get_cloned_does_not_consume() {
        let map: Arc<SlotMap<u32>> = SlotMap::new();
        let h = map.insert(99);
        assert_eq!(map.get_cloned(h), Some(99));
        assert_eq!(map.get_cloned(h), Some(99)); // still there
        assert_eq!(map.take(h), Some(99));
    }

    #[test]
    fn cross_thread_insert_take() {
        let map: Arc<SlotMap<u32>> = SlotMap::new();
        let m = map.clone();
        let producer = std::thread::spawn(move || {
            (0..100u32).map(|i| m.insert(i)).collect::<Vec<_>>()
        });
        let handles = producer.join().unwrap();
        let m = map.clone();
        let consumer = std::thread::spawn(move || {
            handles.into_iter().map(|h| m.take(h)).collect::<Vec<_>>()
        });
        let taken = consumer.join().unwrap();
        assert_eq!(taken.len(), 100);
        assert!(taken.into_iter().all(|v| v.is_some()));
    }
}
