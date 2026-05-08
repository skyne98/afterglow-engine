mod perf_hud;
mod setup;

use bevy::prelude::*;
use perf_hud::trace_collector::reset_trace_data;
use perf_hud::{collect_frame, record_update_end, record_update_start, setup_tracing, sync_shared_metrics, update_hud, AccumMap, PerfHudPlugin};

pub struct AfterglowEnginePlugin {
    trace_accum: AccumMap,
}

impl Plugin for AfterglowEnginePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PerfHudPlugin { trace_accum: self.trace_accum.clone() })
            .add_systems(Startup, setup::spawn_scene)
            .add_systems(Update, (
                record_update_start,
                setup::rotate_cubes,
                setup::update_light,
                collect_frame,
                update_hud,
                record_update_end,
                sync_shared_metrics,
            ).chain())
            .add_systems(Update, reset_trace_data.after(sync_shared_metrics));
    }
}

pub fn run() -> bevy::app::AppExit {
    let trace_data = setup_tracing();
    let trace_accum = trace_data.accum.clone();

    App::new()
        .insert_resource(trace_data)
        .add_plugins((DefaultPlugins, AfterglowEnginePlugin { trace_accum }))
        .run()
}

#[cfg(test)]
mod tests {
    use crate::AfterglowEnginePlugin;
    use bevy::app::App;
    use std::sync::{Arc, Mutex};

    #[test]
    fn plugin_registers() {
        let mut app = App::new();
        app.add_plugins((bevy::MinimalPlugins, AfterglowEnginePlugin {
            trace_accum: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }));
    }
}
