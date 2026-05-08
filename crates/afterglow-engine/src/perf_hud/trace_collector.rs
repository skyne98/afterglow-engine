use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::prelude::*;
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

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
    let accum: AccumMap = Arc::new(Mutex::new(HashMap::new()));
    let layer = TraceCollectorLayer {
        accum: accum.clone(),
        enter_times: Mutex::new(HashMap::new()),
    };

    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::Layer::default().with_writer(std::io::stderr))
        .with(layer);

    match tracing::subscriber::set_global_default(subscriber) {
        Ok(()) => eprintln!("[TraceCollectorLayer] installed"),
        Err(e) => eprintln!("[TraceCollectorLayer] FAILED: {e:?}"),
    }

    TraceData { accum }
}

pub fn reset_trace_data(trace: Res<TraceData>) {
    // Flush accum sorted into spans, then hand off to shared state.
    // The metrics server reads the accum directly so no extra copy needed.
    // Just clear for the next frame.
    if let Ok(mut acc) = trace.accum.lock() {
        acc.clear();
    }
}
