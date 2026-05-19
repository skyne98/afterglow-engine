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
                runs.push(Run { offset: start as u32, data: new[start..i].to_vec() });
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

        DeltaPatch { total_len: new.len() as u32, runs }
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
///   advancing the cursor. If you previously undid, future entries are discarded.
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
    ///   Index 0 is always kept as a full snapshot anchor. Index -1 is always the
    ///   latest full snapshot. Everything between is compact delta patches.
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
            // If the new entry at index 0 is a Delta, apply the old anchor to promote it to Full
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

    /// Immutable: produce a new history truncated to `index`. Negative = from end.
    /// Original is unchanged.
    pub fn truncated_at(&self, index: isize) -> Self {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        let mut h = Self::new(self.capacity);
        h.entries = self.entries[..=i].to_vec();
        h.cursor = i.min(h.entries.len().saturating_sub(1));
        h
    }

    // ── Mutable operations ─────────────────────────────────────────────────

    /// Truncate entries after `index`. Negative = from end.
    pub fn truncate(&mut self, index: isize) {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        self.entries.truncate(i + 1);
        self.cursor = self.cursor.min(i);
    }

    /// Move cursor to an absolute index. Negative = from end.
    pub fn move_to(&mut self, index: isize) {
        let i = self.resolve(index);
        assert!(i < self.entries.len());
        self.cursor = i;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Body { id: u32, pos: [f32; 3], vel: [f32; 3], active: bool }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Snapshot { tick: u32, bodies: Vec<Body> }

    fn snap(tick: u32) -> Snapshot {
        Snapshot { tick, bodies: vec![Body { id: 1, pos: [tick as f32; 3], vel: [0.0; 3], active: true }] }
    }

    // ── DeltaPatch tests ──

    #[test]
    fn delta_identical() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        assert!(DeltaPatch::diff(&a, &a).runs.is_empty());
    }

    #[test]
    fn delta_field_change() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        let mut b = a.clone();
        b.tick = 101;
        let restored: Snapshot = DeltaPatch::diff(&a, &b).apply(&a);
        assert_eq!(restored.tick, 101);
    }

    #[test]
    fn delta_multiple() {
        let a = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [1.0; 3], vel: [0.0; 3], active: false },
        ]};
        let b = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [9.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [8.0; 3], vel: [1.0; 3], active: false },
        ]};
        assert_eq!(DeltaPatch::diff(&a, &b).apply(&a), b);
    }

    #[test]
    fn delta_added_element() {
        let a = Snapshot { tick: 0, bodies: vec![Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true }] };
        let b = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [5.0; 3], vel: [0.0; 3], active: false },
        ]};
        assert_eq!(DeltaPatch::diff(&a, &b).apply(&a), b);
    }

    #[test]
    fn delta_diff_bytes() {
        let old = vec![0u8; 100];
        let mut new = old.clone();
        new[42] = 99; new[43] = 88;
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

    // ── DeltaHistory tests ──

    #[test]
    fn history_undo_redo_simple() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));

        for t in 1..=5 {
            h.push(&snap(t));
        }

        assert!(h.is_at_latest());
        assert_eq!(h.position(), 5);

        let s = h.undo(3).unwrap();
        assert_eq!(s.tick, 2);

        let s = h.redo(1).unwrap();
        assert_eq!(s.tick, 3);
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
    fn history_undoredo_discard_new_timeline() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3));

        h.undo(2); // back to frame 1
        h.push(&snap(42)); // branch — should discard frames 2,3

        assert_eq!(h.len(), 3); // frames 0,1,42 (frames 2,3 discarded)
        assert!(h.is_at_latest());

        let s = h.at(2);
        assert_eq!(s.tick, 42);
    }

    #[test]
    fn history_eviction_oldest_removed() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(3);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3));

        assert_eq!(h.len(), 3); // frames 1,2,3 (frame 0 evicted)
    }

    #[test]
    fn history_restore_from_full_snapshot() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        // frame 3 is a snapshot (interval = 3)
        h.push(&snap(3));
        h.push(&snap(4));
        h.push(&snap(5));

        let s = h.at(5);
        assert_eq!(s.tick, 5);
    }

    #[test]
    fn history_undo_after_eviction() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(4);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3)); // snapshot (3%3==0), evicts frame 0
        h.push(&snap(4)); // evicts frame 1

        assert_eq!(h.len(), 4); // frames 1,2,3,4 (frame 0 evicted)

        let s = h.undo(1).unwrap();
        assert_eq!(s.tick, 3);
    }

    #[test]
    fn history_at_and_bytes_at() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(10));
        h.push(&snap(20));
        h.push(&snap(30));
        assert_eq!(h.at(0).tick, 10);
        assert_eq!(h.at(1).tick, 20);
        assert_eq!(h.at(2).tick, 30);
        assert_eq!(h.at(-1).tick, 30);  // negative index
        assert_eq!(h.at(-2).tick, 20);
        assert_eq!(h.at(-3).tick, 10);
        assert_eq!(postcard::from_bytes::<Snapshot>(&h.bytes_at(2)).unwrap().tick, 30);
        assert_eq!(postcard::from_bytes::<Snapshot>(&h.bytes_at(-1)).unwrap().tick, 30);
    }

    #[test]
    fn history_truncated_at_is_immutable() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.push(&snap(3));

        let h2 = h.truncated_at(1); // keep only 0,1
        assert_eq!(h.len(), 4);    // original unchanged
        assert_eq!(h2.len(), 2);   // new has 2
        assert_eq!(h2.at(1).tick, 1);
        assert_eq!(h2.at(-1).tick, 1);
    }

    #[test]
    fn history_truncate_discards_future() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.truncate(0); // discard everything after frame 0
        assert_eq!(h.len(), 1);
        assert_eq!(h.at(0).tick, 0);
    }

    #[test]
    fn history_move_to_repositions_cursor() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(2));
        h.move_to(0);
        assert!(h.is_at_oldest());
        assert!(!h.is_at_latest());
        h.move_to(2);
        assert!(h.is_at_latest());
        h.move_to(-1);
        assert!(h.is_at_latest());
    }

    #[test]
    fn history_patch_between_frames() {
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(10);
        h.init(&snap(0));
        h.push(&snap(1));
        h.push(&snap(5));
        let patch = h.patch(0, 2);
        assert!(!patch.runs.is_empty());
        let restored = patch.apply(&snap(0));
        assert_eq!(restored.tick, 5);
    }

    #[test]
    fn history_combined_patch_collapses_range() {
        // Keep all frames (capacity > total pushes) so full snapshots are preserved
        let mut h: DeltaHistory<Snapshot> = DeltaHistory::new(20);
        h.init(&snap(0));
        for t in 1..=10 { h.push(&snap(t)); }
        assert!(h.len() >= 11);
        let combined = h.combined_patch(0, -1); // from oldest to newest
        let restored = combined.apply(&h.at(0));
        assert_eq!(restored.tick, 10);
        assert!(combined.runs.len() <= 4, "combined should be compact, got {} runs", combined.runs.len());
    }
}
