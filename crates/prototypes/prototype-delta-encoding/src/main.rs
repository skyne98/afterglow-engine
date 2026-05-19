use prototype_delta_encoding::{ByteDelta, RunDelta, SparseVecDelta};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct BodySnapshot {
    index: u32,
    rigidbody_type: u8,
    translation: [f32; 3],
    rotation: [f32; 4],
    linvel: [f32; 3],
    angvel: [f32; 3],
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct JointSnapshot {
    body1_index: u32,
    body2_index: u32,
    kind: JointKindSnapshot,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
enum JointKindSnapshot {
    Spherical { anchor1: [f32; 3], anchor2: [f32; 3], twist_axis: [f32; 3] },
    Fixed { anchor1: [f32; 3], anchor2: [f32; 3] },
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct PhysicsSnapshot {
    bodies: Vec<BodySnapshot>,
    joints: Vec<JointSnapshot>,
}

fn build_snapshot(num_bodies: u32) -> PhysicsSnapshot {
    let _joints_per_body = 0;
    let mut bodies = Vec::with_capacity(num_bodies as usize);
    let mut joints = Vec::new();

    for i in 0..num_bodies {
        let y = 1.0 + (i % 10) as f32 * 1.1;
        bodies.push(BodySnapshot {
            index: i,
            rigidbody_type: 0,
            translation: [0.0, y, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            linvel: [0.0, 0.0, 0.0],
            angvel: [0.0, 0.0, 0.0],
        });
    }
    for i in 1..num_bodies.min(20) {
        joints.push(JointSnapshot {
            body1_index: i - 1,
            body2_index: i,
            kind: JointKindSnapshot::Spherical {
                anchor1: [0.0, 0.5, 0.0],
                anchor2: [0.0, -0.5, 0.0],
                twist_axis: [0.0, 1.0, 0.0],
            },
        });
    }

    PhysicsSnapshot { bodies, joints }
}

fn mutate_translation(snapshot: &mut PhysicsSnapshot, indices: &[usize]) {
    for &i in indices {
        if i < snapshot.bodies.len() {
            snapshot.bodies[i].translation[0] += 0.1;
        }
    }
}

fn main() {
    let body_counts = [1000, 10_000, 100_000];
    let change_counts = [0, 1, 10, 100];

    println!("=== Delta Encoding: Size Comparison ===");
    println!("Comparing delta encoding formats for PhysicsSnapshot");
    println!("No string keys — all deltas use positional integer encoding.");
    println!();
    println!(
        "{:>8} | {:>7} | {:>10} | {:>10} | {:>10} | {:>10} | {:>7}",
        "bodies", "changed", "full_bytes", "RunDelta", "SparseVec", "Chunk(64)", "best_ratio"
    );

    for &n in &body_counts {
        let base = build_snapshot(n);
        let base_bytes = postcard::to_allocvec(&base).unwrap();

        for &chg in &change_counts {
            let changed: Vec<usize> = (0..chg.min(n as usize)).map(|i| i as usize).collect();
            let mut modified = base.clone();
            mutate_translation(&mut modified, &changed);
            let mod_bytes = postcard::to_allocvec(&modified).unwrap();

            // Full size
            let full = mod_bytes.len();

            // RunDelta (variable-length runs, no padding)
            let rd = RunDelta::diff_bytes(&base_bytes, &mod_bytes);
            let rd_sz = rd.serialized_size();

            // SparseVecDelta (element-level, for Vec<BodySnapshot>)
            let sv = SparseVecDelta::diff(&base.bodies, &modified.bodies);
            let sv_sz = postcard::to_allocvec(&sv).unwrap().len();

            // ByteDelta with 64B chunks (old approach)
            let bd = ByteDelta::diff_bytes(&base_bytes, &mod_bytes, 6);
            let bd_delta = ByteDelta {
                chunk_shift: 6,
                total_len: mod_bytes.len() as u32,
                changes: bd,
            };
            let bd_sz = bd_delta.serialized_size();

            let best = full.min(rd_sz).min(sv_sz).min(bd_sz);
            let ratio = best as f64 / full as f64;

            println!(
                "{:>8} | {:>7} | {:>10} | {:>10} | {:>10} | {:>10} | {:>6.4}",
                n, chg, full, rd_sz, sv_sz, bd_sz, ratio,
            );
        }
    }

    // ── Speed: raw-byte vs serde-based ──
    println!("\n=== Delta Encoding: Speed (10k bodies) ===");
    println!("{:>10} | {:>10} | {:>10} | {:>10} | {:>10}",
        "method", "changed", "encode_us", "decode_us", "total_us");

    let base = build_snapshot(10_000);
    let base_bytes = postcard::to_allocvec(&base).unwrap();

    for &chg in &[0, 1, 10, 100] {
        let changed: Vec<usize> = (0..chg.min(10_000)).collect();
        let mut modified = base.clone();
        mutate_translation(&mut modified, &changed);
        let mod_bytes = postcard::to_allocvec(&modified).unwrap();

        // Raw-byte RunDelta encode
        let s = Instant::now();
        let rd = RunDelta::diff_bytes(&base_bytes, &mod_bytes);
        let enc = s.elapsed().as_secs_f64() * 1_000_000.0;
        let s = Instant::now();
        let _out = rd.apply_bytes(&base_bytes);
        let dec = s.elapsed().as_secs_f64() * 1_000_000.0;
        println!("{:>10} | {:>10} | {:>10.1} | {:>10.1} | {:>10.1}",
            "RunDelta", chg, enc, dec, enc + dec);

        // Serde-based full serialize
        let s = Instant::now();
        let _full = postcard::to_allocvec(&modified).unwrap();
        let ser = s.elapsed().as_secs_f64() * 1_000_000.0;
        let s = Instant::now();
        let _deser: PhysicsSnapshot = postcard::from_bytes(&mod_bytes).unwrap();
        let deser = s.elapsed().as_secs_f64() * 1_000_000.0;
        println!("{:>10} | {:>10} | {:>10.1} | {:>10.1} | {:>10.1}",
            "full_serde", chg, ser, deser, ser + deser);
    }
}
