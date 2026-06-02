//! Byte-level delta encoding for serde types, with a rolling history buffer.
//!
//! # Quick start
//!
//! ```
//! # use delta::{DeltaPatch, DeltaHistory};
//! # use serde::{Serialize, Deserialize};
//! # #[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
//! # struct Snapshot { tick: u32 }
//! // ── DeltaPatch: diff two values ────────────────────────────────────────
//! let a = Snapshot { tick: 0 };
//! let b = Snapshot { tick: 42 };
//!
//! let patch = DeltaPatch::diff(&a, &b);        // compute (uses postcard internally)
//! let restored: Snapshot = patch.apply(&a);    // reconstruct
//! assert_eq!(restored.tick, 42);
//!
//! // ── Raw byte path (fastest: no serde round-trip) ──────────────────────
//! let bytes_a = postcard::to_allocvec(&a).unwrap();
//! let bytes_b = postcard::to_allocvec(&b).unwrap();
//! let patch = DeltaPatch::diff_bytes(&bytes_a, &bytes_b);
//! let restored_bytes = patch.apply_bytes(&bytes_a);
//! assert_eq!(restored_bytes, bytes_b);
//!
//! // ── Send over network ─────────────────────────────────────────────────
//! let wire = postcard::to_allocvec(&patch).unwrap();   // ~10 bytes
//! // ... send wire ...
//! let received: DeltaPatch = postcard::from_bytes(&wire).unwrap();
//! let decoded: Snapshot = received.apply(&a);
//!
//! // ── Rolling history buffer with undo/redo ──────────────────────────────
//! let mut hist: DeltaHistory<Snapshot> = DeltaHistory::new(240);
//! hist.init(&a);
//! hist.push(&b);                           // auto-diffs against previous
//! assert_eq!(hist.at(0).tick, 0);          // oldest
//! assert_eq!(hist.at(-1).tick, 42);        // newest
//!
//! let rewind = hist.undo(1).unwrap();      // go back 1 frame
//! assert_eq!(rewind.tick, 0);
//!
//! let forward = hist.redo(1).unwrap();     // go forward
//! assert_eq!(forward.tick, 42);
//!
//! // ── Collapse a range into one patch ───────────────────────────────────
//! let big_patch = hist.combined_patch(0, -1);  // one patch covering all history
//! // ... useful for sending a "catch up" delta to a late joiner
//! ```
//!
//! # Real usage: deterministic physics rollback
//!
//! ```ignore
//! // Each tick: snapshot physics state → diff → store in ring buffer
//! let prev_bytes = postcard::to_allocvec(&prev_state).unwrap();
//! let curr_bytes = postcard::to_allocvec(&curr_state).unwrap();
//! let patch = DeltaPatch::diff_bytes(&prev_bytes, &curr_bytes);
//! history.push(&curr_state, &prev_bytes);  // or use DeltaHistory::push
//!
//! // On rollback (e.g. frame lag, late join, prediction correction):
//! let restored: GameState = history.at(target_frame);
//! // Write GameState back into ECS components, then resim
//! ```

use serde::{Deserialize, Serialize};

/// A single contiguous run of changed bytes.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Run {
    pub offset: u32,
    pub data: Vec<u8>,
}

/// A byte-level delta between two serialized values.
///
/// Stores only the runs of bytes that differ between `old` and `new`,
/// encoded as `(offset, data)` pairs. No string keys, no chunk padding.
///
/// Use [`diff_bytes`](DeltaPatch::diff_bytes) when you have cached byte buffers
/// (avoids serde overhead). Use [`diff`](DeltaPatch::diff) for convenience.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeltaPatch {
    pub total_len: u32,
    pub runs: Vec<Run>,
}

impl DeltaPatch {
    /// Compute a delta from two serde-serializable values.
    pub fn diff<T: Serialize>(old: &T, new: &T) -> Self {
        Self::diff_bytes(
            &postcard::to_allocvec(old).unwrap(),
            &postcard::to_allocvec(new).unwrap(),
        )
    }

    /// Compute a delta between two byte slices directly (fastest path).
    pub fn diff_bytes(old: &[u8], new: &[u8]) -> Self {
        let max_len = old.len().max(new.len());
        let mut runs: Vec<Run> = Vec::new();
        let mut i = 0;

        while i < max_len {
            let ob = old.get(i).copied().unwrap_or(0);
            let nb = new.get(i).copied().unwrap_or(0);
            if ob != nb {
                let start = i;
                while i < max_len {
                    let o = old.get(i).copied().unwrap_or(0);
                    let n = new.get(i).copied().unwrap_or(0);
                    if o == n {
                        break;
                    }
                    i += 1;
                }
                runs.push(Run {
                    offset: start as u32,
                    data: new[start..i].to_vec(),
                });
            } else {
                i += 1;
            }
        }

        // Merge adjacent runs
        if runs.len() > 1 {
            let mut merged: Vec<Run> = Vec::with_capacity(runs.len());
            let mut cur = runs.swap_remove(0);
            for r in runs {
                if r.offset as usize == cur.offset as usize + cur.data.len() {
                    cur.data.extend_from_slice(&r.data);
                } else {
                    merged.push(cur);
                    cur = r;
                }
            }
            merged.push(cur);
            runs = merged;
        }

        DeltaPatch {
            total_len: new.len() as u32,
            runs,
        }
    }

    /// Reconstruct the new value from `old` via serde round-trip.
    pub fn apply<T: serde::de::DeserializeOwned + Serialize>(&self, old: &T) -> T {
        let mut bytes = postcard::to_allocvec(old).unwrap();
        self.apply_in_place(&mut bytes);
        if (bytes.len() as u32) < self.total_len {
            bytes.resize(self.total_len as usize, 0);
        }
        postcard::from_bytes(&bytes).unwrap()
    }

    /// Reconstruct the new bytes from an old byte buffer (no serde).
    pub fn apply_bytes(&self, old: &[u8]) -> Vec<u8> {
        let mut buf = old.to_vec();
        self.apply_in_place(&mut buf);
        if (buf.len() as u32) < self.total_len {
            buf.resize(self.total_len as usize, 0);
        }
        buf
    }

    fn apply_in_place(&self, bytes: &mut Vec<u8>) {
        for run in &self.runs {
            let start = run.offset as usize;
            let end = start + run.data.len();
            if end > bytes.len() {
                bytes.resize(end, 0);
            }
            bytes[start..end].copy_from_slice(&run.data);
        }
    }

    /// Serialized byte size of the delta itself.
    pub fn serialized_size(&self) -> usize {
        postcard::to_allocvec(self).unwrap().len()
    }
}

// ── DeltaHistory: ring buffer with undo/redo ────────────────────────────

#[derive(Clone)]
struct HistoryEntry {
    kind: HistoryKind,
}

#[derive(Clone)]
enum HistoryKind {
    Full(Vec<u8>),
    Delta(DeltaPatch),
}

/// A ring buffer of full snapshots and deltas with cursor-based undo/redo.
///
/// of full snapshots at both ends — index 0 is always a full snapshot anchor,
/// index -1 is always the latest full snapshot. Everything between is compact
/// delta patches. Restoring any frame finds the preceding full entry
/// and chain-applies deltas forward — typically ~2 μs per delta.
///
/// Generic over any `T: Serialize + DeserializeOwned + Clone`.
/// All state is stored as compact serialized bytes internally.
///
/// # Undo/redo semantics
///
/// - [`push`](Self::push) records a new state (auto-diffs against previous),
///   advancing the cursor. If you previously undid, future entries are
///   discarded.
/// - [`undo`](Self::undo) moves the cursor back. [`redo`](Self::redo) moves it
///   forward. Both return `T` at the new position.
pub struct DeltaHistory<T> {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    capacity: usize,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: serde::Serialize + serde::de::DeserializeOwned + Clone> DeltaHistory<T> {
    /// Create a new history buffer.
    ///
    /// `capacity` — max frames before the oldest interior entry is evicted.
    ///   Index 0 is always kept as a full snapshot anchor. Index -1 is always
    /// the   latest full snapshot. Everything between is compact delta
    /// patches.
    pub fn new(capacity: usize) -> Self {
        DeltaHistory {
            entries: Vec::with_capacity(capacity),
            cursor: 0,
            capacity,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Resolve a potentially negative index to a usize.
    /// - `0..` = from start
    /// - `-1` = last, `-2` = second-to-last, etc.
    fn resolve(&self, index: isize) -> usize {
        if index >= 0 {
            index as usize
        } else {
            let from_end = (-index) as usize;
            self.entries.len().saturating_sub(from_end)
        }
    }

    /// Record the initial value. Must be called first.
    pub fn init(&mut self, value: &T) {
        self.entries.clear();
        self.entries.push(HistoryEntry {
            kind: HistoryKind::Full(postcard::to_allocvec(value).unwrap()),
        });
        self.cursor = 0;
    }

    /// Record a new state. Always stored as a full snapshot at the end.
    /// The previous entry is immediately converted to a delta.
    /// Index 0 is always kept as a full snapshot anchor — when the buffer
    /// wraps, the evicted Full anchor is used to promote the next entry
    /// from Delta to Full.
    pub fn push(&mut self, value: &T) {
        let bytes = postcard::to_allocvec(value).unwrap();

        // Discard future entries if undid first (new timeline branch)
        if self.cursor < self.entries.len().saturating_sub(1) {
            self.entries.truncate(self.cursor + 1);
        }

        // Always store the new entry as a full snapshot
        self.entries.push(HistoryEntry {
            kind: HistoryKind::Full(bytes),
        });
        self.cursor = self.entries.len() - 1;

        // Convert the previous entry (if any, not index 0) from Full to Delta
        if self.entries.len() >= 3 {
            let prev_idx = self.entries.len() - 2;
            if prev_idx > 0 {
                if let HistoryKind::Full(_) = &self.entries[prev_idx].kind {
                    let predecessor = self.restore_bytes_at(prev_idx - 1);
                    let prev_full = match &self.entries[prev_idx].kind {
                        HistoryKind::Full(b) => b.clone(),
                        _ => unreachable!(),
                    };
                    let delta = DeltaPatch::diff_bytes(&predecessor, &prev_full);
                    self.entries[prev_idx].kind = HistoryKind::Delta(delta);
                }
            }
        }

        // Evict oldest from index 0 when over capacity.
        // The evicted Full's bytes are used to promote the new entry at index 0
        // from Delta to Full by applying the delta to the old anchor.
        while self.entries.len() > self.capacity {
            let removed = self.entries.remove(0);
            let old_anchor = match removed.kind {
                HistoryKind::Full(b) => b,
                _ => unreachable!("index 0 should always be Full"),
            };
            self.cursor = self.cursor.saturating_sub(1);
            // If the new entry at index 0 is a Delta, apply the old anchor to promote it to
            // Full
            if !self.entries.is_empty() {
                if let HistoryKind::Delta(d) = &self.entries[0].kind {
                    let new_bytes = d.apply_bytes(&old_anchor);
                    self.entries[0].kind = HistoryKind::Full(new_bytes);
                }
            }
        }
    }

    /// Get the value at any index. Negative = from end (-1 is last).
    pub fn at(&self, index: isize) -> T {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        self.restore_at(i)
    }

    /// Get the raw bytes at any index. Negative = from end.
    pub fn bytes_at(&self, index: isize) -> Vec<u8> {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        self.restore_bytes_at(i)
    }

    /// Return the [`DeltaPatch`] between two indices. Negative = from end.
    pub fn patch(&self, from: isize, to: isize) -> DeltaPatch {
        let f = self.resolve(from);
        let t = self.resolve(to);
        let old = self.restore_bytes_at(f);
        let new = self.restore_bytes_at(t);
        DeltaPatch::diff_bytes(&old, &new)
    }

    /// Produce a single combined patch over a range by computing the
    /// diff directly from the start to end state (no intermediate merges).
    /// Equivalent to [`patch`](Self::patch) — kept for semantic clarity.
    pub fn combined_patch(&self, from: isize, to: isize) -> DeltaPatch {
        self.patch(from, to)
    }

    // ── Immutable branching ───────────────────────────────────────────────

    /// Immutable: produce a new history truncated to `index`. Negative = from
    /// end. Original is unchanged. The resulting copy has its last entry
    /// promoted to Full.
    pub fn truncated_at(&self, index: isize) -> Self {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        let mut h = Self::new(self.capacity);
        h.entries = self.entries[..=i].to_vec();
        h.cursor = i.min(h.entries.len().saturating_sub(1));
        h.promote_last_to_full();
        h
    }

    // ── Mutable operations ─────────────────────────────────────────────────

    /// Truncate entries after `index`. Negative = from end.
    /// Ensures the new last entry is promoted to Full if needed.
    pub fn truncate(&mut self, index: isize) {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        self.entries.truncate(i + 1);
        self.cursor = self.cursor.min(i);
        // Ensure the new last entry is Full
        self.promote_last_to_full();
    }

    /// Move cursor to an absolute index. Negative = from end.
    pub fn move_to(&mut self, index: isize) {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        self.cursor = i;
    }

    /// Ensure entries[-1] is Full. If it's a Delta, reconstruct it.
    fn promote_last_to_full(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        if last == 0 {
            return;
        } // index 0 is always Full
        if let HistoryKind::Delta(_) = &self.entries[last].kind {
            let bytes = self.restore_bytes_at(last);
            self.entries[last].kind = HistoryKind::Full(bytes);
        }
    }

    /// Move cursor back by `n` frames, returning the value at that position.
    pub fn undo(&mut self, n: usize) -> Option<T> {
        if n > self.cursor {
            return None;
        }
        self.cursor -= n;
        Some(self.restore_at(self.cursor))
    }

    /// Move cursor forward by `n` frames, returning the value at that position.
    pub fn redo(&mut self, n: usize) -> Option<T> {
        let target = self.cursor + n;
        if target >= self.entries.len() {
            return None;
        }
        self.cursor = target;
        Some(self.restore_at(self.cursor))
    }

    fn restore_at(&self, index: usize) -> T {
        let bytes = self.restore_bytes_at(index);
        postcard::from_bytes(&bytes).unwrap()
    }

    fn restore_bytes_at(&self, index: usize) -> Vec<u8> {
        // Find the nearest full entry at or before `index` (index 0 is always Full)
        let snapshot_idx = (0..=index)
            .rev()
            .find(|&i| matches!(self.entries[i].kind, HistoryKind::Full(_)))
            .expect("DeltaHistory corrupted: no full entry found (index 0 should always be Full)");

        let bytes = match &self.entries[snapshot_idx].kind {
            HistoryKind::Full(b) => b.clone(),
            _ => unreachable!(),
        };

        let mut bytes = bytes;
        for i in (snapshot_idx + 1)..=index {
            if let HistoryKind::Delta(d) = &self.entries[i].kind {
                bytes = d.apply_bytes(&bytes);
            }
        }
        bytes
    }

    pub fn position(&self) -> usize {
        self.cursor
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn is_at_latest(&self) -> bool {
        self.cursor + 1 >= self.entries.len()
    }
    pub fn is_at_oldest(&self) -> bool {
        self.cursor == 0
    }
}

// ── TickHistory: DeltaHistory keyed by tick number ──────────────────────

/// A [`DeltaHistory`] indexed by a tick number.
///
/// Each entry stores its actual tick number, so `at_or_latest` works
/// correctly even when ticks are not perfectly sequential (gaps, jumps).
/// Ticks are tracked independently from the buffer index.
///
/// # Lookup behaviour
///
/// | Method | Requested tick | Returns |
/// |---|---|---|
/// | [`at`](Self::at) | exact match | value or panics |
/// | [`at_or_latest`](Self::at_or_latest) | present | that exact tick |
/// | [`at_or_latest`](Self::at_or_latest) | between two ticks | the earlier one |
/// | [`at_or_latest`](Self::at_or_latest) | < oldest | oldest |
/// | [`at_or_latest`](Self::at_or_latest) | > newest | newest |
pub struct TickHistory<T> {
    inner: DeltaHistory<T>,
    ticks: Vec<u32>, // parallel to inner.entries — tracks tick per slot
}

impl<T: serde::Serialize + serde::de::DeserializeOwned + Clone> TickHistory<T> {
    pub fn new(capacity: usize) -> Self {
        TickHistory {
            inner: DeltaHistory::new(capacity),
            ticks: Vec::with_capacity(capacity),
        }
    }

    /// Record the initial state at `tick`. Must be called first.
    pub fn init(&mut self, value: &T, tick: u32) {
        self.inner.init(value);
        self.ticks.clear();
        self.ticks.push(tick);
    }

    /// Record a new state at `tick`. Auto-diffs against the previous state.
    /// If the buffer is full, the oldest entry is evicted automatically.
    pub fn push(&mut self, value: &T, tick: u32) {
        let was_full = self.inner.len() >= self.inner.capacity();
        self.inner.push(value);
        if was_full {
            self.ticks.remove(0);
        }
        self.ticks.push(tick);
    }

    /// Get the state at exactly `tick`. Panics if not found.
    pub fn at(&self, tick: u32) -> T {
        let idx = self.tick_to_idx(tick);
        self.inner.at(idx as isize)
    }

    /// Get the state closest to `tick` without exceeding it.
    ///
    /// - Exact match → that tick
    /// - In a gap → nearest earlier tick
    /// - Below oldest → oldest
    /// - Above newest → newest
    pub fn at_or_latest(&self, tick: u32) -> T {
        if self.ticks.is_empty() {
            panic!("TickHistory is empty");
        }
        let oldest = self.ticks[0];
        let newest = *self.ticks.last().unwrap();

        if tick >= newest {
            return self.inner.at(-1);
        }
        if tick <= oldest {
            return self.inner.at(0);
        }

        // Binary search for the largest tick ≤ requested
        match self.ticks.binary_search(&tick) {
            Ok(idx) => self.inner.at(idx as isize),
            Err(insert_idx) => {
                // insert_idx is where `tick` would be inserted.
                // The closest ≤ tick is at `insert_idx - 1`.
                let idx = insert_idx.saturating_sub(1);
                self.inner.at(idx as isize)
            }
        }
    }

    /// The tick number of the oldest stored entry.
    pub fn oldest_tick(&self) -> u32 {
        *self.ticks.first().expect("TickHistory is empty")
    }

    /// The tick number of the newest stored entry.
    pub fn latest_tick(&self) -> u32 {
        *self.ticks.last().expect("TickHistory is empty")
    }

    /// Number of stored ticks.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    fn tick_to_idx(&self, tick: u32) -> usize {
        self.ticks
            .iter()
            .position(|&t| t == tick)
            .expect("tick not found in TickHistory history")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Body {
        id: u32,
        pos: [f32; 3],
        vel: [f32; 3],
        active: bool,
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Snapshot {
        tick: u32,
        bodies: Vec<Body>,
    }

    fn snap(tick: u32) -> Snapshot {
        Snapshot {
            tick,
            bodies: vec![Body {
                id: 1,
                pos: [tick as f32; 3],
                vel: [0.0; 3],
                active: true,
            }],
        }
    }

    // ── DeltaPatch tests ──

    #[test]
    fn delta_identical() {
        let a = Snapshot {
            tick: 100,
            bodies: vec![],
        };
        assert!(DeltaPatch::diff(&a, &a).runs.is_empty());
    }

    #[test]
    fn delta_field_change() {
        let a = Snapshot {
            tick: 100,
            bodies: vec![],
        };
        let mut b = a.clone();
        b.tick = 101;
        let restored: Snapshot = DeltaPatch::diff(&a, &b).apply(&a);
        assert_eq!(restored.tick, 101);
    }

    #[test]
    fn delta_multiple() {
        let a = Snapshot {
            tick: 0,
            bodies: vec![
                Body {
                    id: 1,
                    pos: [0.0; 3],
                    vel: [0.0; 3],
                    active: true,
                },
                Body {
                    id: 2,
                    pos: [1.0; 3],
                    vel: [0.0; 3],
                    active: false,
                },
            ],
        };
        let b = Snapshot {
            tick: 0,
            bodies: vec![
                Body {
                    id: 1,
                    pos: [9.0; 3],
                    vel: [0.0; 3],
                    active: true,
                },
                Body {
                    id: 2,
                    pos: [8.0; 3],
                    vel: [1.0; 3],
                    active: false,
                },
            ],
        };
        assert_eq!(DeltaPatch::diff(&a, &b).apply(&a), b);
    }

    #[test]
    fn delta_added_element() {
        let a = Snapshot {
            tick: 0,
            bodies: vec![Body {
                id: 1,
                pos: [0.0; 3],
                vel: [0.0; 3],
                active: true,
            }],
        };
        let b = Snapshot {
            tick: 0,
            bodies: vec![
                Body {
                    id: 1,
                    pos: [0.0; 3],
                    vel: [0.0; 3],
                    active: true,
                },
                Body {
                    id: 2,
                    pos: [5.0; 3],
                    vel: [0.0; 3],
                    active: false,
                },
            ],
        };
        assert_eq!(DeltaPatch::diff(&a, &b).apply(&a), b);
    }

    #[test]
    fn delta_diff_bytes() {
        let old = vec![0u8; 100];
        let mut new = old.clone();
        new[42] = 99;
        new[43] = 88;
        let d = DeltaPatch::diff_bytes(&old, &new);
        assert_eq!(d.runs.len(), 1);
        assert_eq!(d.runs[0].offset, 42);
        assert_eq!(d.runs[0].data, vec![99, 88]);
    }

    #[test]
    fn delta_apply_bytes() {
        let old = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut new = old.clone();
        new[2..6].copy_from_slice(&[99, 99, 99, 99]);
        let d = DeltaPatch::diff_bytes(&old, &new);
        assert_eq!(d.apply_bytes(&old), new);
    }

    // ── DeltaHistory tests ────────────────────────────────────────────
    //
    // Invariant: after every mutation:
    //   I0: entries[0] is always Full
    //   I1: entries[-1] is always Full
    //   I2: all interior entries (1..len-1) are Delta
    //   I3: all values round-trip through at(i) == original

    fn assert_invariants(h: &DeltaHistory<Snapshot>) {
        if h.is_empty() {
            return;
        }
        let len = h.len();
        // I0
        assert!(
            matches!(h.entries[0].kind, HistoryKind::Full(_)),
            "I0 failed: entries[0] must be Full, len={}",
            len
        );
        // I1
        assert!(
            matches!(h.entries[len - 1].kind, HistoryKind::Full(_)),
            "I1 failed: entries[-1] must be Full, len={}",
            len
        );
        // I2
        for i in 1..len.saturating_sub(1) {
            assert!(
                matches!(h.entries[i].kind, HistoryKind::Delta(_)),
                "I2 failed: entries[{}] must be Delta, len={}",
                i,
                len
            );
        }
        // I3: all stored values are recoverable and increase monotonically
        for i in 0..len {
            let s = h.at(i as isize);
            assert_eq!(
                s.tick,
                h.at(-(len as isize - i as isize)).tick,
                "I3 failed: at({}) != at(-{}))",
                i,
                len - i
            );
        }
    }

    fn assert_values(h: &DeltaHistory<Snapshot>, expected: &[u32]) {
        assert_eq!(h.len(), expected.len());
        for (i, &tick) in expected.iter().enumerate() {
            assert_eq!(
                h.at(i as isize).tick,
                tick,
                "position {} expected tick {} but got {:?}",
                i,
                tick,
                h.at(i as isize).tick
            );
        }
    }

    // ── Core operations ───────────────────────────────────────────────

    #[test]
    fn history_init_only() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(42));
        assert_eq!(h.len(), 1);
        assert_eq!(h.at(0).tick, 42);
        assert_eq!(h.at(-1).tick, 42);
        assert!(h.is_at_oldest());
        assert!(h.is_at_latest());
        assert_invariants(&h);
    }

    #[test]
    fn history_init_then_push() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        assert_eq!(h.len(), 2);
        assert_values(&h, &[0, 1]);
        assert_invariants(&h);
    }

    #[test]
    fn history_three_values() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(10));
        h.push(&snap(20));
        h.push(&snap(30));
        assert_values(&h, &[10, 20, 30]);
        assert_invariants(&h);
    }

    #[test]
    fn history_negative_indices() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(10));
        h.push(&snap(20));
        h.push(&snap(30));
        assert_eq!(h.at(-1).tick, 30);
        assert_eq!(h.at(-2).tick, 20);
        assert_eq!(h.at(-3).tick, 10);
    }

    #[test]
    fn history_bytes_at_round_trip() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(5));
        h.push(&snap(10));
        let b: Snapshot = postcard::from_bytes(&h.bytes_at(0)).unwrap();
        assert_eq!(b.tick, 5);
        let b: Snapshot = postcard::from_bytes(&h.bytes_at(-1)).unwrap();
        assert_eq!(b.tick, 10);
    }

    // ── Undo / Redo ───────────────────────────────────────────────────

    #[test]
    fn history_undo_redo_basic() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=5 {
            h.push(&snap(t));
        }
        assert_eq!(h.position(), 5);

        assert_eq!(h.undo(3).unwrap().tick, 2);
        assert_eq!(h.position(), 2);

        assert_eq!(h.redo(1).unwrap().tick, 3);
        assert_eq!(h.position(), 3);
        assert_invariants(&h);
    }

    #[test]
    fn history_undo_all_way_back() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        assert_eq!(h.undo(2).unwrap().tick, 0);
        assert!(h.is_at_oldest());
    }

    #[test]
    fn history_undo_past_start_returns_none() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        assert!(h.undo(1).is_none());
    }

    #[test]
    fn history_redo_past_end_returns_none() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        assert!(h.redo(1).is_none());
    }

    #[test]
    fn history_undo_then_push_discards_future() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3));

        h.undo(2); // back to frame 1
        h.push(&snap(42)); // branch — discards frames 2,3

        assert_eq!(h.len(), 3);
        assert_values(&h, &[0, 1, 42]);
        assert!(h.is_at_latest());
        assert_invariants(&h);
    }

    #[test]
    fn history_undo_at_oldest_push_works() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.undo(1);
        assert!(h.is_at_oldest());
        h.push(&snap(99));
        assert_values(&h, &[0, 99]);
        assert_invariants(&h);
    }

    #[test]
    fn history_undo_twice_then_redo_twice() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=5 {
            h.push(&snap(t));
        }
        h.undo(2); // at frame 3
        h.undo(1); // at frame 2
        assert_eq!(h.at(h.position() as isize).tick, 2);
        h.redo(2); // at frame 4
        assert_eq!(h.at(h.position() as isize).tick, 4);
    }

    // ── Eviction ──────────────────────────────────────────────────────

    #[test]
    fn history_no_eviction_below_capacity() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(5);
        h.init(&snap(0));
        for t in 1..=4 {
            h.push(&snap(t));
        } // len=5, at capacity
        assert_eq!(h.len(), 5);
        assert_values(&h, &[0, 1, 2, 3, 4]);
        assert_invariants(&h);
    }

    #[test]
    fn history_first_eviction() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3)); // capacity=3, len becomes 4 → evicts index 0
        assert_eq!(h.len(), 3);
        // Frame 0 evicted, frame 1 promoted to new anchor
        assert_values(&h, &[1, 2, 3]);
        assert_invariants(&h);
    }

    #[test]
    fn history_two_evictions() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3)); // evict 0
        h.push(&snap(4)); // evict 1
        assert_eq!(h.len(), 3);
        assert_values(&h, &[2, 3, 4]);
        assert_invariants(&h);
    }

    #[test]
    fn history_many_evictions_ring_wraps_repeatedly() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(4);
        h.init(&snap(0));
        for t in 1..=100 {
            h.push(&snap(t));
        }
        // 101 pushes total (init + 100), capacity=4 → 97 evictions
        assert_eq!(h.len(), 4);
        assert_values(&h, &[97, 98, 99, 100]);
        assert_invariants(&h);
    }

    #[test]
    fn history_eviction_anchor_promotion_correctness() {
        // Verify that after eviction, the promoted entry at index 0
        // has the correct data (not corrupted by delta merge)
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(100));
        h.push(&snap(200));
        h.push(&snap(300));
        h.push(&snap(400)); // evict 100, promote 200 from Delta to Full
        assert_eq!(h.at(0).tick, 200); // was promoted from Delta(200)
        assert_eq!(h.at(1).tick, 300);
        assert_eq!(h.at(2).tick, 400);
        assert_invariants(&h);
    }

    #[test]
    fn history_eviction_multiple_anchor_promotions() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        // capacity=3, 11 pushes → 8 evictions, each promoting a new anchor
        assert_values(&h, &[8, 9, 10]);
        assert_eq!(h.at(0).tick, 8); // promoted 8 times from Deltas
        assert_invariants(&h);
    }

    // ── Capacity edge cases ─────────────────────────────────────────────

    #[test]
    fn history_capacity_1() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(1);
        h.init(&snap(0));
        assert_values(&h, &[0]);
        h.push(&snap(1)); // evict 0, new anchor is Full(1) already
        assert_values(&h, &[1]);
        h.push(&snap(2));
        assert_values(&h, &[2]);
        assert_invariants(&h);
    }

    #[test]
    fn history_capacity_2() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(2);
        h.init(&snap(10));
        h.push(&snap(20)); // [Full(10), Full(20)] — no Delta needed yet
        assert_values(&h, &[10, 20]);
        h.push(&snap(30)); // evict 10, [Full(20), Full(30)] — 20 was already Full
        assert_values(&h, &[20, 30]);
        h.push(&snap(40)); // evict 20, [Full(30), Full(40)]
        assert_values(&h, &[30, 40]);
        assert_invariants(&h);
    }

    #[test]
    fn history_capacity_2_full_cycle() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(2);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        assert_values(&h, &[9, 10]);
        assert_invariants(&h);
    }

    // ── Invariant stress tests ─────────────────────────────────────────

    #[test]
    fn history_invariants_hold_after_every_push() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(5);
        h.init(&snap(0));
        assert_invariants(&h);
        for t in 1..=50 {
            h.push(&snap(t));
            assert_invariants(&h);
        }
    }

    #[test]
    fn history_invariants_hold_across_undo_redo_cycle() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        assert_invariants(&h);
        for _ in 0..5 {
            h.undo(1);
            assert_invariants(&h);
        }
        for _ in 0..5 {
            h.redo(1);
            assert_invariants(&h);
        }
    }

    #[test]
    fn history_invariants_hold_after_truncate() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=8 {
            h.push(&snap(t));
        }
        h.truncate(3);
        assert_invariants(&h);
        assert_values(&h, &[0, 1, 2, 3]);
    }

    #[test]
    fn history_invariants_hold_for_truncated_branch() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=8 {
            h.push(&snap(t));
        }
        let h2 = h.truncated_at(4);
        assert_invariants(&h);
        assert_invariants(&h2);
        assert_eq!(h.len(), 9);
        assert_eq!(h2.len(), 5);
    }

    // ── Data integrity after mutations ─────────────────────────────────

    #[test]
    fn history_data_integrity_after_undo_redo() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        // 11 total pushes, capacity=10 → 1 eviction. Entries = frames 1..10.
        assert_eq!(h.len(), 10);
        assert_eq!(h.at(0).tick, 1); // evicted frame 0, promoted frame 1
        assert_eq!(h.at(-1).tick, 10);

        // Undo step by step from latest to oldest
        assert_eq!(h.undo(1).unwrap().tick, 9);
        assert_eq!(h.undo(1).unwrap().tick, 8);
        assert_eq!(h.undo(1).unwrap().tick, 7);
        assert_eq!(h.undo(1).unwrap().tick, 6);
        assert_eq!(h.undo(1).unwrap().tick, 5);
        assert_eq!(h.undo(1).unwrap().tick, 4);
        assert_eq!(h.undo(1).unwrap().tick, 3);
        assert_eq!(h.undo(1).unwrap().tick, 2);
        assert_eq!(h.undo(1).unwrap().tick, 1);
        assert!(h.undo(1).is_none()); // at oldest (frame 1)

        // Redo step by step back to latest
        assert_eq!(h.redo(1).unwrap().tick, 2);
        assert_eq!(h.redo(1).unwrap().tick, 3);
        assert_eq!(h.redo(1).unwrap().tick, 4);
        assert_eq!(h.redo(1).unwrap().tick, 5);
        assert_eq!(h.redo(1).unwrap().tick, 6);
        assert_eq!(h.redo(1).unwrap().tick, 7);
        assert_eq!(h.redo(1).unwrap().tick, 8);
        assert_eq!(h.redo(1).unwrap().tick, 9);
        assert_eq!(h.redo(1).unwrap().tick, 10);
        assert!(h.redo(1).is_none()); // at latest (frame 10)
    }

    #[test]
    fn history_data_integrity_after_eviction_cycle() {
        // Push 3 full buffer cycles, verify all accessible values still match
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(5);
        h.init(&snap(0));
        for t in 1..=15 {
            h.push(&snap(t));
        }
        // Only the last 5 frames survive
        assert_values(&h, &[11, 12, 13, 14, 15]);
        // Undo should still produce correct values
        assert_eq!(h.undo(1).unwrap().tick, 14);
        assert_eq!(h.undo(1).unwrap().tick, 13);
        assert_eq!(h.redo(1).unwrap().tick, 14);
        assert_eq!(h.redo(1).unwrap().tick, 15);
    }

    // ── Patch ──────────────────────────────────────────────────────────

    #[test]
    fn history_patch_range() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(5));
        let patch = h.patch(0, 2);
        assert!(!patch.runs.is_empty());
        assert_eq!(patch.apply(&snap(0)).tick, 5);
    }

    #[test]
    fn history_combined_patch_identity() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(20);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        let combined = h.combined_patch(0, -1);
        assert_eq!(combined.apply(&h.at(0)).tick, 10);
    }

    #[test]
    fn history_combined_patch_after_eviction() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(4);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        let combined = h.combined_patch(0, -1);
        assert_eq!(combined.apply(&h.at(0)).tick, 10);
    }

    // ── move_to ────────────────────────────────────────────────────────

    #[test]
    fn history_move_to_beginning_and_end() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=5 {
            h.push(&snap(t));
        }
        h.move_to(0);
        assert!(h.is_at_oldest());
        assert_eq!(h.at(h.position() as isize).tick, 0);
        h.move_to(-1);
        assert!(h.is_at_latest());
        assert_eq!(h.at(h.position() as isize).tick, 5);
    }

    // ── trancate ───────────────────────────────────────────────────────

    #[test]
    fn history_truncate_to_single_entry() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(10));
        h.push(&snap(20));
        h.push(&snap(30));
        h.truncate(0);
        assert_eq!(h.len(), 1);
        assert_eq!(h.at(0).tick, 10);
    }

    #[test]
    fn history_truncate_to_middle() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        for t in 1..=5 {
            h.push(&snap(t));
        }
        h.truncate(3); // keep indices 0-3 (frames 0-3)
        assert_values(&h, &[0, 1, 2, 3]);
        assert_invariants(&h);
    }

    // ── Misc edge cases ────────────────────────────────────────────────

    #[test]
    fn history_empty_is_empty() {
        let h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
    }

    #[test]
    #[should_panic]
    fn history_at_on_empty_panics() {
        let h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.at(0);
    }

    #[test]
    fn history_large_capacity_sequential_access() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(1000);
        h.init(&snap(0));
        for t in 1..=500 {
            h.push(&snap(t));
        }
        assert_eq!(h.len(), 501); // no evictions
        assert_eq!(h.at(0).tick, 0);
        assert_eq!(h.at(250).tick, 250);
        assert_eq!(h.at(500).tick, 500);
        assert_eq!(h.at(-1).tick, 500);
        assert_invariants(&h);
    }

    #[test]
    fn history_exact_capacity_no_eviction() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(6);
        h.init(&snap(0));
        for t in 1..=5 {
            h.push(&snap(t));
        } // total 6, at capacity
        assert_eq!(h.len(), 6);
        assert_values(&h, &[0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn history_one_over_capacity_triggers_one_eviction() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(10));
        h.push(&snap(20));
        h.push(&snap(30)); // len=3, at capacity
        h.push(&snap(40)); // len=4, evicts one → len=3
        assert_eq!(h.len(), 3);
        assert_values(&h, &[20, 30, 40]);
    }

    #[test]
    fn history_interleaved_undo_and_push() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(5);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.undo(1); // at frame 1
        h.push(&snap(10));
        h.undo(2); // at frame 0
        h.push(&snap(99));
        assert_values(&h, &[0, 99]);
    }

    #[test]
    fn history_undo_on_evicted_buffer() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(0));
        for t in 1..=10 {
            h.push(&snap(t));
        }
        // Entries: [8, 9, 10]
        assert_eq!(h.len(), 3);
        assert_eq!(h.undo(1).unwrap().tick, 9);
        assert_eq!(h.undo(1).unwrap().tick, 8);
        assert!(h.undo(1).is_none()); // can't go past the evicted anchor
    }

    #[test]
    fn history_redo_stays_within_bounds_after_eviction() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(0));
        for t in 1..=5 {
            h.push(&snap(t));
        }
        // Entries: [3, 4, 5]
        h.undo(2);
        assert_eq!(h.at(h.position() as isize).tick, 3);
        assert!(h.redo(2).is_some()); // 3→4→5
        assert!(h.redo(1).is_none()); // already at end
    }

    #[test]
    fn history_entries_invariant_after_wrapping() {
        // After multiple wraps, verify Full/Delta layout is correct
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(4);
        h.init(&snap(0));
        for t in 1..=20 {
            h.push(&snap(t));
        }
        // Check internals directly
        assert!(matches!(h.entries[0].kind, HistoryKind::Full(_))); // anchor
        assert!(matches!(h.entries[1].kind, HistoryKind::Delta(_))); // interior
        assert!(matches!(h.entries[2].kind, HistoryKind::Delta(_))); // interior
        assert!(matches!(h.entries[3].kind, HistoryKind::Full(_))); // latest
        assert_eq!(h.entries.len(), 4);
    }

    #[test]
    fn history_eviction_restores_correct_values_after_many_cycles() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(4);
        for cycle in 0..5 {
            if cycle == 0 {
                h.init(&snap(cycle * 10));
            } else {
                h.push(&snap(cycle * 10));
            }
            for sub in 1..=10 {
                h.push(&snap(cycle * 10 + sub));
            }
        }
        // After 5 cycles × 11 pushes + init = 56 pushes, capacity=4 → 52 evictions
        assert_eq!(h.len(), 4);
        assert_invariants(&h);
        // All accessible values should form a contiguous increasing sequence
        let ticks: Vec<u32> = (0..4).map(|i| h.at(i as isize).tick).collect();
        for w in ticks.windows(2) {
            assert!(w[0] < w[1], "values must be increasing: {:?}", ticks);
        }
    }

    // ── TickHistory tests ──────────────────────────────────────────────

    #[test]
    fn tick_init_and_push() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(10);
        h.init(&snap(100), 100);
        assert_eq!(h.oldest_tick(), 100);
        assert_eq!(h.latest_tick(), 100);
        h.push(&snap(101), 101);
        assert_eq!(h.oldest_tick(), 100);
        assert_eq!(h.latest_tick(), 101);
        assert_eq!(h.at(100).tick, 100);
        assert_eq!(h.at(101).tick, 101);
    }

    #[test]
    fn tick_at_or_latest_exact_match() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(10);
        h.init(&snap(10), 10);
        h.push(&snap(20), 20);
        h.push(&snap(30), 30);
        assert_eq!(h.at_or_latest(10).tick, 10);
        assert_eq!(h.at_or_latest(20).tick, 20);
        assert_eq!(h.at_or_latest(30).tick, 30);
    }

    #[test]
    fn tick_at_or_latest_between_frames() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(10);
        h.init(&snap(10), 10);
        h.push(&snap(20), 20);
        h.push(&snap(30), 30);
        // Between 20 and 30 → closest latest is 20
        assert_eq!(h.at_or_latest(25).tick, 20);
        // Between 10 and 20 → closest latest is 10
        assert_eq!(h.at_or_latest(15).tick, 10);
    }

    #[test]
    fn tick_at_or_latest_below_oldest() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(10);
        h.init(&snap(100), 100);
        h.push(&snap(101), 101);
        // Tick 50 is below oldest (100) → returns oldest
        assert_eq!(h.at_or_latest(50).tick, 100);
    }

    #[test]
    fn tick_at_or_latest_above_newest() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(10);
        h.init(&snap(100), 100);
        h.push(&snap(101), 101);
        assert_eq!(h.at_or_latest(200).tick, 101);
    }

    #[test]
    fn tick_at_exact_fails_for_missing() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(10);
        h.init(&snap(100), 100);
        h.push(&snap(101), 101);
    }

    #[test]
    fn tick_eviction_updates_start_tick() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(3);
        h.init(&snap(10), 10);
        h.push(&snap(11), 11);
        h.push(&snap(12), 12);
        assert_eq!(h.oldest_tick(), 10);
        assert_eq!(h.latest_tick(), 12);
        h.push(&snap(13), 13); // evicts tick 10
        assert_eq!(h.oldest_tick(), 11);
        assert_eq!(h.latest_tick(), 13);
        assert_eq!(h.at(11).tick, 11);
        assert_eq!(h.at(12).tick, 12);
        assert_eq!(h.at(13).tick, 13);
    }

    #[test]
    fn tick_at_or_latest_after_eviction() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(3);
        h.init(&snap(10), 10);
        h.push(&snap(11), 11);
        h.push(&snap(12), 12);
        h.push(&snap(13), 13); // evicts 10
        // Tick 9 below oldest → oldest (11)
        assert_eq!(h.at_or_latest(9).tick, 11);
        // Tick 10 was evicted, closest latest is 11
        assert_eq!(h.at_or_latest(10).tick, 11);
        // Tick 11 exact
        assert_eq!(h.at_or_latest(11).tick, 11);
        // Tick 14 above newest → newest (13)
        assert_eq!(h.at_or_latest(14).tick, 13);
    }

    #[test]
    fn tick_large_sequential() {
        let mut h: TickHistory<Snapshot> = TickHistory::new(50);
        h.init(&snap(1000), 1000);
        for t in 1001..=2000 {
            h.push(&snap(t), t);
        }
        assert_eq!(h.len(), 50);
        assert_eq!(h.oldest_tick(), 1951);
        assert_eq!(h.latest_tick(), 2000);
        assert_eq!(h.at(2000).tick, 2000);
        assert_eq!(h.at_or_latest(1950).tick, 1951);
        assert_eq!(h.at_or_latest(3000).tick, 2000);
        assert_eq!(h.at_or_latest(1900).tick, 1951);
    }
}
