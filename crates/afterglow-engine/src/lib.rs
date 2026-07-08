pub mod console;
pub mod controller;
pub mod core;
pub mod demo;
pub mod demos;
pub mod input;
pub mod network;
mod perf_hud;
pub mod physics;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod units;
extern crate self as afterglow_engine;

use bevy::{app::PluginGroupBuilder, prelude::*, window::WindowPlugin, winit::WinitSettings};
use console::DevConsolePlugin;
use controller::AfterglowFirstPersonControllerPlugin;
use core::{AfterglowCorePlugin, schedule::AfterglowSet};
use demo::AfterglowDemoPlugin;
use input::AfterglowInputPlugin;
use network::AfterglowNetworkPlugin;
use perf_hud::{
    AccumMap, PerfHudPlugin, collect_frame, record_update_end, record_update_start, setup_tracing,
    sync_shared_metrics, trace_collector::reset_trace_data, update_hud,
};
use physics::AfterglowPhysicsPlugin;

pub struct AfterglowRuntimePlugins;

pub struct AfterglowEnginePlugin {
    trace_accum: AccumMap,
}

impl PluginGroup for AfterglowRuntimePlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(AfterglowCorePlugin)
            .add(DevConsolePlugin)
            .add(AfterglowNetworkPlugin)
            .add(AfterglowInputPlugin)
            .add(AfterglowPhysicsPlugin)
            .add(AfterglowFirstPersonControllerPlugin)
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

    let mut app = App::new();
    keep_windowed_runtime_unthrottled_when_unfocused(&mut app);
    app.insert_resource(trace_data).add_plugins((
        default_plugins(),
        AfterglowEnginePlugin { trace_accum },
        AfterglowDemoPlugin,
    ));
    app.run()
}

pub fn run_fps_controller_demo() -> bevy::app::AppExit {
    let trace_data = setup_tracing();
    let trace_accum = trace_data.accum.clone();

    let mut app = App::new();
    keep_windowed_runtime_unthrottled_when_unfocused(&mut app);
    app.insert_resource(trace_data).add_plugins((
        default_plugins(),
        AfterglowEnginePlugin { trace_accum },
        demos::fps_controller::FpsControllerDemoPlugin,
    ));
    app.run()
}

#[cfg(feature = "multiplayer")]
pub fn run_multiplayer_boxes_demo(
    config: demos::multiplayer_boxes::MultiplayerBoxesDemoConfig,
) -> bevy::app::AppExit {
    demos::multiplayer_boxes::run_multiplayer_boxes_demo(config)
}

/// Server App for multiplayer boxes demo. Runs in a detached thread.
#[cfg(feature = "multiplayer")]
pub fn run_multiplayer_boxes_server(listen: &str) {
    demos::multiplayer_boxes::run_multiplayer_boxes_server(listen);
}

/// Client App for multiplayer boxes demo.
#[cfg(feature = "multiplayer")]
pub fn run_multiplayer_boxes_client(connect: &str, player_name: &str) -> bevy::app::AppExit {
    demos::multiplayer_boxes::run_multiplayer_boxes_client(connect, player_name)
}

fn keep_windowed_runtime_unthrottled_when_unfocused(app: &mut App) {
    app.insert_resource(WinitSettings::continuous());
}

struct RequireHardwareRendererPlugin;

impl Plugin for RequireHardwareRendererPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, reject_software_renderer);
    }
}

fn is_software_renderer(device_type: &str, adapter_name: &str) -> bool {
    let adapter_name = adapter_name.to_ascii_lowercase();
    device_type == "Cpu"
        || adapter_name.contains("llvmpipe")
        || adapter_name.contains("lavapipe")
        || adapter_name.contains("softpipe")
        || adapter_name.contains("swiftshader")
}

fn reject_software_renderer(adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>) {
    let Some(adapter) = adapter else {
        return;
    };
    let device_type = format!("{:?}", adapter.device_type);

    if is_software_renderer(&device_type, &adapter.name)
        && std::env::var_os("AFTERGLOW_ALLOW_SOFTWARE_RENDERER").is_none()
    {
        panic!(
            "Afterglow selected a software renderer ({:?}). This is CPU rendering, not GPU rendering. \
Install/fix your GPU driver, make the GPU visible to Vulkan/WGPU, or explicitly select an adapter \
with WGPU_ADAPTER_NAME. Set AFTERGLOW_ALLOW_SOFTWARE_RENDERER=1 only for intentional CPU-rendered tests.",
            **adapter
        );
    }
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
                bevy::render::settings::WgpuSettings {
                    power_preference: bevy::render::settings::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    ..render
                },
            ),
            ..default()
        })
        .build()
        .add(RequireHardwareRendererPlugin);

    #[cfg(target_arch = "wasm32")]
    let plugins = plugins
        .disable::<bevy::anti_alias::AntiAliasPlugin>()
        .disable::<bevy::audio::AudioPlugin>();

    plugins
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "lightyear")]
    use crate::AfterglowRuntimePlugins;
    use crate::{AfterglowEnginePlugin, keep_windowed_runtime_unthrottled_when_unfocused};
    use bevy::{
        app::App,
        winit::{UpdateMode, WinitSettings},
    };
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
    fn software_renderer_detector_rejects_llvmpipe_and_cpu_adapters() {
        assert!(crate::is_software_renderer("Cpu", "llvmpipe"));
        assert!(crate::is_software_renderer("DiscreteGpu", "SwiftShader"));
        assert!(!crate::is_software_renderer(
            "DiscreteGpu",
            "NVIDIA GeForce"
        ));
        assert!(!crate::is_software_renderer("IntegratedGpu", "AMD RADV"));
    }

    #[test]
    fn windowed_runtime_does_not_throttle_when_unfocused() {
        let mut app = App::new();
        keep_windowed_runtime_unthrottled_when_unfocused(&mut app);

        let settings = app.world().resource::<WinitSettings>();
        assert_eq!(settings.focused_mode, UpdateMode::Continuous);
        assert_eq!(settings.unfocused_mode, UpdateMode::Continuous);
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn runtime_plugins_do_not_double_install_leafwing_with_bevy_input() {
        let mut app = App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            bevy::input::InputPlugin,
            AfterglowRuntimePlugins,
        ));

        app.update();
    }
}
