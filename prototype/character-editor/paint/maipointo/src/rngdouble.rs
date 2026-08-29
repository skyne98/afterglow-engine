//! Double-precision lagged-Fibonacci RNG — port of `rng-double.c`
//! (Knuth's TAoCP 3.6–15, MyPaint's low-quality constants).
//!
//! Every operation is `f64` and must stay `f64`: the brush engine's jitter
//! streams depend on the exact bit patterns of this generator.

/// Low-quality settings MyPaint uses (not the original 1009/70/100/37).
const QUALITY: usize = 19;
const TT: i32 = 7;
const KK: usize = 10;
const LL: usize = 7;

/// `(x+y) mod 1.0` exactly as the C macro: `(x+y)-(int)(x+y)`.
/// Args are in `[0,1)`, so the sum is in `[0,2)` and `(int)` truncation is
/// toward zero (values are non-negative).
#[inline]
fn mod_sum(x: f64, y: f64) -> f64 {
    let s = x + y;
    s - (s as i64) as f64
}

/// `RngDouble` — state plus the rolling output buffer.
///
/// The C uses `ranf_arr_ptr` walking a buffer with a `-1.0` sentinel at
/// `buf[KK]`; here `ptr == KK` plays the sentinel role. The two
/// constructor sentinels (`dummy`/`started`) both mean "cycle on next".
#[derive(Debug)]
pub struct RngDouble {
    ran_u: [f64; KK],
    buf: [f64; QUALITY],
    /// Index of the next random in `buf`; `KK` means "cycle next".
    ptr: usize,
}

impl RngDouble {
    /// `rng_double_new`.
    pub fn new(seed: i64) -> Self {
        let mut rng = Self {
            ran_u: [0.0; KK],
            buf: [0.0; QUALITY],
            ptr: KK, // dummy sentinel: next() cycles
        };
        rng.set_seed(seed);
        rng
    }

    /// `rng_double_get_array` (static so `buf` and `ran_u` can be passed
    /// as disjoint borrows, matching the C using a caller-provided array).
    fn get_array(ran_u: &mut [f64; KK], aa: &mut [f64], n: usize) {
        for (j, slot) in aa.iter_mut().enumerate().take(KK) {
            *slot = ran_u[j];
        }
        for j in KK..n {
            aa[j] = mod_sum(aa[j - KK], aa[j - LL]);
        }
        let mut j = n;
        for i in 0..LL {
            ran_u[i] = mod_sum(aa[j - KK], aa[j - LL]);
            j += 1;
        }
        for i in LL..KK {
            ran_u[i] = mod_sum(aa[j - KK], ran_u[i - LL]);
            j += 1;
        }
    }

    /// `rng_double_set_seed` — verbatim port of the bootstrap/"square"/
    /// "multiply by z" stream setup.
    pub fn set_seed(&mut self, seed: i64) {
        let mut u = [0.0f64; KK + KK - 1];
        let ulp: f64 = (1.0 / (1u32 << 30) as f64) / (1u32 << 22) as f64; // 2^-52
        let mut ss: f64 = 2.0 * ulp * (((seed & 0x3fff_ffff) + 2) as f64);

        for slot in u.iter_mut().take(KK) {
            *slot = ss; // bootstrap the buffer
            ss += ss;
            if ss >= 1.0 {
                ss -= 1.0 - 2.0 * ulp; // cyclic shift of 51 bits
            }
        }
        u[1] += ulp; // make u[1] (and only u[1]) "odd"
        let mut s = seed & 0x3fff_ffff;
        let mut t = TT - 1;
        while t != 0 {
            for j in (1..KK).rev() {
                u[j + j] = u[j];
                u[j + j - 1] = 0.0; // "square"
            }
            for j in (KK..KK + KK - 1).rev() {
                u[j - (KK - LL)] = mod_sum(u[j - (KK - LL)], u[j]);
                u[j - KK] = mod_sum(u[j - KK], u[j]);
            }
            if (s & 1) != 0 {
                // "multiply by z"
                for j in (1..=KK).rev() {
                    u[j] = u[j - 1];
                }
                u[0] = u[KK]; // shift the buffer cyclically
                u[LL] = mod_sum(u[LL], u[KK]);
            }
            if s != 0 {
                s >>= 1;
            } else {
                t -= 1;
            }
        }
        for (j, slot) in u.iter().enumerate().take(LL) {
            self.ran_u[j + KK - LL] = *slot;
        }
        for j in LL..KK {
            self.ran_u[j - LL] = u[j];
        }
        let mut warm = [0.0f64; KK + KK - 1];
        for _ in 0..10 {
            Self::get_array(&mut self.ran_u, &mut warm, KK + KK - 1); // warm things up
        }
        self.ptr = KK; // started sentinel: next() cycles
    }

    /// `rng_double_cycle`.
    fn cycle(&mut self) -> f64 {
        Self::get_array(&mut self.ran_u, &mut self.buf, QUALITY);
        self.buf[KK] = -1.0;
        self.ptr = 1;
        self.buf[0]
    }

    /// `rng_double_next`.
    pub fn next(&mut self) -> f64 {
        if self.ptr < KK {
            let v = self.buf[self.ptr];
            self.ptr += 1;
            v
        } else {
            self.cycle()
        }
    }
}
