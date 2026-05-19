use serde::{Deserialize, Serialize};

/// A single contiguous run of changed bytes.
#[derive(Serialize, Deserialize, Debug)]
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
#[derive(Serialize, Deserialize, Debug)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Body { id: u32, pos: [f32; 3], vel: [f32; 3], active: bool }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Snapshot { tick: u32, bodies: Vec<Body> }

    #[test]
    fn identical() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        assert!(DeltaPatch::diff(&a, &a).runs.is_empty());
    }

    #[test]
    fn field_change() {
        let a = Snapshot { tick: 100, bodies: vec![] };
        let mut b = a.clone();
        b.tick = 101;
        let restored: Snapshot = DeltaPatch::diff(&a, &b).apply(&a);
        assert_eq!(restored.tick, 101);
    }

    #[test]
    fn multiple_changes() {
        let a = Snapshot {
            tick: 0, bodies: vec![
                Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
                Body { id: 2, pos: [1.0; 3], vel: [0.0; 3], active: false },
            ],
        };
        let b = Snapshot {
            tick: 0, bodies: vec![
                Body { id: 1, pos: [9.0; 3], vel: [0.0; 3], active: true },
                Body { id: 2, pos: [8.0; 3], vel: [1.0; 3], active: false },
            ],
        };
        assert_eq!(DeltaPatch::diff(&a, &b).apply(&a), b);
    }

    #[test]
    fn added_element() {
        let a = Snapshot { tick: 0, bodies: vec![Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true }] };
        let b = Snapshot { tick: 0, bodies: vec![
            Body { id: 1, pos: [0.0; 3], vel: [0.0; 3], active: true },
            Body { id: 2, pos: [5.0; 3], vel: [0.0; 3], active: false },
        ]};
        assert_eq!(DeltaPatch::diff(&a, &b).apply(&a), b);
    }

    #[test]
    fn diff_bytes_direct() {
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
    fn apply_bytes_round_trip() {
        let old = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut new = old.clone();
        new[2..6].copy_from_slice(&[99, 99, 99, 99]);
        let d = DeltaPatch::diff_bytes(&old, &new);
        let restored = d.apply_bytes(&old);
        assert_eq!(restored, new);
    }
}
