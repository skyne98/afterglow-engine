pub mod controller;
pub mod core;
pub mod demo;
pub mod demos;
pub mod input;
pub mod network;
mod perf_hud;
pub mod persistence;
pub mod physics;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod units;
pub mod world;

extern crate self as afterglow_engine;

pub use afterglow_engine_macros::{Replicate, replicate};
use bevy::{app::PluginGroupBuilder, prelude::*, window::WindowPlugin};
use controller::AfterglowFirstPersonControllerPlugin;
use core::{AfterglowCorePlugin, schedule::AfterglowSet};
use demo::AfterglowDemoPlugin;
use input::AfterglowInputPlugin;
use network::AfterglowNetworkPlugin;
use perf_hud::{
    AccumMap, PerfHudPlugin, collect_frame, record_update_end, record_update_start, setup_tracing,
    sync_shared_metrics, trace_collector::reset_trace_data, update_hud,
};
use persistence::AfterglowPersistencePlugin;
use physics::AfterglowPhysicsPlugin;
use world::AfterglowWorldPlugin;

pub struct AfterglowRuntimePlugins;

pub struct AfterglowEnginePlugin {
    trace_accum: AccumMap,
}

impl PluginGroup for AfterglowRuntimePlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(AfterglowCorePlugin)
            .add(AfterglowInputPlugin)
            .add(AfterglowNetworkPlugin)
            .add(AfterglowPhysicsPlugin)
            .add(AfterglowFirstPersonControllerPlugin)
            .add(AfterglowPersistencePlugin)
            .add(AfterglowWorldPlugin)
    }
}

impl Plugin for AfterglowEnginePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(AfterglowRuntimePlugins)
            .add_plugins(PerfHudPlugin {
                trace_accum: self.trace_accum.clone(),
            })
            .add_systems(
                Update,
                (
                    record_update_start,
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
    run_default_demo()
}

pub fn run_default_demo() -> bevy::app::AppExit {
    let trace_data = setup_tracing();
    let trace_accum = trace_data.accum.clone();

    App::new()
        .insert_resource(trace_data)
        .add_plugins((
            default_plugins(),
            AfterglowEnginePlugin { trace_accum },
            AfterglowDemoPlugin,
        ))
        .run()
}

pub fn run_fps_controller_demo() -> bevy::app::AppExit {
    let trace_data = setup_tracing();
    let trace_accum = trace_data.accum.clone();

    App::new()
        .insert_resource(trace_data)
        .add_plugins((
            default_plugins(),
            AfterglowEnginePlugin { trace_accum },
            demos::fps_controller::FpsControllerDemoPlugin,
        ))
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
            filter: format!(
                "wgpu_hal::vulkan::instance=off,{}",
                bevy::log::DEFAULT_FILTER
            ),
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
    use crate::{AfterglowEnginePlugin, AfterglowRuntimePlugins};
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

    #[test]
    fn runtime_plugins_register_without_demo_content() {
        let mut app = App::new();
        app.add_plugins((bevy::MinimalPlugins, AfterglowRuntimePlugins));

        assert!(
            app.world()
                .resource::<crate::world::cell::CellManifestRegistry>()
                .chunks()
                .next()
                .is_none()
        );
        assert!(
            app.world()
                .resource::<crate::world::cell::CellLoadRequests>()
                .pending()
                .is_empty()
        );
    }
}
