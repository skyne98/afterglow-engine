//! Trace clocks and explicit collector-side clock mappings.

use std::sync::OnceLock;
use std::time::Instant;

pub trait Clock {
    /// Monotonic ticks in this producer's declared clock domain.
    fn now(&self) -> u64;
}

/// Process-wide monotonic nanosecond clock shared by every native producer.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    #[inline]
    fn now(&self) -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN
            .get_or_init(Instant::now)
            .elapsed()
            .as_nanos()
            .min(u64::MAX as u128) as u64
    }
}

/// Maps producer ticks into one collector reference timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockMapping {
    pub clock_domain: u32,
    pub origin_tick: u64,
    pub origin_reference_ns: u64,
    /// Reference nanoseconds added for each `rate_numerator/rate_denominator`
    /// producer ticks. A native nanosecond clock uses 1/1.
    pub rate_numerator: u64,
    pub rate_denominator: u64,
    pub uncertainty_ns: u64,
}

impl ClockMapping {
    pub const fn native(clock_domain: u32) -> Self {
        Self {
            clock_domain,
            origin_tick: 0,
            origin_reference_ns: 0,
            rate_numerator: 1,
            rate_denominator: 1,
            uncertainty_ns: 0,
        }
    }

    pub fn map_to_reference_ns(self, tick: u64) -> Option<u64> {
        if self.rate_denominator == 0 {
            return None;
        }
        let delta = tick.checked_sub(self.origin_tick)? as u128;
        let scaled =
            delta.checked_mul(self.rate_numerator as u128)? / self.rate_denominator as u128;
        let mapped = (self.origin_reference_ns as u128).checked_add(scaled)?;
        u64::try_from(mapped).ok()
    }
}
