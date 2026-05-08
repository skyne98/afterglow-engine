use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::prelude::*;

use crate::perf_hud;

pub(super) fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(1.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.6, 1.0),
            ..default()
        })),
        Rotates { speed: 0.5 },
    ));

    commands.spawn((
        PointLight {
            intensity: 2_000_000.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 2.0),
    ));

    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        TemporalAntiAliasing::default(),
        Transform::from_xyz(-3.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[derive(Component)]
pub struct Rotates {
    speed: f32,
}

pub fn rotate_cubes(
    time: Res<Time>,
    mut query: Query<(&Rotates, &mut Transform)>,
    mut data: ResMut<perf_hud::PerfData>,
) {
    let start = std::time::Instant::now();
    for (r, mut t) in &mut query {
        t.rotate_y(r.speed * time.delta_secs());
    }
    perf_hud::record_system(&mut data, "rotate", start.elapsed().as_secs_f64() * 1000.0);
}

pub fn update_light(
    time: Res<Time>,
    mut query: Query<&mut Transform, With<PointLight>>,
    mut data: ResMut<perf_hud::PerfData>,
) {
    let start = std::time::Instant::now();
    for mut t in &mut query {
        t.translation.x = 3.0 * time.elapsed_secs().cos();
        t.translation.z = 2.0 * time.elapsed_secs().sin();
    }
    perf_hud::record_system(&mut data, "light", start.elapsed().as_secs_f64() * 1000.0);
}
