//! Fixed-capacity voice ownership and sample-clock automation.
//!
//! This module contains no backend policy: every producer ultimately occupies
//! one slot, and placement determines whether that slot requires the complete
//! world-acoustic chain. It allocates nothing after construction.

use crate::{PHYSICAL_VOICE_CAPACITY, SAMPLE_RATE, VOICE_CAPACITY, sound::SoundBank};

pub(crate) const INVALID_VOICE_HANDLE: u32 = 0;
const INDEX_BITS: u32 = 8;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
const MAX_GENERATION: u32 = (1 << (32 - INDEX_BITS)) - 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum VoicePlacement {
    World([f32; 3]),
    Attached(u32),
    TwoD,
    SpatialOnly([f32; 3]),
    ListenerRelative([f32; 3]),
}

impl VoicePlacement {
    fn is_world_physical(self) -> bool {
        matches!(self, Self::World(_) | Self::Attached(_))
    }

    fn valid(self) -> bool {
        match self {
            Self::World(position)
            | Self::SpatialOnly(position)
            | Self::ListenerRelative(position) => position.into_iter().all(f32::is_finite),
            Self::Attached(entity) => entity != 0,
            Self::TwoD => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RampCurve {
    Smooth,
    EqualPowerIn,
    EqualPowerOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Completion {
    None,
    Pause,
    Stop,
}

#[derive(Clone, Copy, Debug)]
struct GainRamp {
    start: f32,
    target: f32,
    elapsed: u32,
    duration: u32,
    curve: RampCurve,
    completion: Completion,
}

impl GainRamp {
    const NONE: Self = Self {
        start: 1.0,
        target: 1.0,
        elapsed: 0,
        duration: 0,
        curve: RampCurve::Smooth,
        completion: Completion::None,
    };

    fn value(self) -> f32 {
        if self.duration == 0 {
            return self.target;
        }
        let t = (self.elapsed as f32 / self.duration as f32).clamp(0.0, 1.0);
        let shaped = match self.curve {
            RampCurve::Smooth => t * t * (3.0 - 2.0 * t),
            RampCurve::EqualPowerIn => (t * core::f32::consts::FRAC_PI_2).sin(),
            RampCurve::EqualPowerOut => 1.0 - (t * core::f32::consts::FRAC_PI_2).cos(),
        };
        self.start + (self.target - self.start) * shaped
    }
}

#[derive(Clone, Copy, Debug)]
struct VoiceSlot {
    generation: u32,
    occupied: bool,
    paused: bool,
    sound: u32,
    placement: VoicePlacement,
    priority: u32,
    nominal_volume: f32,
    gain: f32,
    cursor: u64,
    ramp: GainRamp,
}

impl VoiceSlot {
    const EMPTY: Self = Self {
        generation: 1,
        occupied: false,
        paused: false,
        sound: 0,
        placement: VoicePlacement::TwoD,
        priority: 0,
        nominal_volume: 1.0,
        gain: 0.0,
        cursor: 0,
        ramp: GainRamp::NONE,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct VoiceSchedulerStats {
    pub active: u32,
    pub active_world_physical: u32,
    pub rejected_capacity: u64,
    pub rejected_physical_capacity: u64,
    pub stale_handles: u64,
    pub completed_fades: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VoiceRenderState {
    pub active: bool,
    pub sound: u32,
    pub placement: VoicePlacement,
    pub gain: f32,
    pub cursor: u64,
}

pub(crate) struct VoiceScheduler {
    slots: [VoiceSlot; VOICE_CAPACITY as usize],
    stats: VoiceSchedulerStats,
    controlled: bool,
}

impl VoiceScheduler {
    pub(crate) fn new() -> Self {
        Self {
            slots: [VoiceSlot::EMPTY; VOICE_CAPACITY as usize],
            stats: VoiceSchedulerStats::default(),
            controlled: false,
        }
    }

    pub(crate) fn spawn(
        &mut self,
        sound: u32,
        placement: VoicePlacement,
        volume: f32,
        priority: u32,
    ) -> u32 {
        self.spawn_with_gain(sound, placement, volume, volume, priority)
    }

    fn spawn_with_gain(
        &mut self,
        sound: u32,
        placement: VoicePlacement,
        initial_gain: f32,
        nominal_volume: f32,
        priority: u32,
    ) -> u32 {
        if sound == 0
            || !placement.valid()
            || !valid_volume(initial_gain)
            || !valid_volume(nominal_volume)
        {
            return INVALID_VOICE_HANDLE;
        }
        if placement.is_world_physical()
            && self.stats.active_world_physical >= PHYSICAL_VOICE_CAPACITY
        {
            self.stats.rejected_physical_capacity += 1;
            return INVALID_VOICE_HANDLE;
        }
        // Reserve the leading slots for complete world-physical voices so a
        // backend slot always has matching HRTF+reflection state. Explicitly
        // nonphysical voices use the remaining slots and cannot crowd them out.
        let range = if placement.is_world_physical() {
            0..PHYSICAL_VOICE_CAPACITY as usize
        } else {
            PHYSICAL_VOICE_CAPACITY as usize..self.slots.len()
        };
        let Some(index) = range.into_iter().find(|&index| !self.slots[index].occupied) else {
            self.stats.rejected_capacity += 1;
            return INVALID_VOICE_HANDLE;
        };
        let slot = &mut self.slots[index];
        slot.occupied = true;
        slot.paused = false;
        slot.sound = sound;
        slot.placement = placement;
        slot.priority = priority;
        slot.nominal_volume = nominal_volume;
        slot.gain = initial_gain;
        slot.cursor = 0;
        slot.ramp = GainRamp::NONE;
        self.controlled = true;
        self.stats.active += 1;
        if placement.is_world_physical() {
            self.stats.active_world_physical += 1;
        }
        encode_handle(index, slot.generation)
    }

    pub(crate) fn set_volume(&mut self, handle: u32, volume: f32, seconds: f32) -> bool {
        if !valid_volume(volume) {
            return false;
        }
        let duration = match duration_samples(seconds) {
            Some(duration) => duration,
            None => return false,
        };
        let Some(index) = self.resolve(handle) else {
            return false;
        };
        let slot = &mut self.slots[index];
        slot.nominal_volume = volume;
        slot.paused = false;
        start_ramp(slot, volume, duration, RampCurve::Smooth, Completion::None);
        true
    }

    pub(crate) fn pause(&mut self, handle: u32, seconds: f32) -> bool {
        let duration = match duration_samples(seconds) {
            Some(duration) => duration,
            None => return false,
        };
        let Some(index) = self.resolve(handle) else {
            return false;
        };
        let slot = &mut self.slots[index];
        if slot.paused {
            return true;
        }
        start_ramp(slot, 0.0, duration, RampCurve::Smooth, Completion::Pause);
        true
    }

    pub(crate) fn resume(&mut self, handle: u32, seconds: f32) -> bool {
        let duration = match duration_samples(seconds) {
            Some(duration) => duration,
            None => return false,
        };
        let Some(index) = self.resolve(handle) else {
            return false;
        };
        let slot = &mut self.slots[index];
        slot.paused = false;
        start_ramp(
            slot,
            slot.nominal_volume,
            duration,
            RampCurve::Smooth,
            Completion::None,
        );
        true
    }

    pub(crate) fn stop(&mut self, handle: u32, seconds: f32) -> bool {
        let duration = match duration_samples(seconds) {
            Some(duration) => duration,
            None => return false,
        };
        let Some(index) = self.resolve(handle) else {
            return false;
        };
        if duration == 0 {
            self.release(index);
        } else {
            start_ramp(
                &mut self.slots[index],
                0.0,
                duration,
                RampCurve::Smooth,
                Completion::Stop,
            );
        }
        true
    }

    pub(crate) fn crossfade(&mut self, from: u32, to: u32, seconds: f32) -> bool {
        let duration = match duration_samples(seconds) {
            Some(duration) => duration,
            None => return false,
        };
        let Some(from_index) = self.resolve(from) else {
            return false;
        };
        let Some(to_index) = self.resolve(to) else {
            return false;
        };
        if from_index == to_index {
            return false;
        }
        if duration == 0 {
            self.release(from_index);
            let target = self.slots[to_index].nominal_volume;
            self.slots[to_index].gain = target;
            self.slots[to_index].ramp = GainRamp::NONE;
            return true;
        }
        start_ramp(
            &mut self.slots[from_index],
            0.0,
            duration,
            RampCurve::EqualPowerOut,
            Completion::Stop,
        );
        let target = self.slots[to_index].nominal_volume;
        start_ramp(
            &mut self.slots[to_index],
            target,
            duration,
            RampCurve::EqualPowerIn,
            Completion::None,
        );
        true
    }

    pub(crate) fn crossfade_to(&mut self, from: u32, sound: u32, seconds: f32) -> u32 {
        let Some(from_index) = self.resolve(from) else {
            return INVALID_VOICE_HANDLE;
        };
        let source = self.slots[from_index];
        let to = self.spawn_with_gain(
            sound,
            source.placement,
            0.0,
            source.nominal_volume,
            source.priority,
        );
        if to == INVALID_VOICE_HANDLE {
            return INVALID_VOICE_HANDLE;
        }
        if !self.crossfade(from, to, seconds) {
            if let Some(index) = self.resolve(to) {
                self.release(index);
            }
            return INVALID_VOICE_HANDLE;
        }
        to
    }

    /// Advance all gain automation by a fixed number of device samples. This is
    /// bounded by the compile-time target voice capacity and allocates nothing.
    pub(crate) fn advance(&mut self, frames: u32, sounds: &SoundBank) {
        for index in 0..self.slots.len() {
            let slot = &mut self.slots[index];
            if !slot.occupied {
                continue;
            }
            if !slot.paused {
                slot.cursor = slot.cursor.saturating_add(u64::from(frames));
            }
            if slot.ramp.duration != 0 {
                slot.ramp.elapsed = slot.ramp.elapsed.saturating_add(frames);
                slot.gain = slot.ramp.value();
                if slot.ramp.elapsed >= slot.ramp.duration {
                    slot.gain = slot.ramp.target;
                    let completion = slot.ramp.completion;
                    slot.ramp = GainRamp::NONE;
                    match completion {
                        Completion::None => {}
                        Completion::Pause => slot.paused = true,
                        Completion::Stop => {
                            self.release(index);
                            self.stats.completed_fades += 1;
                            continue;
                        }
                    }
                }
            }
            if !slot.paused
                && sounds
                    .playback_length(slot.sound)
                    .is_some_and(|(length, looped)| !looped && slot.cursor >= u64::from(length))
            {
                self.release(index);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn gain(&mut self, handle: u32) -> Option<f32> {
        self.resolve(handle).map(|index| self.slots[index].gain)
    }

    #[cfg(test)]
    pub(crate) fn placement(&mut self, handle: u32) -> Option<VoicePlacement> {
        self.resolve(handle)
            .map(|index| self.slots[index].placement)
    }

    #[cfg(test)]
    pub(crate) fn is_paused(&mut self, handle: u32) -> Option<bool> {
        self.resolve(handle).map(|index| self.slots[index].paused)
    }

    pub(crate) fn stats(&self) -> VoiceSchedulerStats {
        self.stats
    }

    pub(crate) fn is_controlled(&self) -> bool {
        self.controlled
    }

    pub(crate) fn uses_sound(&self, sound: u32) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.occupied && slot.sound == sound)
    }

    pub(crate) fn render_state(&self, index: usize) -> VoiceRenderState {
        let slot = self.slots[index];
        VoiceRenderState {
            active: slot.occupied && !slot.paused,
            sound: slot.sound,
            placement: slot.placement,
            gain: slot.gain,
            cursor: slot.cursor,
        }
    }

    fn resolve(&mut self, handle: u32) -> Option<usize> {
        let Some((index, generation)) = decode_handle(handle) else {
            self.stats.stale_handles += 1;
            return None;
        };
        let Some(slot) = self.slots.get(index) else {
            self.stats.stale_handles += 1;
            return None;
        };
        if !slot.occupied || slot.generation != generation {
            self.stats.stale_handles += 1;
            return None;
        }
        Some(index)
    }

    fn release(&mut self, index: usize) {
        let slot = &mut self.slots[index];
        if !slot.occupied {
            return;
        }
        if slot.placement.is_world_physical() {
            self.stats.active_world_physical -= 1;
        }
        self.stats.active -= 1;
        let generation = next_generation(slot.generation);
        *slot = VoiceSlot::EMPTY;
        slot.generation = generation;
    }
}

fn valid_volume(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn duration_samples(seconds: f32) -> Option<u32> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    if seconds == 0.0 {
        return Some(0);
    }
    let samples = (seconds as f64 * SAMPLE_RATE as f64).round();
    Some(samples.clamp(1.0, u32::MAX as f64) as u32)
}

fn start_ramp(
    slot: &mut VoiceSlot,
    target: f32,
    duration: u32,
    curve: RampCurve,
    completion: Completion,
) {
    if duration == 0 {
        slot.gain = target;
        slot.ramp = GainRamp::NONE;
        if completion == Completion::Pause {
            slot.paused = true;
        }
        return;
    }
    slot.ramp = GainRamp {
        start: slot.gain,
        target,
        elapsed: 0,
        duration,
        curve,
        completion,
    };
}

fn encode_handle(index: usize, generation: u32) -> u32 {
    debug_assert!(index < INDEX_MASK as usize);
    (generation << INDEX_BITS) | (index as u32 + 1)
}

fn decode_handle(handle: u32) -> Option<(usize, u32)> {
    let encoded_index = handle & INDEX_MASK;
    let generation = handle >> INDEX_BITS;
    if encoded_index == 0 || generation == 0 {
        return None;
    }
    Some(((encoded_index - 1) as usize, generation))
}

fn next_generation(generation: u32) -> u32 {
    let next = (generation + 1) & MAX_GENERATION;
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;
    use afterglow_rpc::allocation::assert_no_alloc;

    fn scheduler() -> VoiceScheduler {
        VoiceScheduler::new()
    }

    #[test]
    fn stale_handles_never_control_reused_slots() {
        let mut voices = scheduler();
        let first = voices.spawn(1, VoicePlacement::TwoD, 1.0, 0);
        assert_ne!(first, INVALID_VOICE_HANDLE);
        assert!(voices.stop(first, 0.0));
        let second = voices.spawn(2, VoicePlacement::TwoD, 1.0, 0);
        assert_ne!(first, second);
        assert!(!voices.set_volume(first, 0.5, 0.0));
        assert_eq!(voices.gain(second), Some(1.0));
    }

    #[test]
    fn physical_capacity_is_all_or_nothing() {
        let mut voices = scheduler();
        for index in 0..PHYSICAL_VOICE_CAPACITY {
            assert_ne!(
                voices.spawn(
                    index + 1,
                    VoicePlacement::World([index as f32, 0.0, 0.0]),
                    1.0,
                    0
                ),
                INVALID_VOICE_HANDLE
            );
        }
        assert_eq!(
            voices.spawn(999, VoicePlacement::Attached(1), 1.0, 0),
            INVALID_VOICE_HANDLE
        );
        assert_eq!(
            voices.stats().active_world_physical,
            PHYSICAL_VOICE_CAPACITY
        );
        assert_eq!(voices.stats().rejected_physical_capacity, 1);
    }

    #[test]
    fn crossfade_to_inherits_placement_and_invalidates_outgoing_on_completion() {
        let mut voices = scheduler();
        let from = voices.spawn(1, VoicePlacement::ListenerRelative([1.0, 0.0, 0.0]), 0.8, 7);
        let to = voices.crossfade_to(from, 2, 1.0);
        assert_ne!(to, INVALID_VOICE_HANDLE);
        assert_eq!(
            voices.placement(to),
            Some(VoicePlacement::ListenerRelative([1.0, 0.0, 0.0]))
        );
        let sounds = SoundBank::new();
        voices.advance(SAMPLE_RATE / 2, &sounds);
        let from_mid = voices.gain(from).unwrap();
        let to_mid = voices.gain(to).unwrap();
        assert!(from_mid > 0.5 && from_mid < 0.6);
        assert!(to_mid > 0.5 && to_mid < 0.6);
        voices.advance(SAMPLE_RATE / 2, &sounds);
        assert_eq!(voices.gain(from), None);
        assert_eq!(voices.gain(to), Some(0.8));
    }

    #[test]
    fn volume_pause_resume_and_stop_are_sample_clock_driven() {
        let mut voices = scheduler();
        let voice = voices.spawn(1, VoicePlacement::TwoD, 1.0, 0);
        assert!(voices.set_volume(voice, 0.25, 1.0));
        let sounds = SoundBank::new();
        voices.advance(SAMPLE_RATE / 2, &sounds);
        assert!((voices.gain(voice).unwrap() - 0.625).abs() < 0.001);
        assert!(voices.pause(voice, 0.5));
        voices.advance(SAMPLE_RATE / 2, &sounds);
        assert_eq!(voices.gain(voice), Some(0.0));
        assert_eq!(voices.is_paused(voice), Some(true));
        assert!(voices.resume(voice, 0.5));
        voices.advance(SAMPLE_RATE / 2, &sounds);
        assert_eq!(voices.gain(voice), Some(0.25));
        assert!(voices.stop(voice, 0.25));
        voices.advance(SAMPLE_RATE / 4, &sounds);
        assert_eq!(voices.gain(voice), None);
    }

    #[test]
    fn hot_automation_and_advance_allocate_nothing() {
        let mut voices = scheduler();
        let from = voices.spawn(1, VoicePlacement::TwoD, 1.0, 0);
        let to = voices.spawn(2, VoicePlacement::TwoD, 0.0, 0);
        assert!(assert_no_alloc(|| voices.crossfade(from, to, 0.5)));
        let sounds = SoundBank::new();
        assert_no_alloc(|| {
            voices.advance(128, &sounds);
        });
        assert!(assert_no_alloc(|| voices.set_volume(to, 0.5, 0.25)));
    }
}
