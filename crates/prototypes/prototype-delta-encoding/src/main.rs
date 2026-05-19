use prototype_delta_encoding::ByteDelta;
use serde::{Deserialize, Serialize};
use std::time::Instant;

// ── Concrete snapshot types representative of physics state ─────────────

#[derive(Serialize, Deserialize, Clone)]
struct PhysicsSnapshot {
    bodies: Vec<BodySnapshot>,
    joints: Vec<JointSnapshot>,
}

#[derive(Serialize, Deserialize, Clone)]
struct BodySnapshot {
    index: u32,
    rigidbody_type: u8,
    translation: [f32; 3],
    rotation: [f32; 4],
    linvel: [f32; 3],
    angvel: [f32; 3],
    collider: Option<ColliderSnapshot>,
}

#[derive(Serialize, Deserialize, Clone)]
enum ColliderSnapshot {
    Cuboid { hx: f32, hy: f32, hz: f32 },
    Sphere { radius: f32 },
    Capsule { radius: f32, half_height: f32 },
}

#[derive(Serialize, Deserialize, Clone)]
struct JointSnapshot {
    body1_index: u32,
    body2_index: u32,
    kind: JointKindSnapshot,
}

#[derive(Serialize, Deserialize, Clone)]
enum JointKindSnapshot {
    Spherical {
        anchor1: [f32; 3],
        anchor2: [f32; 3],
        twist_axis: [f32; 3],
    },
    Fixed {
        anchor1: [f32; 3],
        anchor2: [f32; 3],
    },
}

// ── Scene builder ────────────────────────────────────────────────────────

fn build_snapshot(num_bodies: u32, chain_len: u32, offset: f32) -> PhysicsSnapshot {
    let chains = (num_bodies / chain_len).max(1) as usize;
    let links = chain_len as usize;
    let total_bodies = chains * links;
    let mut bodies = Vec::with_capacity(total_bodies);
    let mut joints = Vec::new();

    for ci in 0..chains {
        let base_x = (ci as f32 - chains as f32 / 2.0) * 3.0 + offset;
        let mut prev_idx: Option<u32> = None;
        for li in 0..links {
            let idx = bodies.len() as u32;
            let y = 1.0 + li as f32 * 1.1;
            bodies.push(BodySnapshot {
                index: idx,
                rigidbody_type: 0, // Dynamic
                translation: [base_x, y, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                linvel: [offset * 0.1, 0.0, 0.0],
                angvel: [0.0, 0.0, 0.0],
                collider: Some(ColliderSnapshot::Cuboid {
                    hx: 0.2,
                    hy: 0.2,
                    hz: 0.2,
                }),
            });
            if let Some(prev) = prev_idx {
                joints.push(JointSnapshot {
                    body1_index: prev,
                    body2_index: idx,
                    kind: JointKindSnapshot::Spherical {
                        anchor1: [0.0, 0.5, 0.0],
                        anchor2: [0.0, -0.5, 0.0],
                        twist_axis: [0.0, 1.0, 0.0],
                    },
                });
            }
            prev_idx = Some(idx);
        }
    }

    // Fill remainder with loose boxes
    let remainder = (num_bodies as usize).saturating_sub(total_bodies);
    let grid = (remainder as f32).sqrt().ceil() as i32;
    let spacing = 2.5;
    let x_offset = -(chains as f32 / 2.0) * 3.0 - 20.0 - 10.0;
    for i in 0..remainder {
        let idx = bodies.len() as u32;
        let x = (i as i32 % grid) as f32 * spacing + x_offset;
        let z = (i as i32 / grid) as f32 * spacing - (grid as f32 * spacing) / 2.0;
        bodies.push(BodySnapshot {
            index: idx,
            rigidbody_type: 0,
            translation: [x, 5.0 + i as f32 * 0.01, z],
            rotation: [0.0, 0.0, 0.0, 1.0],
            linvel: [offset * 0.1, 0.0, 0.0],
            angvel: [0.0, 0.0, 0.0],
            collider: Some(ColliderSnapshot::Cuboid {
                hx: 0.25,
                hy: 0.25,
                hz: 0.25,
            }),
        });
    }

    PhysicsSnapshot { bodies, joints }
}

/// Apply small random perturbations to the snapshot to simulate tick-to-tick changes.
fn mutate(snapshot: &mut PhysicsSnapshot, fraction: f32) {
    let n = (snapshot.bodies.len() as f32 * fraction) as usize;
    for i in 0..n.min(snapshot.bodies.len()) {
        let b = &mut snapshot.bodies[i];
        b.translation[0] += 0.01;
        b.translation[1] += 0.005;
        b.linvel[0] += 0.1;
    }
}

fn main() {
    let body_counts = [1000, 10_000];
    let mutations = [0.001, 0.01, 0.05];
    let chunk_shifts = [6]; // 64B chunks

    println!("=== Delta Encoding Benchmark ===");
    println!("Comparing delta encoding vs full serialization for physics snapshots");
    println!("All times in microseconds. 'ratio' = delta_size / full_size.\n");

    println!(
        "{:>6} | {:>7} | {:>10} | {:>10} | {:>6} | {:>8} {:>8} {:>8} {:>7} | {:>10}",
        "bodies", "changed", "full_bytes", "delta_bytes", "ratio",
        "ser_us", "diff_us", "apply_us", "total_us", "raw_diff_us"
    );

    for &n in &body_counts {
        let base = build_snapshot(n, 10, 0.0);
        let base_bytes = postcard::to_allocvec(&base).unwrap();

        for &frac in &mutations {
            let mut modified = base.clone();
            mutate(&mut modified, frac);
            let mod_bytes = postcard::to_allocvec(&modified).unwrap();

            // Full serialization time
            let start = Instant::now();
            let full_bytes = postcard::to_allocvec(&modified).unwrap();
            let ser_us = start.elapsed().as_secs_f64() * 1_000_000.0;

            for &cs in &chunk_shifts {
                // Generic serde-based diff
                let d_start = Instant::now();
                let delta = ByteDelta::diff(&base, &modified, cs);
                let diff_us = d_start.elapsed().as_secs_f64() * 1_000_000.0;

                let a_start = Instant::now();
                let restored: PhysicsSnapshot = delta.apply(&base);
                let a_us = a_start.elapsed().as_secs_f64() * 1_000_000.0;

                let full2 = postcard::to_allocvec(&restored).unwrap();
                assert_eq!(full_bytes, full2);

                // Raw-byte diff (no serde) — optimal path when old bytes are cached
                let rd_start = Instant::now();
                let _changes = ByteDelta::diff_bytes(&base_bytes, &mod_bytes, cs);
                let raw_diff_us = rd_start.elapsed().as_secs_f64() * 1_000_000.0;

                let delta_size = delta.serialized_size();
                let ratio = delta_size as f64 / full_bytes.len() as f64;

                println!(
                    "{:>6} | {:>6.3} | {:>10} | {:>10} | {:>6.4} | {:>8.1} {:>8.1} {:>8.1} {:>7.1} | {:>10.1}",
                    n,
                    frac,
                    full_bytes.len(),
                    delta_size,
                    ratio,
                    ser_us,
                    diff_us,
                    a_us,
                    ser_us.min(diff_us + a_us),
                    raw_diff_us,
                );
            }
        }
    }

    // ── Extreme: 1 of 10k changed ──
    println!("\n--- Extreme: 1 of 10k bodies changed ---");
    let base = build_snapshot(10_000, 10, 0.0);
    let base_b = postcard::to_allocvec(&base).unwrap();
    let mut m = base.clone();
    m.bodies[0].translation[0] += 1.0;
    let m_b = postcard::to_allocvec(&m).unwrap();
    let delta = ByteDelta::diff(&base, &m, 6);
    println!("full: {} bytes, delta: {} bytes, ratio: {:.4}",
        m_b.len(), delta.serialized_size(),
        delta.serialized_size() as f64 / m_b.len() as f64);
    let changes = ByteDelta::diff_bytes(&base_b, &m_b, 6);
    let raw_sz: usize = changes.iter().map(|c| c.data.len()).sum();
    println!("raw-byte delta payload: {} bytes ({} chunks)", raw_sz, changes.len());

    // ── Extreme: 0 changed ──
    println!("\n--- Extreme: 0 of 10k bodies changed ---");
    let delta0 = ByteDelta::diff(&base, &base, 6);
    println!("delta serialized overhead: {} bytes", delta0.serialized_size());
}
