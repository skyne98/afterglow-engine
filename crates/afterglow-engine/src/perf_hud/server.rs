use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use bevy::prelude::*;
use serde::Serialize;

use super::{
    data::{PerfData, SystemStats},
    trace_collector::{AccumMap, SpanSample},
};

#[derive(Serialize)]
struct MetricsResponse {
    fps: FpsMetrics,
    frame_time: FtMetrics,
    systems: Vec<SysMetrics>,
    update_time_ms: f64,
    render_time_ms: f64,
    extraction_time_ms: f64,
    trace_spans: Vec<SpanSample>,
}

#[derive(Serialize)]
struct FpsMetrics {
    current: u64,
    min: u64,
    max: u64,
    avg: f64,
    p5: u64,
    p1: u64,
}

#[derive(Serialize)]
struct FtMetrics {
    current: f64,
    avg: f64,
    p95: f64,
    p99: f64,
    refresh_hz: u64,
}

#[derive(Serialize)]
struct SysMetrics {
    name: String,
    avg: f64,
    p95: f64,
    p99: f64,
}

#[derive(Resource)]
pub struct SharedMetrics(pub Arc<Mutex<PerfData>>);

pub fn start_metrics_server(
    port: u16,
    shared: Arc<Mutex<PerfData>>,
    trace_accum: Option<AccumMap>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    spawn_metrics_server(port, shared, trace_accum);

    #[cfg(target_arch = "wasm32")]
    let _ = (port, shared, trace_accum);
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_metrics_server(port: u16, shared: Arc<Mutex<PerfData>>, trace_accum: Option<AccumMap>) {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();

    thread::spawn(move || {
        let addr = format!("0.0.0.0:{port}");
        let server = match tiny_http::Server::http(&addr) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to start metrics server on {addr}: {e}");
                return;
            }
        };
        loop {
            if !flag.load(Ordering::Relaxed) {
                break;
            }
            match server.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(Some(request)) => {
                    let url = request.url().to_string();
                    if url == "/metrics" || url == "/" || url == "/traces" {
                        let Ok(data) = shared.lock() else {
                            let _ = request.respond(
                                tiny_http::Response::from_string("metrics lock failed")
                                    .with_status_code(503),
                            );
                            continue;
                        };
                        let top_spans = trace_accum
                            .as_ref()
                            .and_then(|a| a.lock().ok())
                            .map(|acc| {
                                let mut spans: Vec<_> = acc
                                    .iter()
                                    .map(|(name, (total, count))| {
                                        super::trace_collector::SpanSample {
                                            name: name.clone(),
                                            duration_ms: *total,
                                            count: *count,
                                        }
                                    })
                                    .collect();
                                spans.sort_by(|a, b| b.duration_ms.total_cmp(&a.duration_ms));
                                if url != "/traces" {
                                    spans.truncate(15);
                                }
                                spans
                            })
                            .unwrap_or_default();
                        let resp = build_response(&data, &top_spans);
                        let json = serde_json::to_string(&resp).unwrap_or_default();
                        let Ok(content_type) =
                            "Content-Type: application/json".parse::<tiny_http::Header>()
                        else {
                            let _ = request.respond(
                                tiny_http::Response::from_string("internal error")
                                    .with_status_code(500),
                            );
                            continue;
                        };
                        let response =
                            tiny_http::Response::from_string(json).with_header(content_type);
                        let _ = request.respond(response);
                    } else {
                        let response =
                            tiny_http::Response::from_string("not found").with_status_code(404);
                        let _ = request.respond(response);
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });
}

fn build_response(data: &PerfData, trace_spans: &[SpanSample]) -> MetricsResponse {
    let fpss: Vec<f64> = data.history.iter().map(|s| s.fps).collect();
    let cur_fps = data.history.last().map(|s| s.fps as u64).unwrap_or(0);
    let min_fps = fpss.iter().cloned().fold(f64::MAX, f64::min) as u64;
    let max_fps = fpss.iter().cloned().fold(0.0f64, f64::max) as u64;
    let avg_fps = fpss.iter().sum::<f64>() / fpss.len().max(1) as f64;

    let mut sfps = fpss.clone();
    sfps.sort_unstable_by(|a, b| a.total_cmp(b));
    let p5 = sfps
        .get((sfps.len() as f64 * 0.05) as usize)
        .copied()
        .unwrap_or(0.0) as u64;
    let p1 = sfps
        .get((sfps.len() as f64 * 0.01) as usize)
        .copied()
        .unwrap_or(0.0) as u64;

    let fts: Vec<f64> = data.history.iter().map(|s| s.frame_time_ms).collect();
    let cur_ft = data.history.last().map(|s| s.frame_time_ms).unwrap_or(0.0);
    let avg_ft = fts.iter().sum::<f64>() / fts.len().max(1) as f64;
    let mut sft = fts.clone();
    sft.sort_unstable_by(|a, b| a.total_cmp(b));
    let p95_ft = sft
        .get((sft.len() as f64 * 0.95) as usize)
        .copied()
        .unwrap_or(0.0);
    let p99_ft = sft
        .get((sft.len() as f64 * 0.99) as usize)
        .copied()
        .unwrap_or(0.0);

    let top = data.top_systems_sorted();
    let systems: Vec<SysMetrics> = top
        .iter()
        .map(|s: &SystemStats| SysMetrics {
            name: s.name.clone(),
            avg: s.avg,
            p95: s.p95,
            p99: s.p99,
        })
        .collect();

    let update_ms = data.update_time_ms;
    let extraction_ms = data.extraction_time_ms;
    let total_ms = cur_ft;
    let render_ms = (total_ms - update_ms - extraction_ms).max(0.0);

    MetricsResponse {
        fps: FpsMetrics {
            current: cur_fps,
            min: min_fps,
            max: max_fps,
            avg: avg_fps,
            p5,
            p1,
        },
        frame_time: FtMetrics {
            current: cur_ft,
            avg: avg_ft,
            p95: p95_ft,
            p99: p99_ft,
            refresh_hz: 60,
        },
        systems,
        update_time_ms: update_ms,
        extraction_time_ms: extraction_ms,
        render_time_ms: render_ms,
        trace_spans: trace_spans.to_vec(),
    }
}
