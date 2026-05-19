use avian3d::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use core::time::Duration;

fn main() {
    let total: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let steps: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);

    // Run simulation twice from seed
    let hash_a = simulate(total, steps);
    let hash_b = simulate(total, steps);

    eprintln!("run 1 hash: {hash_a:016x}");
    eprintln!("run 2 hash: {hash_b:016x}");

    if hash_a == hash_b {
        eprintln!("DETERMINISM: CONFIRMED — bit-identical across two runs");
    } else {
        eprintln!("DETERMINISM: FAILED — hashes differ");
        std::process::exit(1);
    }
}

fn simulate(total: u32, steps: u32) -> u64 {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
    app.add_plugins(PhysicsPlugins::new(FixedUpdate));
    app.insert_resource(BodyCount(total));
    app.add_systems(Startup, spawn_scene);

    // Spawn + 1 physics tick to settle
    app.update();
    app.world_mut()
        .resource_mut::<Time<Physics>>()
        .advance_by(Duration::from_secs_f64(1.0 / 60.0));
    let _ = app.world_mut().run_schedule(FixedUpdate);

    let dt = Duration::from_secs_f64(1.0 / 60.0);
    for _ in 0..steps {
        app.world_mut()
            .resource_mut::<Time<Physics>>()
            .advance_by(dt);
        let _ = app.world_mut().run_schedule(FixedUpdate);
    }

    // Hash final transforms of all dynamic bodies via raw bytes
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let world = app.world_mut();
    let mut query = world.query::<(&RigidBody, &Transform)>();
    for (_, tf) in query.iter(world) {
        let t = tf.translation;
        let r = tf.rotation;
        (t.x.to_bits(), t.y.to_bits(), t.z.to_bits()).hash(&mut hasher);
        (r.x.to_bits(), r.y.to_bits(), r.z.to_bits(), r.w.to_bits()).hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Resource)]
struct BodyCount(u32);

fn spawn_scene(mut commands: Commands, count: Res<BodyCount>) {
    commands.spawn((
        Collider::cuboid(500.0, 1.0, 500.0),
        Transform::from_xyz(0.0, -0.5, 0.0),
    ));

    let total = count.0;
    let chain_count = (total / 10).max(1).min(2000);
    let chains = chain_count;
    let links = 10;
    let extra = total.saturating_sub(chains * links);

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
                commands.spawn(SphericalJoint::new(prev, e)
                    .with_local_anchor1(Vec3::new(0.0, 0.5, 0.0))
                    .with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)));
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
