use serde::{Deserialize, Serialize};

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
                let data = new[start..i].to_vec();
                if !data.is_empty() {
                    runs.push(Run { offset: start as u32, data });
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

    pub fn apply<T: serde::de::DeserializeOwned + Serialize>(&self, old: &T) -> T {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Body { id: u32, pos: [f32; 3], vel: [f32; 3], active: bool }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Snapshot { tick: u32, bodies: Vec<Body> }

    #[test]
    fn run_identical() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        let d = RunDelta::diff(&a, &a);
        assert!(d.runs.is_empty());
    }

    #[test]
    fn run_single_field_change() {
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
        let a = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [1.0; 3], vel: [0.0; 3], active: false },
        ]};
        let b = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [9.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [8.0; 3], vel: [1.0; 3], active: false },
        ]};
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
}
