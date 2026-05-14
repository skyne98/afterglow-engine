pub mod core;
pub mod input;
pub mod network;
mod perf_hud;
pub mod persistence;
mod setup;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod world;

extern crate self as afterglow_engine;

pub use afterglow_engine_macros::{Replicate, replicate};
use bevy::{app::PluginGroupBuilder, prelude::*, window::WindowPlugin};
use core::{AfterglowCorePlugin, schedule::AfterglowSet};
use input::AfterglowInputPlugin;
use network::AfterglowNetworkPlugin;
use perf_hud::{
    AccumMap, PerfHudPlugin, collect_frame, record_update_end, record_update_start, setup_tracing,
    sync_shared_metrics, trace_collector::reset_trace_data, update_hud,
};
use persistence::AfterglowPersistencePlugin;
use world::AfterglowWorldPlugin;

pub struct AfterglowEnginePlugin {
    trace_accum: AccumMap,
}

impl Plugin for AfterglowEnginePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PerfHudPlugin {
            trace_accum: self.trace_accum.clone(),
        })
        .add_plugins(AfterglowCorePlugin)
        .add_plugins(AfterglowInputPlugin)
        .add_plugins(AfterglowNetworkPlugin)
        .add_plugins(AfterglowPersistencePlugin)
        .add_plugins(AfterglowWorldPlugin)
        .add_systems(
            Update,
            (
                record_update_start,
                setup::rotate_cubes,
                setup::update_light,
                collect_frame,
                update_hud,
                record_update_end,
                sync_shared_metrics,
            )
                .chain()
                .in_set(AfterglowSet::DebugAndMetrics),
        )
        .add_systems(
            Update,
            reset_trace_data
                .after(sync_shared_metrics)
                .in_set(AfterglowSet::DebugAndMetrics),
        );
    }
}

pub fn run() -> bevy::app::AppExit {
    let trace_data = setup_tracing();
    let trace_accum = trace_data.accum.clone();

    App::new()
        .insert_resource(trace_data)
        .add_plugins((default_plugins(), AfterglowEnginePlugin { trace_accum }))
        .run()
}

fn default_plugins() -> PluginGroupBuilder {
    let window = {
        #[cfg(target_arch = "wasm32")]
        {
            Window {
                fit_canvas_to_parent: true,
                ..default()
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            default()
        }
    };

    let render = {
        #[cfg(target_arch = "wasm32")]
        {
            bevy::render::settings::WgpuSettings {
                backends: Some(
                    bevy::render::settings::Backends::BROWSER_WEBGPU
                        | bevy::render::settings::Backends::GL,
                ),
                ..default()
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            default()
        }
    };

    let plugins = DefaultPlugins
        .set(bevy::log::LogPlugin {
            custom_layer: perf_hud::trace_collector::bevy_trace_layer,
            ..default()
        })
        .set(WindowPlugin {
            primary_window: Some(window),
            ..default()
        })
        .set(bevy::render::RenderPlugin {
            render_creation: bevy::render::settings::RenderCreation::Automatic(
                bevy::render::settings::WgpuSettings { ..render },
            ),
            ..default()
        })
        .build();

    #[cfg(target_arch = "wasm32")]
    let plugins = plugins
        .disable::<bevy::anti_alias::AntiAliasPlugin>()
        .disable::<bevy::audio::AudioPlugin>();

    plugins
}

#[cfg(test)]
mod tests {
    use crate::AfterglowEnginePlugin;
    use bevy::app::App;
    use std::sync::{Arc, Mutex};

    #[test]
    fn plugin_registers() {
        let mut app = App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            AfterglowEnginePlugin {
                trace_accum: Arc::new(Mutex::new(std::collections::HashMap::new())),
            },
        ));
    }
}
