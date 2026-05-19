use serde::{Deserialize, Serialize};

// ── Byte-level delta (generic, serde-based) ─────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct ByteDelta {
    pub chunk_shift: u8,
    pub total_len: u32,
    pub changes: Vec<ChunkChange>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChunkChange {
    pub idx: u32,
    pub data: Vec<u8>,
}

impl ByteDelta {
    pub fn diff<T: Serialize>(old: &T, new: &T, chunk_shift: u8) -> Self {
        let old_bytes = postcard::to_allocvec(old).unwrap();
        let new_bytes = postcard::to_allocvec(new).unwrap();
        let changes = Self::diff_bytes(&old_bytes, &new_bytes, chunk_shift);
        ByteDelta {
            chunk_shift,
            total_len: new_bytes.len() as u32,
            changes,
        }
    }

    /// Diff two byte slices directly — no serde overhead.
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

    pub fn apply<T: serde::de::DeserializeOwned + Serialize>(&self, old: &T) -> T {
        let mut old_bytes = postcard::to_allocvec(old).unwrap();
        apply_bytes_in_place(&mut old_bytes, &self.changes, self.chunk_shift);
        if (old_bytes.len() as u32) < self.total_len {
            old_bytes.resize(self.total_len as usize, 0);
        }
        postcard::from_bytes(&old_bytes).unwrap()
    }

    /// Apply delta bytes directly to an existing byte buffer.
    pub fn apply_bytes(&self, old: &[u8]) -> Vec<u8> {
        let mut buf = old.to_vec();
        apply_bytes_in_place(&mut buf, &self.changes, self.chunk_shift);
        if (buf.len() as u32) < self.total_len {
            buf.resize(self.total_len as usize, 0);
        }
        buf
    }

    pub fn changed_bytes(&self) -> usize {
        self.changes.iter().map(|c| c.data.len()).sum()
    }

    pub fn serialized_size(&self) -> usize {
        postcard::to_allocvec(self).unwrap().len()
    }
}

fn apply_bytes_in_place(bytes: &mut Vec<u8>, changes: &[ChunkChange], chunk_shift: u8) {
    let chunk_size = 1usize << chunk_shift;
    for change in changes {
        let start = (change.idx as usize) * chunk_size;
        let end = start + change.data.len();
        if end > bytes.len() {
            bytes.resize(end, 0);
        }
        bytes[start..end].copy_from_slice(&change.data);
    }
}

/// Compute delta using rapid field-level comparison via a trait.
///
/// This approach requires the type to partition itself into `&[u8]` slices.
/// For fixed-struct types this is far faster than serde round-trip.
pub trait RawDelta: Sized {
    fn raw_diff(&self, new: &Self, chunk_shift: u8) -> ByteDelta;
    fn raw_apply(&mut self, delta: &ByteDelta);
}

/// A helper that provides raw byte access for any type that can be
/// viewed as `&[u8]` (e.g. via bytemuck) plus serde for variable parts.
pub trait ByteView {
    fn as_bytes(&self) -> &[u8];
    fn from_bytes(bytes: &[u8]) -> Self;
}

/// Fast memcmp-based chunk comparison.
pub fn compare_chunks(old: &[u8], new: &[u8], chunk_shift: u8) -> Vec<ChunkChange> {
    let chunk_size = 1usize << chunk_shift;
    let len = old.len().max(new.len());
    let n_chunks = len.div_ceil(chunk_size);
    let mut changes = Vec::new();

    for ci in 0..n_chunks {
        let start = ci * chunk_size;
        let end = (start + chunk_size).min(len);

        let old_slice = old.get(start..end).unwrap_or(&[]);
        let new_slice = new.get(start..end).unwrap_or(&[]);

        if old_slice != new_slice {
            changes.push(ChunkChange {
                idx: ci as u32,
                data: new_slice.to_vec(),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Point {
        x: f32,
        y: f32,
        z: f32,
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Snapshot {
        tick: u32,
        bodies: Vec<Body>,
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Body {
        id: u32,
        pos: [f32; 3],
        vel: [f32; 3],
        active: bool,
    }

    #[test]
    fn round_trip_identical() {
        let a = Snapshot {
            tick: 100,
            bodies: vec![Body {
                id: 1,
                pos: [1.0, 2.0, 3.0],
                vel: [0.0, 0.0, 0.0],
                active: true,
            }],
        };
        let delta = ByteDelta::diff(&a, &a, 6);
        assert!(delta.changes.is_empty(), "identical should have no changes");
    }

    #[test]
    fn round_trip_single_change() {
        let a = Snapshot {
            tick: 100,
            bodies: vec![Body {
                id: 1,
                pos: [1.0, 2.0, 3.0],
                vel: [0.0, 0.0, 0.0],
                active: true,
            }],
        };
        let b = Snapshot {
            tick: 100,
            bodies: vec![Body {
                id: 1,
                pos: [9.0, 2.0, 3.0],
                vel: [0.0, 0.0, 0.0],
                active: true,
            }],
        };
        let delta = ByteDelta::diff(&a, &b, 6);
        assert!(!delta.changes.is_empty(), "changed should have deltas");

        let restored: Snapshot = delta.apply(&a);
        assert_eq!(restored, b, "apply should reconstruct new value");

        // Also test apply in place
        assert_eq!(b, restored);
    }

    #[test]
    fn round_trip_add_body() {
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
                    pos: [1.0; 3],
                    vel: [0.0; 3],
                    active: true,
                },
            ],
        };
        let delta = ByteDelta::diff(&a, &b, 6);
        let restored: Snapshot = delta.apply(&a);
        assert_eq!(restored, b);
    }
}
