use avian3d::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use core::time::Duration;
use serde::{Deserialize, Serialize};

// ── Snapshot types ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct PhysicsSnapshot {
    bodies: Vec<BodySnapshot>,
    joints: Vec<JointSnapshot>,
}

#[derive(Serialize, Deserialize)]
struct BodySnapshot {
    index: u32,
    rigidbody_type: RigidBodyTypeSnapshot,
    translation: [f32; 3],
    rotation: [f32; 4],
    linvel: [f32; 3],
    angvel: [f32; 3],
    collider: Option<ColliderSnapshot>,
}

#[derive(Serialize, Deserialize)]
enum RigidBodyTypeSnapshot {
    Dynamic,
    Static,
    Kinematic,
}

#[derive(Serialize, Deserialize)]
enum ColliderSnapshot {
    Cuboid {
        hx: f32,
        hy: f32,
        hz: f32,
    },
    Sphere {
        radius: f32,
    },
    Capsule {
        radius: f32,
        half_height: f32,
    },
}

#[derive(Serialize, Deserialize)]
struct JointSnapshot {
    body1_index: u32,
    body2_index: u32,
    kind: JointKindSnapshot,
}

#[derive(Serialize, Deserialize)]
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

// ── Serialization ─────────────────────────────────────────────────────────

fn take_snapshot(world: &mut World) -> PhysicsSnapshot {
    // Collect collider data by entity (clone to break borrows)
    let mut collider_map: std::collections::HashMap<Entity, ColliderSnapshot> =
        std::collections::HashMap::new();
    for (entity, collider) in world.query::<(Entity, &Collider)>().iter(&*world) {
        if let Some(snap) = try_snapshot_collider(collider) {
            collider_map.insert(entity, snap);
        }
    }

    // Collect body data
    let mut bodies: Vec<BodySnapshot> = Vec::new();
    let mut entity_to_index: Vec<(Entity, u32)> = Vec::new();

    for (entity, rb, pos, vel, angvel) in world
        .query::<(
            Entity,
            &RigidBody,
            &Transform,
            &LinearVelocity,
            &AngularVelocity,
        )>()
        .iter(world)
    {
        let idx = bodies.len() as u32;
        entity_to_index.push((entity, idx));

        let rb_type = match rb {
            RigidBody::Dynamic => RigidBodyTypeSnapshot::Dynamic,
            RigidBody::Static => RigidBodyTypeSnapshot::Static,
            RigidBody::Kinematic => RigidBodyTypeSnapshot::Kinematic,
        };

        bodies.push(BodySnapshot {
            index: idx,
            rigidbody_type: rb_type,
            translation: [pos.translation.x, pos.translation.y, pos.translation.z],
            rotation: [
                pos.rotation.x,
                pos.rotation.y,
                pos.rotation.z,
                pos.rotation.w,
            ],
            linvel: [vel.0.x, vel.0.y, vel.0.z],
            angvel: [angvel.0.x, angvel.0.y, angvel.0.z],
            collider: collider_map.remove(&entity),
        });
    }

    // Build a lookup from Entity → index
    let e2i: std::collections::HashMap<Entity, u32> = entity_to_index.into_iter().collect();

    // Collect joints
    let mut joints = Vec::new();

    for joint in world.query::<&SphericalJoint>().iter(&*world) {
        let Some(&i1) = e2i.get(&joint.body1) else {
            continue;
        };
        let Some(&i2) = e2i.get(&joint.body2) else {
            continue;
        };

        let a1 = joint.frame1.anchor;
        let a2 = joint.frame2.anchor;
        let anchor1 = match a1 {
            JointAnchor::Local(v) => [v.x, v.y, v.z],
            JointAnchor::FromGlobal(v) => [v.x, v.y, v.z],
        };
        let anchor2 = match a2 {
            JointAnchor::Local(v) => [v.x, v.y, v.z],
            JointAnchor::FromGlobal(v) => [v.x, v.y, v.z],
        };

        joints.push(JointSnapshot {
            body1_index: i1,
            body2_index: i2,
            kind: JointKindSnapshot::Spherical {
                anchor1,
                anchor2,
                twist_axis: [joint.twist_axis.x, joint.twist_axis.y, joint.twist_axis.z],
            },
        });
    }

    for joint in world.query::<&FixedJoint>().iter(&*world) {
        let Some(&i1) = e2i.get(&joint.body1) else {
            continue;
        };
        let Some(&i2) = e2i.get(&joint.body2) else {
            continue;
        };

        let a1 = joint.frame1.anchor;
        let a2 = joint.frame2.anchor;
        let anchor1 = match a1 {
            JointAnchor::Local(v) => [v.x, v.y, v.z],
            JointAnchor::FromGlobal(v) => [v.x, v.y, v.z],
        };
        let anchor2 = match a2 {
            JointAnchor::Local(v) => [v.x, v.y, v.z],
            JointAnchor::FromGlobal(v) => [v.x, v.y, v.z],
        };

        joints.push(JointSnapshot {
            body1_index: i1,
            body2_index: i2,
            kind: JointKindSnapshot::Fixed { anchor1, anchor2 },
        });
    }

    PhysicsSnapshot { bodies, joints }
}

fn try_snapshot_collider(collider: &Collider) -> Option<ColliderSnapshot> {
    let shape = collider.shape();
    if let Some(cub) = shape.as_cuboid() {
        let he = cub.half_extents;
        return Some(ColliderSnapshot::Cuboid {
            hx: he.x,
            hy: he.y,
            hz: he.z,
        });
    }
    if let Some(sph) = shape.as_ball() {
        return Some(ColliderSnapshot::Sphere {
            radius: sph.radius,
        });
    }
    if let Some(cap) = shape.as_capsule() {
        let half_height = cap.segment.a.y.abs().max(cap.segment.b.y.abs());
        return Some(ColliderSnapshot::Capsule {
            radius: cap.radius,
            half_height,
        });
    }
    None
}

// ── Restoration ───────────────────────────────────────────────────────────

fn apply_snapshot(snapshot: &PhysicsSnapshot, commands: &mut Commands) {
    // Phase 1: spawn all bodies, record their Entity IDs
    let mut index_to_entity: Vec<Entity> = Vec::with_capacity(snapshot.bodies.len());

    for body in &snapshot.bodies {
        let rb = match body.rigidbody_type {
            RigidBodyTypeSnapshot::Dynamic => RigidBody::Dynamic,
            RigidBodyTypeSnapshot::Static => RigidBody::Static,
            RigidBodyTypeSnapshot::Kinematic => RigidBody::Kinematic,
        };

        let mut cmd = commands.spawn((
            rb,
            Transform::from_xyz(
                body.translation[0],
                body.translation[1],
                body.translation[2],
            )
            .with_rotation(Quat::from_xyzw(
                body.rotation[0],
                body.rotation[1],
                body.rotation[2],
                body.rotation[3],
            )),
            LinearVelocity(Vec3::new(body.linvel[0], body.linvel[1], body.linvel[2])),
            AngularVelocity(Vec3::new(
                body.angvel[0],
                body.angvel[1],
                body.angvel[2],
            )),
        ));

        if let Some(ref collider) = body.collider {
            let col = match collider {
                ColliderSnapshot::Cuboid { hx, hy, hz } => {
                    Collider::cuboid(hx * 2.0, hy * 2.0, hz * 2.0)
                }
                ColliderSnapshot::Sphere { radius } => Collider::sphere(*radius),
                ColliderSnapshot::Capsule { radius, half_height } => {
                    Collider::capsule(*radius, half_height * 2.0)
                }
            };
            cmd.insert(col);
        }

        index_to_entity.push(cmd.id());
    }

    // Phase 2: spawn joints using remapped Entity IDs
    for joint in &snapshot.joints {
        let e1 = index_to_entity[joint.body1_index as usize];
        let e2 = index_to_entity[joint.body2_index as usize];

        match &joint.kind {
            JointKindSnapshot::Spherical {
                anchor1,
                anchor2,
                twist_axis,
            } => {
                commands.spawn(
                    SphericalJoint::new(e1, e2)
                        .with_local_anchor1(Vec3::new(anchor1[0], anchor1[1], anchor1[2]))
                        .with_local_anchor2(Vec3::new(anchor2[0], anchor2[1], anchor2[2]))
                        .with_twist_axis(Vec3::new(twist_axis[0], twist_axis[1], twist_axis[2])),
                );
            }
            JointKindSnapshot::Fixed { anchor1, anchor2 } => {
                commands.spawn(
                    FixedJoint::new(e1, e2)
                        .with_local_anchor1(Vec3::new(anchor1[0], anchor1[1], anchor1[2]))
                        .with_local_anchor2(Vec3::new(anchor2[0], anchor2[1], anchor2[2])),
                );
            }
        }
    }
}

// ── Hash ──────────────────────────────────────────────────────────────────

fn hash_world(world: &mut World) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut query = world.query::<(&RigidBody, &Transform)>();
    for (_, tf) in query.iter(&*world) {
        let t = tf.translation;
        let r = tf.rotation;
        (t.x.to_bits(), t.y.to_bits(), t.z.to_bits()).hash(&mut hasher);
        (r.x.to_bits(), r.y.to_bits(), r.z.to_bits(), r.w.to_bits()).hash(&mut hasher);
    }
    hasher.finish()
}

// ── Verification ──────────────────────────────────────────────────────────

fn build_test_world(total: u32) -> App {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
    app.add_plugins(PhysicsPlugins::new(FixedUpdate));
    app.insert_resource(BodyCount(total));
    app.add_systems(Startup, spawn_scene);

    app.update();

    // Settle physics for 1 step
    app.world_mut()
        .resource_mut::<Time<Physics>>()
        .advance_by(Duration::from_secs_f64(1.0 / 60.0));
    let _ = app.world_mut().run_schedule(FixedUpdate);

    app
}

fn run_n_steps(app: &mut App, n: u32) {
    let dt = Duration::from_secs_f64(1.0 / 60.0);
    for _ in 0..n {
        app.world_mut()
            .resource_mut::<Time<Physics>>()
            .advance_by(dt);
        let _ = app.world_mut().run_schedule(FixedUpdate);
    }
}

fn main() {
    let total: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let pre_steps: u32 = 40;
    let post_steps: u32 = 60;

    // ── Phase 1: run forward, snapshot, run more, record hash ──
    let mut app_a = build_test_world(total);
    run_n_steps(&mut app_a, pre_steps);

    let snapshot = take_snapshot(app_a.world_mut());

    let serialized = postcard::to_allocvec(&snapshot).unwrap();
    eprintln!(
        "snapshot: {} bodies, {} joints, {} bytes",
        snapshot.bodies.len(),
        snapshot.joints.len(),
        serialized.len(),
    );

    run_n_steps(&mut app_a, post_steps);
    let hash_a = hash_world(app_a.world_mut());
    eprintln!("reference hash after {} extra steps: {hash_a:016x}", post_steps);

    // ── Phase 2: restore from snapshot, run same steps, compare hash ──
    let mut app_b = App::new();
    app_b.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
    app_b.add_plugins(PhysicsPlugins::new(FixedUpdate));
    app_b.add_systems(Startup, move |mut commands: Commands| {
        let snapshot: PhysicsSnapshot = postcard::from_bytes(&serialized).unwrap();
        apply_snapshot(&snapshot, &mut commands);
    });

    app_b.update();

    // Settle for 1 step (same as build_test_world)
    app_b
        .world_mut()
        .resource_mut::<Time<Physics>>()
        .advance_by(Duration::from_secs_f64(1.0 / 60.0));
    let _ = app_b.world_mut().run_schedule(FixedUpdate);

    run_n_steps(&mut app_b, post_steps);
    let hash_b = hash_world(app_b.world_mut());
    eprintln!("restored  hash after {} extra steps: {hash_b:016x}", post_steps);

    // ── Result ──
    if hash_a == hash_b {
        eprintln!("SERIALIZE ROUND-TRIP: CONFIRMED — hashes match");
    } else {
        eprintln!("SERIALIZE ROUND-TRIP: FAILED — hashes differ");
        std::process::exit(1);
    }
}

// ── Scene spawner ─────────────────────────────────────────────────────────

#[derive(Resource)]
struct BodyCount(u32);

fn spawn_scene(mut commands: Commands, count: Res<BodyCount>) {
    commands.spawn((
        Collider::cuboid(500.0, 1.0, 500.0),
        Transform::from_xyz(0.0, -0.5, 0.0),
    ));

    let total = count.0;
    let chain_count = (total / 10).max(1).min(200);
    let chains = chain_count;
    let links = 10;
    let jointed = chains * links;
    let extra = total.saturating_sub(jointed);

    for ci in 0..chains {
        let base_x = (ci as f32 - chains as f32 / 2.0) * 3.0;
        let mut prev: Option<Entity> = None;
        for li in 0..links {
            let y = 1.0 + li as f32 * 1.1;
            let e = commands
                .spawn((
                    RigidBody::Dynamic,
                    Collider::cuboid(0.4, 0.4, 0.4),
                    Transform::from_xyz(base_x, y, 0.0),
                ))
                .id();
            if let Some(prev) = prev {
                commands.spawn(
                    SphericalJoint::new(prev, e)
                        .with_local_anchor1(Vec3::new(0.0, 0.5, 0.0))
                        .with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)),
                );
            }
            prev = Some(e);
        }
    }

    if extra > 0 {
        let grid = (extra as f32).sqrt().ceil() as i32;
        let spacing = 2.5;
        let x_offset = -(chains as f32 / 2.0) * 3.0 - 20.0 - 10.0;
        for i in 0..extra {
            let x = (i % grid as u32) as f32 * spacing + x_offset;
            let z = (i / grid as u32) as f32 * spacing - (grid as f32 * spacing) / 2.0;
            commands.spawn((
                RigidBody::Dynamic,
                Collider::cuboid(0.5, 0.5, 0.5),
                Transform::from_xyz(x, 5.0 + i as f32 * 0.01, z),
            ));
        }
    }
}
