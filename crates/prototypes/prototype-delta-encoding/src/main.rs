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
    let body_counts = [1000, 10_000, 100_000];

    println!("=== RunDelta: Size ===");
    println!("{:>8} | {:>7} | {:>10} | {:>10} | {:>7}",
        "bodies", "changed", "full_bytes", "delta_bytes", "ratio");

    for &n in &body_counts {
        let base = build_snapshot(n);
        let base_bytes = postcard::to_allocvec(&base).unwrap();

        for &chg in &[0, 1, 10, 100] {
            let changed: Vec<usize> = (0..chg.min(n as usize)).map(|i| i as usize).collect();
            let mut modified = base.clone();
            mutate_translation(&mut modified, &changed);
            let mod_bytes = postcard::to_allocvec(&modified).unwrap();

            let rd = RunDelta::diff_bytes(&base_bytes, &mod_bytes);
            let rd_sz = rd.serialized_size();
            let ratio = rd_sz as f64 / mod_bytes.len() as f64;

            println!("{:>8} | {:>7} | {:>10} | {:>10} | {:>6.4}",
                n, chg, mod_bytes.len(), rd_sz, ratio);
        }
    }

    println!("\n=== RunDelta: Speed (10k bodies) ===");
    println!("{:>10} | {:>10} | {:>10} | {:>10}",
        "method", "changed", "encode_us", "total_us");

    let base = build_snapshot(10_000);
    let base_bytes = postcard::to_allocvec(&base).unwrap();

    for &chg in &[0, 1, 10, 100] {
        let changed: Vec<usize> = (0..chg.min(10_000)).collect();
        let mut modified = base.clone();
        mutate_translation(&mut modified, &changed);
        let mod_bytes = postcard::to_allocvec(&modified).unwrap();

        let s = Instant::now();
        let rd = RunDelta::diff_bytes(&base_bytes, &mod_bytes);
        let enc = s.elapsed().as_secs_f64() * 1_000_000.0;
        let _out = rd.apply_bytes(&base_bytes);

        let s = Instant::now();
        let _full = postcard::to_allocvec(&modified).unwrap();
        let ser = s.elapsed().as_secs_f64() * 1_000_000.0;

        println!("{:>10} | {:>10} | {:>10.1} | {:>10.1}",
            "RunDelta", chg, enc, enc);
        println!("{:>10} | {:>10} | {:>10.1} | {:>10.1}",
            "full_serde", chg, ser, ser);
    }
}
