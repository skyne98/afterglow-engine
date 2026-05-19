use prototype_delta_encoding::RunDelta;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct BodySnapshot {
    index: u32,
    rigidbody_type: u8,
    translation: [f32; 3],
    rotation: [f32; 4],
    linvel: [f32; 3],
    angvel: [f32; 3],
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct JointSnapshot {
    body1_index: u32,
    body2_index: u32,
    kind: JointKindSnapshot,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
enum JointKindSnapshot {
    Spherical { anchor1: [f32; 3], anchor2: [f32; 3], twist_axis: [f32; 3] },
    Fixed { anchor1: [f32; 3], anchor2: [f32; 3] },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct PhysicsSnapshot {
    bodies: Vec<BodySnapshot>,
    joints: Vec<JointSnapshot>,
}

fn build_snapshot(num_bodies: u32) -> PhysicsSnapshot {
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
    let n = 10_000u32;

    println!("=== Full Pipeline: RunDelta vs Baseline (10k bodies) ===");
    println!();
    println!("ENCODE (sender): what it costs to produce the delta");
    println!("  baseline: ser(new) → send full bytes");
    println!("  cold:     ser(old) + ser(new) + diff → send delta");
    println!("  warm:     ser(new) + diff             → send delta (old bytes cached)");
    println!();
    println!("DECODE (receiver): what it costs to recover the snapshot");
    println!("  baseline: deser(bytes) → snapshot");
    println!("  delta:   apply + deser               → snapshot");
    println!();

    let base = build_snapshot(n);
    let base_bytes = postcard::to_allocvec(&base).unwrap();
    let mut results = Vec::new();

    for &chg in &[0, 1, 10, 100, 1000] {
        let changed: Vec<usize> = (0..chg.min(n)).map(|i| i as usize).collect();
        let mut modified = base.clone();
        mutate_translation(&mut modified, &changed);
        let mod_bytes = postcard::to_allocvec(&modified).unwrap();

        // ── Encode costs ──

        // Baseline: just ser(new)
        let s = Instant::now();
        let _base_enc = postcard::to_allocvec(&modified).unwrap();
        let base_enc_us = s.elapsed().as_secs_f64() * 1_000_000.0;

        // Cold: ser(old) + ser(new) + diff
        let s = Instant::now();
        let old_b = postcard::to_allocvec(&base).unwrap();
        let new_b = postcard::to_allocvec(&modified).unwrap();
        let rd = RunDelta::diff_bytes(&old_b, &new_b);
        let cold_enc_us = s.elapsed().as_secs_f64() * 1_000_000.0;

        // Warm: ser(new) + diff (old bytes cached)
        let s = Instant::now();
        let new_b2 = postcard::to_allocvec(&modified).unwrap();
        let _rd2 = RunDelta::diff_bytes(&base_bytes, &new_b2);
        let warm_enc_us = s.elapsed().as_secs_f64() * 1_000_000.0;

        // ── Decode costs ──

        // Baseline: just deser(bytes)
        let s = Instant::now();
        let _val: PhysicsSnapshot = postcard::from_bytes(&mod_bytes).unwrap();
        let base_dec_us = s.elapsed().as_secs_f64() * 1_000_000.0;

        // Delta: apply + deser
        let s = Instant::now();
        let restored = rd.apply_bytes(&base_bytes);
        let _val2: PhysicsSnapshot = postcard::from_bytes(&restored).unwrap();
        let delta_dec_us = s.elapsed().as_secs_f64() * 1_000_000.0;

        results.push((chg, base_enc_us, cold_enc_us, warm_enc_us, base_dec_us, delta_dec_us, rd.serialized_size(), new_b.len()));
    }

    // Table
    println!("{:>7} | {:>10} {:>10} {:>10} | {:>10} {:>10} | {:>10} {:>8}",
        "changed", "enc_base", "enc_cold", "enc_warm", "dec_base", "dec_delta",
        "delta_B", "ratio");

    for (chg, base_enc, cold_enc, warm_enc, base_dec, delta_dec, sz, full) in &results {
        println!("{:>7} | {:>10.1} {:>10.1} {:>10.1} | {:>10.1} {:>10.1} | {:>10} {:>7.4}",
            chg, base_enc, cold_enc, warm_enc, base_dec, delta_dec, sz, *sz as f64 / *full as f64);
    }

    // Summary
    println!();
    println!("=== Takeaways ===");
    println!("Baseline encode+decode: {:.1} μs", results[0].1 + results[0].4);
    println!("Warm delta encode+decode (10 changed): {:.1} μs", results[3].3 + results[3].5);
    println!("Delta bandwidth (10 changed): {} bytes vs {} full ({:.4})",
        results[3].6, results[3].7, results[3].6 as f64 / results[3].7 as f64);
}
