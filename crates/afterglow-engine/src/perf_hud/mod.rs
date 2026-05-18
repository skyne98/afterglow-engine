mod data;
mod server;
pub mod trace_collector;
mod ui;

use std::sync::{Arc, Mutex};

use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
};
use web_time::Instant;

pub use data::PerfData;
pub use server::SharedMetrics;
pub use trace_collector::{AccumMap, setup_tracing};
pub use ui::update_hud;

pub struct PerfHudPlugin {
    pub trace_accum: AccumMap,
}

impl Plugin for PerfHudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }

        let perf_data = PerfData::default();
        let shared = Arc::new(Mutex::new(perf_data));

        let port = std::env::var("AGX_METRICS_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(9877);

        server::start_metrics_server(port, shared.clone(), Some(self.trace_accum.clone()));

        app.init_resource::<PerfData>()
            .init_resource::<FrameProfiler>()
            .insert_resource(SharedMetrics(shared))
            .add_systems(Startup, ui::spawn_hud)
            .add_systems(
                PostUpdate,
                (record_postupdate_start, record_postupdate_end).chain(),
            );
    }
}

pub fn perf_hud_toggle_requested(keys: &ButtonInput<KeyCode>) -> bool {
    keys.just_pressed(KeyCode::Backquote)
        && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_hud_toggle_requires_backquote_and_shift() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::Backquote);

        assert!(perf_hud_toggle_requested(&keys));

        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::Backquote);
        assert!(!perf_hud_toggle_requested(&keys));
    }
}

#[derive(Resource, Default)]
pub struct FrameProfiler {
    pub update_start: Option<Instant>,
    pub postupdate_start: Option<Instant>,
}

pub fn record_update_start(mut profiler: ResMut<FrameProfiler>) {
    profiler.update_start = Some(Instant::now());
}

pub fn record_update_end(profiler: Res<FrameProfiler>, mut data: ResMut<PerfData>) {
    if let Some(start) = profiler.update_start {
        data.update_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

pub fn record_postupdate_start(mut profiler: ResMut<FrameProfiler>) {
    profiler.postupdate_start = Some(Instant::now());
}

pub fn record_postupdate_end(profiler: Res<FrameProfiler>, mut data: ResMut<PerfData>) {
    if let Some(start) = profiler.postupdate_start {
        data.extraction_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

pub fn sync_shared_metrics(data: Res<PerfData>, shared: Res<SharedMetrics>) {
    if let Ok(mut dst) = shared.0.try_lock() {
        dst.history = data.history.clone();
        dst.frame_systems = data.frame_systems.clone();
        dst.update_time_ms = data.update_time_ms;
        dst.extraction_time_ms = data.extraction_time_ms;
    }
}

pub fn collect_frame(mut data: ResMut<PerfData>, store: Res<DiagnosticsStore>) {
    let fps = store
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let ft = store
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.value())
        .unwrap_or(0.0);

    let systems = data.frame_systems.drain(..).collect();
    data.push(data::FrameSample {
        fps,
        frame_time_ms: ft,
        systems,
    });
}

pub fn record_system(data: &mut PerfData, name: &str, elapsed_ms: f64) {
    data.frame_systems.push((name.to_string(), elapsed_ms));
}
