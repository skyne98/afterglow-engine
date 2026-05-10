use bevy::prelude::*;
use web_time::Instant;

use crate::perf_hud;

#[derive(Component)]
pub struct Rotates {
    pub speed: f32,
}

pub fn rotate_cubes(
    time: Res<Time>,
    mut query: Query<(&Rotates, &mut Transform)>,
    mut data: ResMut<perf_hud::PerfData>,
) {
    let start = Instant::now();
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
    let start = Instant::now();
    for mut t in &mut query {
        t.translation.x = 3.0 * time.elapsed_secs().cos();
        t.translation.z = 2.0 * time.elapsed_secs().sin();
    }
    perf_hud::record_system(&mut data, "light", start.elapsed().as_secs_f64() * 1000.0);
}
