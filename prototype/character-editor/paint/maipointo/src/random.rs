//! The pixel-sampling randomness used by `get_color` (smudge color pickup).
//!
//! libmypaint consumes the raw libc `rand()` stream here — unseeded and
//! platform-defined, which is a fragility we do not copy. Instead this is
//! an explicit abstraction:
//!
//! - [`GlibcRand`] reproduces glibc's TYPE_3 `rand()` stream from seed 1.
//!   Only the parity oracle injects this, to stay byte-compatible with the
//!   vendored native C reference.
//! - [`PortableRand`] is the default for actual use: deterministic,
//!   platform-independent, based on [`crate::rngdouble::RngDouble`].

use crate::rngdouble::RngDouble;

/// Contract: `next()` returns a value in `[0, RAND_MAX]`, mirroring the C's
/// `int rand(void)`; the engine compares it against
/// `(int)(random_sample_rate * rand_max())`.
pub trait RandomSource {
    fn next(&mut self) -> i32;

    /// The inclusive maximum of `next()` (`RAND_MAX` in the C).
    fn rand_max(&self) -> i32 {
        2147483647
    }
}

/// glibc `rand()` — the TYPE_3 additive-feedback generator (DEG 31, SEP 3).
/// Parity-check implementation only.
pub struct GlibcRand {
    state: [i32; 31],
    fptr: usize,
    rptr: usize,
}

impl GlibcRand {
    /// glibc's implicit `srand(1)` startup state.
    pub fn fresh() -> Self {
        let mut state = [0i32; 31];
        state[0] = 1;
        for i in 1..31 {
            let prev = state[i - 1];
            let hi = prev / 127773;
            let lo = prev - hi * 127773;
            let mut word = 16807i32
                .wrapping_mul(lo)
                .wrapping_sub(2836i32.wrapping_mul(hi));
            if word < 0 {
                word += 2147483647;
            }
            state[i] = word;
        }
        // Discard 10 × 31 outputs (glibc's kc = deg × 10 warm-up).
        let (mut fptr, mut rptr) = (3usize, 0usize);
        for _ in 0..310 {
            state[fptr] = state[fptr].wrapping_add(state[rptr]);
            fptr = (fptr + 1) % 31;
            rptr = (rptr + 1) % 31;
        }
        Self { state, fptr, rptr }
    }
}

impl RandomSource for GlibcRand {
    fn next(&mut self) -> i32 {
        self.state[self.fptr] = self.state[self.fptr].wrapping_add(self.state[self.rptr]);
        let v = self.state[self.fptr];
        self.fptr = (self.fptr + 1) % 31;
        self.rptr = (self.rptr + 1) % 31;
        ((v >> 1) & 0x7fff_ffff) as i32
    }
}

/// Actual-use implementation: deterministic and portable. Wraps the brush
/// engine's own lagged-Fibonacci stream shape (`RngDouble`) mapped to the
/// `[0, RAND_MAX]` integer contract.
pub struct PortableRand {
    rng: RngDouble,
}

impl PortableRand {
    pub fn new(seed: i64) -> Self {
        Self {
            rng: RngDouble::new(seed),
        }
    }
}

impl Default for PortableRand {
    fn default() -> Self {
        Self::new(1)
    }
}

impl RandomSource for PortableRand {
    fn next(&mut self) -> i32 {
        (self.rng.next() * 2147483647.0) as i32
    }
}
