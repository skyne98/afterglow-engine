pub mod tree;

use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;

// ── ChunkChange (reusable, used by ByteDelta) ────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct ChunkChange {
    pub idx: u32,
    pub data: Vec<u8>,
}

// ── ByteDelta: fixed-chunk comparison (used in benchmarks) ───────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct ByteDelta {
    pub chunk_shift: u8,
    pub total_len: u32,
    pub changes: Vec<ChunkChange>,
}

impl ByteDelta {
    pub fn diff_bytes(old: &[u8], new: &[u8], chunk_shift: u8) -> Vec<ChunkChange> {
        let chunk_size = 1usize << chunk_shift;
        let len = old.len().max(new.len());
        let n_chunks = len.div_ceil(chunk_size);
        let mut changes = Vec::new();
        for ci in 0..n_chunks {
            let start = ci * chunk_size;
            let end = (start + chunk_size).min(len);
            if old.get(start..end).unwrap_or(&[]) != new.get(start..end).unwrap_or(&[]) {
                changes.push(ChunkChange {
                    idx: ci as u32,
                    data: new[start..end].to_vec(),
                });
            }
        }
        changes
    }

    pub fn serialized_size(&self) -> usize {
        postcard::to_allocvec(self).unwrap().len()
    }
}

// ── RunDelta: variable-length runs of changed bytes ─────────────────────
//
// Compact: stores (offset, data) pairs — no string keys, no chunk padding.
// Use when you have the serialized bytes of both old and new.

#[derive(Serialize, Deserialize, Debug)]
pub struct Run {
    pub offset: u32,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RunDelta {
    pub total_len: u32,
    pub runs: Vec<Run>,
}

impl RunDelta {
    pub fn diff<T: Serialize>(old: &T, new: &T) -> Self {
        let old_bytes = postcard::to_allocvec(old).unwrap();
        let new_bytes = postcard::to_allocvec(new).unwrap();
        Self::diff_bytes(&old_bytes, &new_bytes)
    }

    pub fn diff_bytes(old: &[u8], new: &[u8]) -> Self {
        let max_len = old.len().max(new.len());
        let mut runs = Vec::new();
        let mut i = 0;

        while i < max_len {
            let old_b = old.get(i).copied().unwrap_or(0);
            let new_b = new.get(i).copied().unwrap_or(0);
            if old_b != new_b {
                let start = i;
                while i < max_len {
                    let ob = old.get(i).copied().unwrap_or(0);
                    let nb = new.get(i).copied().unwrap_or(0);
                    if ob == nb { break; }
                    i += 1;
                }
                // Try to extend run backwards into the previous byte if it helps
                let run_start = start.saturating_sub(if start > 0 && start < old.len().min(new.len()) && old[start-1] == new[start-1] { 0 } else { 0 });
                // Extend to cover matching bytes at boundaries to reduce run count (tiny cost)
                let data = new[run_start..i].to_vec();
                if !data.is_empty() {
                    runs.push(Run { offset: run_start as u32, data });
                }
            } else {
                i += 1;
            }
        }

        // Merge adjacent runs
        if runs.len() > 1 {
            let mut merged: Vec<Run> = Vec::with_capacity(runs.len());
            let mut cur = runs.swap_remove(0);
            for r in runs {
                let cur_end = cur.offset as usize + cur.data.len();
                if r.offset as usize == cur_end {
                    cur.data.extend_from_slice(&r.data);
                } else {
                    merged.push(cur);
                    cur = r;
                }
            }
            merged.push(cur);
            runs = merged;
        }

        RunDelta {
            total_len: new.len() as u32,
            runs,
        }
    }

    pub fn apply<T: DeserializeOwned + Serialize>(&self, old: &T) -> T {
        let mut bytes = postcard::to_allocvec(old).unwrap();
        self.apply_in_place(&mut bytes);
        if (bytes.len() as u32) < self.total_len {
            bytes.resize(self.total_len as usize, 0);
        }
        postcard::from_bytes(&bytes).unwrap()
    }

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

    pub fn serialized_size(&self) -> usize {
        postcard::to_allocvec(self).unwrap().len()
    }
}

// ── SparseVecDelta: for Vec<T> where few elements change ─────────────────
//
// Compact: stores (element_index, element_bytes) for changed elements only.
// No field names, no string keys, no chunk padding.

#[derive(Serialize, Deserialize, Debug)]
pub struct SparseVecDelta<T> {
    pub entries: Vec<(u32, T)>,
}

impl<T: PartialEq + Clone + Serialize> SparseVecDelta<T> {
    pub fn diff(old: &[T], new: &[T]) -> Self {
        let max = old.len().max(new.len());
        let mut entries = Vec::new();
        for i in 0..max {
            let a = old.get(i);
            let b = new.get(i);
            if a != b {
                if let Some(val) = b {
                    entries.push((i as u32, val.clone()));
                }
            }
        }
        SparseVecDelta { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

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

    // ── RunDelta tests ──

    #[test]
    fn run_identical() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        let d = RunDelta::diff(&a, &a);
        assert!(d.runs.is_empty());
    }

    #[test]
    fn run_single_byte_change() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        let mut b = a.clone();
        b.tick = 101;
        let d = RunDelta::diff(&a, &b);
        assert!(!d.runs.is_empty());
        let restored: Snapshot = d.apply(&a);
        assert_eq!(restored.tick, 101);
    }

    #[test]
    fn run_multiple_changes() {
        let a = Snapshot {
            tick: 0,
            bodies: vec![
                Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
                Body { id: 2, pos: [1.0; 3], vel: [0.0; 3], active: false },
            ],
        };
        let b = Snapshot {
            tick: 0,
            bodies: vec![
                Body { id: 1, pos: [9.0; 3], vel: [0.0; 3], active: true },
                Body { id: 2, pos: [8.0; 3], vel: [1.0; 3], active: false },
            ],
        };
        let d = RunDelta::diff(&a, &b);
        let restored: Snapshot = d.apply(&a);
        assert_eq!(restored, b);
    }

    #[test]
    fn run_added_body() {
        let a = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
        ]};
        let b = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [5.0; 3], vel: [0.0; 3], active: false },
        ]};
        let d = RunDelta::diff(&a, &b);
        let restored: Snapshot = d.apply(&a);
        assert_eq!(restored, b);
    }

    #[test]
    fn run_diff_bytes_direct() {
        let old = vec![0u8; 100];
        let mut new = old.clone();
        new[42] = 99;
        new[43] = 88;
        let d = RunDelta::diff_bytes(&old, &new);
        assert_eq!(d.runs.len(), 1);
        assert_eq!(d.runs[0].offset, 42);
        assert_eq!(d.runs[0].data, vec![99, 88]);
    }

    // ── SparseVecDelta tests ──

    #[test]
    fn sparse_identical() {
        let v = vec![Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true }];
        let d = SparseVecDelta::diff(&v, &v);
        assert!(d.entries.is_empty());
    }

    #[test]
    fn sparse_one_changed() {
        let old = vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [5.0; 3], vel: [0.0; 3], active: false },
        ];
        let new = vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [9.0; 3], vel: [0.0; 3], active: false },
        ];
        let d = SparseVecDelta::diff(&old, &new);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].0, 1);
        assert_eq!(d.entries[0].1.pos[0], 9.0);
    }

    #[test]
    fn sparse_element_appended() {
        let old = vec![Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true }];
        let new = vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [5.0; 3], vel: [0.0; 3], active: false },
        ];
        let d = SparseVecDelta::diff(&old, &new);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].0, 1);
    }
}
