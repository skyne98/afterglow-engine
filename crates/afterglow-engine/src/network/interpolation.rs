use crate::core::identity::StableEntityId;
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Resource, Clone, Debug, PartialEq, Reflect)]
pub struct RemoteInterpolationBuffer {
    delay_ticks: u32,
    max_extrapolation_ticks: u32,
    samples: BTreeMap<StableEntityId, BTreeMap<u32, RemoteEntitySample>>,
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct RemoteEntitySample {
    pub fields: BTreeMap<String, f32>,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct SmoothedEntitySample {
    pub entity: StableEntityId,
    pub tick: f32,
    pub mode: SmoothingMode,
    pub fields: BTreeMap<String, f32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum SmoothingMode {
    Exact,
    Interpolated,
    Extrapolated,
}

impl Default for RemoteInterpolationBuffer {
    fn default() -> Self {
        Self {
            delay_ticks: 2,
            max_extrapolation_ticks: 2,
            samples: BTreeMap::new(),
        }
    }
}

impl RemoteInterpolationBuffer {
    pub fn with_timing(mut self, delay_ticks: u32, max_extrapolation_ticks: u32) -> Self {
        self.delay_ticks = delay_ticks;
        self.max_extrapolation_ticks = max_extrapolation_ticks;
        self
    }

    pub fn delay_ticks(&self) -> u32 {
        self.delay_ticks
    }

    pub fn max_extrapolation_ticks(&self) -> u32 {
        self.max_extrapolation_ticks
    }

    pub fn record(
        &mut self,
        entity: StableEntityId,
        tick: u32,
        sample: RemoteEntitySample,
    ) -> Option<RemoteEntitySample> {
        self.samples.entry(entity).or_default().insert(tick, sample)
    }

    pub fn sample_at(
        &self,
        entity: StableEntityId,
        render_tick: f32,
    ) -> Option<SmoothedEntitySample> {
        let samples = self.samples.get(&entity)?;
        let before = samples
            .range(..=render_tick.floor() as u32)
            .next_back()
            .map(|(tick, sample)| (*tick, sample));
        let after = samples
            .range(render_tick.ceil() as u32..)
            .next()
            .map(|(tick, sample)| (*tick, sample));

        match (before, after) {
            (Some((tick, sample)), _) if tick as f32 == render_tick => Some(smoothed(
                entity,
                render_tick,
                SmoothingMode::Exact,
                sample.fields.clone(),
            )),
            (Some((a_tick, a)), Some((b_tick, b))) if a_tick != b_tick => {
                let alpha = (render_tick - a_tick as f32) / (b_tick - a_tick) as f32;
                Some(smoothed(
                    entity,
                    render_tick,
                    SmoothingMode::Interpolated,
                    blend_fields(&a.fields, &b.fields, alpha),
                ))
            }
            _ => self.extrapolate(entity, render_tick),
        }
    }

    pub fn sample_for_server_tick(
        &self,
        entity: StableEntityId,
        latest_server_tick: u32,
    ) -> Option<SmoothedEntitySample> {
        let render_tick = latest_server_tick.saturating_sub(self.delay_ticks) as f32;
        self.sample_at(entity, render_tick)
    }

    pub fn prune_before(&mut self, tick: u32) {
        for samples in self.samples.values_mut() {
            samples.retain(|sample_tick, _| *sample_tick >= tick);
        }
        self.samples.retain(|_, samples| !samples.is_empty());
    }

    pub fn clear_entity(&mut self, entity: StableEntityId) {
        self.samples.remove(&entity);
    }

    pub fn sample_count(&self, entity: StableEntityId) -> usize {
        self.samples.get(&entity).map_or(0, BTreeMap::len)
    }

    fn extrapolate(
        &self,
        entity: StableEntityId,
        render_tick: f32,
    ) -> Option<SmoothedEntitySample> {
        let samples = self.samples.get(&entity)?;
        let mut tail = samples.iter().rev();
        let (b_tick, b) = tail.next()?;
        let (a_tick, a) = tail.next()?;
        if render_tick <= *b_tick as f32 {
            return None;
        }
        let delta = render_tick - *b_tick as f32;
        if delta > self.max_extrapolation_ticks as f32 {
            return None;
        }
        let tick_delta = (*b_tick - *a_tick).max(1) as f32;
        Some(smoothed(
            entity,
            render_tick,
            SmoothingMode::Extrapolated,
            extrapolate_fields(&a.fields, &b.fields, delta / tick_delta),
        ))
    }
}

impl RemoteEntitySample {
    pub fn with_field(mut self, name: impl Into<String>, value: f32) -> Self {
        self.fields.insert(name.into(), value);
        self
    }
}

fn smoothed(
    entity: StableEntityId,
    tick: f32,
    mode: SmoothingMode,
    fields: BTreeMap<String, f32>,
) -> SmoothedEntitySample {
    SmoothedEntitySample {
        entity,
        tick,
        mode,
        fields,
    }
}

fn blend_fields(
    a: &BTreeMap<String, f32>,
    b: &BTreeMap<String, f32>,
    alpha: f32,
) -> BTreeMap<String, f32> {
    a.iter()
        .filter_map(|(name, a_value)| {
            let b_value = b.get(name)?;
            Some((name.clone(), a_value + (b_value - a_value) * alpha))
        })
        .collect()
}

fn extrapolate_fields(
    a: &BTreeMap<String, f32>,
    b: &BTreeMap<String, f32>,
    alpha: f32,
) -> BTreeMap<String, f32> {
    a.iter()
        .filter_map(|(name, a_value)| {
            let b_value = b.get(name)?;
            Some((name.clone(), b_value + (b_value - a_value) * alpha))
        })
        .collect()
}

#[cfg(test)]
mod tests;
