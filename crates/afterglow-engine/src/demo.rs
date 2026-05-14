use bevy::prelude::*;
use web_time::Instant;

use crate::{
    core::schedule::AfterglowSet,
    perf_hud::{self, PerfData},
    world::cell::{CellLoadRequests, CellManifestRegistry, DEMO_CELL_CHUNK, demo_cell_manifest},
};

pub struct AfterglowDemoPlugin;

#[derive(Component)]
pub struct Rotates {
    pub speed: f32,
}

impl Plugin for AfterglowDemoPlugin {
    fn build(&self, app: &mut App) {
        install_demo_cell(app);
        app.add_systems(
            Update,
            (rotate_cubes, update_light)
                .chain()
                .in_set(AfterglowSet::DebugAndMetrics),
        );
    }
}

fn install_demo_cell(app: &mut App) {
    app.init_resource::<CellManifestRegistry>()
        .init_resource::<CellLoadRequests>();
    app.world_mut()
        .resource_mut::<CellManifestRegistry>()
        .insert(demo_cell_manifest())
        .expect("built-in demo cell manifest is valid");
    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(DEMO_CELL_CHUNK)
        .expect("built-in demo cell chunk is valid");
}

fn rotate_cubes(
    time: Res<Time>,
    mut query: Query<(&Rotates, &mut Transform)>,
    data: Option<ResMut<PerfData>>,
) {
    let start = Instant::now();
    for (r, mut t) in &mut query {
        t.rotate_y(r.speed * time.delta_secs());
    }
    record_optional_system(data, "rotate", start);
}

fn update_light(
    time: Res<Time>,
    mut query: Query<&mut Transform, With<PointLight>>,
    data: Option<ResMut<PerfData>>,
) {
    let start = Instant::now();
    for mut t in &mut query {
        t.translation.x = 3.0 * time.elapsed_secs().cos();
        t.translation.z = 2.0 * time.elapsed_secs().sin();
    }
    record_optional_system(data, "light", start);
}

fn record_optional_system(data: Option<ResMut<PerfData>>, name: &str, start: Instant) {
    if let Some(mut data) = data {
        perf_hud::record_system(&mut data, name, start.elapsed().as_secs_f64() * 1000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::AfterglowCorePlugin,
        persistence::AfterglowPersistencePlugin,
        world::{AfterglowWorldPlugin, cell::DEMO_CUBE_ID},
    };

    #[test]
    fn demo_plugin_installs_demo_cell_explicitly() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            AfterglowCorePlugin,
            AfterglowPersistencePlugin,
            AfterglowWorldPlugin,
            AfterglowDemoPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

        app.update();
        app.update();

        let registry = app
            .world()
            .resource::<crate::core::identity::StableEntityRegistry>();
        assert!(registry.entity(DEMO_CUBE_ID).is_some());
    }
}
