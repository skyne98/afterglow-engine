use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub trait RuntimeClock: Send + Sync {
    fn now_micros(&self) -> u64;

    fn now_millis(&self) -> f64 {
        self.now_micros() as f64 / 1_000.0
    }
}

pub struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl RuntimeClock for MonotonicClock {
    fn now_micros(&self) -> u64 {
        self.origin.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
}

#[derive(Default)]
pub struct DeterministicClock {
    micros: AtomicU64,
}

impl DeterministicClock {
    pub fn set_micros(&self, micros: u64) {
        self.micros.store(micros, Ordering::Release);
    }

    pub fn advance_micros(&self, micros: u64) -> u64 {
        self.micros.fetch_add(micros, Ordering::AcqRel) + micros
    }
}

impl RuntimeClock for DeterministicClock {
    fn now_micros(&self) -> u64 {
        self.micros.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_clock_advances_explicitly() {
        let clock = DeterministicClock::default();
        assert_eq!(clock.now_millis(), 0.0);
        clock.advance_micros(16_000);
        assert_eq!(clock.now_millis(), 16.0);
    }
}
