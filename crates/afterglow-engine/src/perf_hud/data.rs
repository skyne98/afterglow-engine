use bevy::prelude::*;
use std::collections::HashMap;

const MAX_HISTORY: usize = 60;

#[derive(Clone)]
pub struct FrameSample {
    pub fps: f64,
    pub frame_time_ms: f64,
    pub systems: Vec<(String, f64)>,
}

pub struct PerfData {
    pub history: Vec<FrameSample>,
    pub frame_systems: Vec<(String, f64)>,
    pub trace_snapshots: Vec<Vec<(String, f64)>>,
    pub update_time_ms: f64,
    pub extraction_time_ms: f64,
    pub name_colors: HashMap<String, usize>,
    pub next_color: usize,
    pub smoothed_trace_max: f32,
}

impl Resource for PerfData {}

impl Default for PerfData {
    fn default() -> Self {
        Self {
            history: Vec::with_capacity(MAX_HISTORY),
            frame_systems: Vec::new(),
            trace_snapshots: Vec::with_capacity(MAX_HISTORY),
            update_time_ms: 0.0,
            extraction_time_ms: 0.0,
            name_colors: HashMap::new(),
            next_color: 0,
            smoothed_trace_max: 0.001,
        }
    }
}

impl PerfData {
    pub fn push(&mut self, sample: FrameSample) {
        self.history.push(sample);
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }

    pub fn top_systems_sorted(&self) -> Vec<SystemStats> {
        let mut acc: Vec<(String, Vec<f64>)> = Vec::new();
        for sample in &self.history {
            for (name, time) in &sample.systems {
                if let Some((_, times)) = acc.iter_mut().find(|(n, _)| n == name) {
                    times.push(*time);
                } else {
                    acc.push((name.clone(), vec![*time]));
                }
            }
        }

        let mut result: Vec<SystemStats> = acc
            .into_iter()
            .map(|(name, mut times)| {
                times.sort_unstable_by(|a, b| a.total_cmp(b));
                let avg = times.iter().sum::<f64>() / times.len() as f64;
                let p95 = percentile(&times, 95.0);
                let p99 = percentile(&times, 99.0);
                SystemStats {
                    name,
                    avg,
                    p95,
                    p99,
                }
            })
            .collect();

        result.sort_by(|a, b| a.name.cmp(&b.name));
        result.truncate(5);
        result
    }
}

#[derive(Clone)]
pub struct SystemStats {
    pub name: String,
    pub avg: f64,
    pub p95: f64,
    pub p99: f64,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    let idx = ((n as f64) * p / 100.0).ceil() as usize - 1;
    sorted[idx.clamp(0, n - 1)]
}
