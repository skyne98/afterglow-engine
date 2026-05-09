use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bevy::{app::App, log::BoxedLayer, prelude::*};
use serde::Serialize;
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};
use web_time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct SpanSample {
    pub name: String,
    pub duration_ms: f64,
    pub count: u32,
}

/// Shared accum map: name → (total_ms, count)
pub type AccumMap = Arc<Mutex<HashMap<String, (f64, u32)>>>;

#[derive(Resource)]
pub struct TraceData {
    pub accum: AccumMap,
}

pub struct TraceCollectorLayer {
    accum: AccumMap,
    enter_times: Mutex<HashMap<tracing::span::Id, Instant>>,
}

impl<S> Layer<S> for TraceCollectorLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &tracing::span::Id, _ctx: Context<'_, S>) {
        if let Ok(mut times) = self.enter_times.lock() {
            times.insert(id.clone(), Instant::now());
        }
    }

    fn on_exit(&self, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let name = match ctx.span(id).map(|s| s.name().to_string()) {
            Some(n) => n,
            None => return,
        };
        let start = match self.enter_times.lock().ok().and_then(|mut t| t.remove(id)) {
            Some(s) => s,
            None => return,
        };
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if elapsed < 0.001 {
            return;
        }
        if let Ok(mut acc) = self.accum.lock() {
            let (total, count) = acc.entry(name).or_insert((0.0, 0));
            *total += elapsed;
            *count += 1;
        }
    }
}

pub fn setup_tracing() -> TraceData {
    TraceData {
        accum: Arc::new(Mutex::new(HashMap::new())),
    }
}

pub(crate) fn bevy_trace_layer(app: &mut App) -> Option<BoxedLayer> {
    let accum = app.world().get_resource::<TraceData>()?.accum.clone();
    Some(Box::new(TraceCollectorLayer {
        accum,
        enter_times: Mutex::new(HashMap::new()),
    }))
}

pub fn reset_trace_data(trace: Res<TraceData>) {
    // Flush accum sorted into spans, then hand off to shared state.
    // The metrics server reads the accum directly so no extra copy needed.
    // Just clear for the next frame.
    if let Ok(mut acc) = trace.accum.lock() {
        acc.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceData, bevy_trace_layer, setup_tracing};
    use bevy::app::App;

    #[test]
    fn bevy_trace_layer_requires_trace_data_resource() {
        let mut app = App::new();
        assert!(bevy_trace_layer(&mut app).is_none());
    }

    #[test]
    fn bevy_trace_layer_uses_existing_trace_data_resource() {
        let mut app = App::new();
        app.insert_resource(setup_tracing());
        assert!(app.world().contains_resource::<TraceData>());
        assert!(bevy_trace_layer(&mut app).is_some());
    }
}
