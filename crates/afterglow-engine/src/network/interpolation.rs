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
            samples: VecDeque::new(),
            max_samples: 32,
            delay_ticks: 2,
            teleport_distance: 3.0,
        }
    }
}

impl NetworkTransformInterpolationBuffer {
    pub fn with_sample(sample: NetworkTransformSample) -> Self {
        let mut buffer = Self::default();
        buffer.push_sample(sample);
        buffer
    }

    pub fn push_sample(&mut self, sample: NetworkTransformSample) {
        if self.should_reset_for_discontinuity(sample) {
            self.samples.clear();
        }
        if let Some(existing) = self
            .samples
            .iter()
            .position(|existing| existing.tick == sample.tick)
        {
            self.samples[existing] = sample;
            return;
        }
        let insert_at = self
            .samples
            .iter()
            .position(|existing| sample.tick < existing.tick)
            .unwrap_or(self.samples.len());
        self.samples.insert(insert_at, sample);
        while self.samples.len() > self.max_samples.max(1) {
            self.samples.pop_front();
        }
    }

    pub fn sample_delayed(&self, latest_tick: u32) -> Option<NetworkTransformSample> {
        self.sample_at(latest_tick.saturating_sub(self.delay_ticks))
    }

    pub fn sample_at(&self, render_tick: u32) -> Option<NetworkTransformSample> {
        let first = self.samples.front().copied()?;
        if render_tick <= first.tick {
            return Some(first);
        }
        for index in 0..self.samples.len().saturating_sub(1) {
            let a = self.samples[index];
            let b = self.samples[index + 1];
            if render_tick == a.tick {
                return Some(a);
            }
            if render_tick < b.tick {
                return Some(interpolate_sample(a, b, render_tick));
            }
        }
        self.samples.back().copied()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.samples.len()
    }

    fn should_reset_for_discontinuity(&self, sample: NetworkTransformSample) -> bool {
        let Some(last) = self.samples.back() else {
            return false;
        };
        sample.tick > last.tick
            && self.teleport_distance.is_finite()
            && self.teleport_distance > 0.0
            && last.translation.distance(sample.translation) > self.teleport_distance
    }
}

fn interpolate_sample(
    a: NetworkTransformSample,
    b: NetworkTransformSample,
    render_tick: u32,
) -> NetworkTransformSample {
    let span = b.tick.saturating_sub(a.tick).max(1) as f32;
    let t = (render_tick.saturating_sub(a.tick) as f32 / span).clamp(0.0, 1.0);
    NetworkTransformSample {
        tick: render_tick,
        translation: a.translation.lerp(b.translation, t),
        rotation: a.rotation.slerp(b.rotation, t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u32, x: f32) -> NetworkTransformSample {
        NetworkTransformSample::new(tick, Vec3::new(x, 0.0, 0.0), Quat::IDENTITY)
    }

    #[test]
    fn interpolation_returns_between_adjacent_samples() {
        let mut buffer = NetworkTransformInterpolationBuffer::default();
        buffer.push_sample(sample(10, 0.0));
        buffer.push_sample(sample(12, 2.0));

        let rendered = buffer.sample_at(11).unwrap();

        assert_eq!(rendered.tick, 11);
        assert_eq!(rendered.translation, Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn interpolation_handles_missing_samples_deterministically() {
        let mut buffer = NetworkTransformInterpolationBuffer {
            teleport_distance: 10.0,
            ..default()
        };
        buffer.push_sample(sample(10, 0.0));
        buffer.push_sample(sample(14, 4.0));

        assert_eq!(buffer.sample_at(12).unwrap().translation.x, 2.0);
        assert_eq!(buffer.sample_at(20).unwrap().translation.x, 4.0);
    }

    #[test]
    fn duplicate_ticks_replace_existing_samples() {
        let mut buffer = NetworkTransformInterpolationBuffer::default();
        buffer.push_sample(sample(10, 0.0));
        buffer.push_sample(sample(10, 5.0));

        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.sample_at(10).unwrap().translation.x, 5.0);
    }

    #[test]
    fn stale_samples_are_inserted_in_tick_order() {
        let mut buffer = NetworkTransformInterpolationBuffer::default();
        buffer.push_sample(sample(12, 2.0));
        buffer.push_sample(sample(10, 0.0));

        assert_eq!(buffer.sample_at(11).unwrap().translation.x, 1.0);
    }

    #[test]
    fn overflow_drops_oldest_samples() {
        let mut buffer = NetworkTransformInterpolationBuffer {
            max_samples: 2,
            ..default()
        };
        buffer.push_sample(sample(10, 0.0));
        buffer.push_sample(sample(11, 1.0));
        buffer.push_sample(sample(12, 2.0));

        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.sample_at(10).unwrap().translation.x, 1.0);
    }

    #[test]
    fn teleport_distance_clears_old_samples() {
        let mut buffer = NetworkTransformInterpolationBuffer {
            teleport_distance: 1.0,
            ..default()
        };
        buffer.push_sample(sample(10, 0.0));
        buffer.push_sample(sample(11, 4.0));

        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.sample_at(10).unwrap().translation.x, 4.0);
    }
}
