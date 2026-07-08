use bevy::prelude::*;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkTransformSample {
    pub tick: u32,
    pub translation: Vec3,
    pub rotation: Quat,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct NetworkTransformInterpolationBuffer {
    samples: VecDeque<NetworkTransformSample>,
    pub max_samples: usize,
    pub delay_ticks: u32,
    pub teleport_distance: f32,
}

impl NetworkTransformSample {
    pub fn new(tick: u32, translation: Vec3, rotation: Quat) -> Self {
        Self {
            tick,
            translation,
            rotation,
        }
    }
}

impl Default for NetworkTransformInterpolationBuffer {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(32),
            max_samples: 64,
            delay_ticks: 2,
            teleport_distance: 5.0,
        }
    }
}

impl NetworkTransformInterpolationBuffer {
    pub fn with_sample(sample: NetworkTransformSample) -> Self {
        let mut buffer = Self::default();
        buffer.samples.push_back(sample);
        buffer
    }

    pub fn push_sample(&mut self, sample: NetworkTransformSample) {
        while self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn interpolate(&self, now_tick: u32) -> Option<(Vec3, Quat)> {
        let render_tick = now_tick.saturating_sub(self.delay_ticks);
        if self.samples.len() < 2 {
            return self.samples.back().map(|s| (s.translation, s.rotation));
        }

        let sample_a = self.samples.front()?;
        let sample_b = self.samples.get(1)?;
        let t = if sample_b.tick > sample_a.tick {
            ((render_tick - sample_a.tick) as f32 / (sample_b.tick - sample_a.tick) as f32)
                .clamp(0.0, 1.0)
        } else {
            0.0
        };

        let translation = sample_a.translation.lerp(sample_b.translation, t);
        let rotation = sample_a.rotation.slerp(sample_b.rotation, t);
        Some((translation, rotation))
    }

    pub fn samples_len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Returns true if the given tick is newer than the latest sample, false
    /// otherwise.
    pub fn needs_newer_sample(&self, tick: u32) -> bool {
        self.samples.back().map_or(true, |s| s.tick < tick)
    }

    /// Get a sample at a specific index (used in physics_interactions tests).
    pub fn sample_at(&self, index: usize) -> Option<&NetworkTransformSample> {
        self.samples.get(index)
    }
}
