//! Port of `helpers.c` (GIMP-derived color math, spectral WGM mixing) and
//! the two `fastapprox` functions libmypaint uses (`fastpow2`, `fastlog2`).
//! All floating-point operation order matches the C.

/// `WGM_EPSILON` from helpers.h.
pub const WGM_EPSILON: f32 = 0.001;

/// The three spectral conversion matrices (10-bin, `helpers.c`).
#[rustfmt::skip]
static T_MATRIX_SMALL: [[f32; 10]; 3] = [
    [0.026595621243689, 0.049779426257903, 0.022449850859496, -0.218453689278271,
     -0.256894883201278, 0.445881722194840, 0.772365886289756, 0.194498761382537,
     0.014038157587820, 0.007687264480513],
    [-0.032601672674412, -0.061021043498478, -0.052490001018404,
     0.206659098273522, 0.572496335158169, 0.317837248815438, -0.021216624031211,
     -0.019387668756117, -0.001521339050858, -0.000835181622534],
    [0.339475473216284, 0.635401374177222, 0.771520797089589, 0.113222640692379,
     -0.055251113343776, -0.048222578468680, -0.012966666339586,
     -0.001523814504223, -0.000094718948810, -0.000051604594741],
];

#[rustfmt::skip]
static SPECTRAL_R_SMALL: [f32; 10] = [
    0.009281362787953, 0.009732627042016, 0.011254252737167, 0.015105578649573,
    0.024797924177217, 0.083622585502406, 0.977865045723212, 1.000000000000000,
    0.999961046144372, 0.999999992756822,
];

#[rustfmt::skip]
static SPECTRAL_G_SMALL: [f32; 10] = [
    0.002854127435775, 0.003917589679914, 0.012132151699187, 0.748259205918013,
    1.000000000000000, 0.865695937531795, 0.037477469241101, 0.022816789725717,
    0.021747419446456, 0.021384940572308,
];

#[rustfmt::skip]
static SPECTRAL_B_SMALL: [f32; 10] = [
    0.537052150373386, 0.546646402401469, 0.575501819073983, 0.258778829633924,
    0.041709923751716, 0.012662638828324, 0.007485593127390, 0.006766900622462,
    0.006699764779016, 0.006676219883241,
];

use crate::rngdouble::RngDouble;

/// `rand_gauss` — sum of four uniform doubles, scaled.
pub fn rand_gauss(rng: &mut RngDouble) -> f32 {
    let mut sum: f64 = 0.0;
    sum += rng.next();
    sum += rng.next();
    sum += rng.next();
    sum += rng.next();
    (sum * 1.732_050_807_57 - 3.464_101_615_14) as f32
}

/// `mod_arith` — arithmetic modulo (C `fmodf` mishandles negatives).
pub fn mod_arith(a: f32, n: f32) -> f32 {
    // C: float ret = a - N * floor(a/N); — a/N is float, floor and the
    // multiply/subtract run in double, then narrow to float.
    let q = (a / n) as f64;
    ((a as f64) - (n as f64) * q.floor()) as f32
}

/// `smallest_angular_difference`.
pub fn smallest_angular_difference(angle_a: f32, angle_b: f32) -> f32 {
    let mut a = angle_b - angle_a;
    a = mod_arith(a + 180.0, 360.0) - 180.0;
    a += if a > 180.0 {
        -360.0
    } else if a < -180.0 {
        360.0
    } else {
        0.0
    };
    a
}

/// `rgb_to_hsv_float` (GIMP `gimp_rgb_to_hsv`); writes back h, s, v into
/// the same three slots (r → h, g → s, b → v as the C does via pointers).
pub fn rgb_to_hsv_float(rgb: &mut [f32; 3]) {
    let (r, g, b) = (clamp(rgb[0], 0.0, 1.0), clamp(rgb[1], 0.0, 1.0), clamp(rgb[2], 0.0, 1.0));
    let max = max3(r, g, b);
    let min = min3(r, g, b);

    let v = max;
    let delta = max - min;

    let mut h = 0.0f32;
    let s;
    if delta > 0.0001 {
        s = delta / max;
        if r == max {
            h = (g - b) / delta;
            if h < 0.0 {
                h += 6.0;
            }
        } else if g == max {
            h = 2.0 + (b - r) / delta;
        } else if b == max {
            h = 4.0 + (r - g) / delta;
        }
        h /= 6.0;
    } else {
        s = 0.0;
        h = 0.0;
    }

    rgb[0] = h;
    rgb[1] = s;
    rgb[2] = v;
}

/// `hsv_to_rgb_float` (GIMP `gimp_hsv_to_rgb`; note the double-precision
/// internals with float outputs, matching the C exactly).
pub fn hsv_to_rgb_float(hsv: &mut [f32; 3]) {
    let (mut h, mut s, v) = (hsv[0], hsv[1], hsv[2]);
    let mut rgb = [0.0f32; 3];

    h -= h.floor();
    s = clamp(s, 0.0, 1.0);
    let v = clamp(v, 0.0, 1.0);

    if s == 0.0 {
        rgb = [v, v, v];
    } else {
        let mut hue = h as f64;
        if hue == 1.0 {
            hue = 0.0;
        }
        hue *= 6.0;
        let i = hue as i32; // C (int) truncation
        let f = hue - i as f64;
        let w = (v as f64) * (1.0 - s as f64);
        let q = (v as f64) * (1.0 - (s as f64 * f));
        let t = (v as f64) * (1.0 - (s as f64 * (1.0 - f)));
        let (r, g, b) = match i {
            0 => (v, t as f32, w as f32),
            1 => (q as f32, v, w as f32),
            2 => (w as f32, v, t as f32),
            3 => (w as f32, q as f32, v),
            4 => (t as f32, w as f32, v),
            5 => (v, w as f32, q as f32),
            _ => (0.0, 0.0, 0.0),
        };
        rgb = [r, g, b];
    }
    hsv[0] = rgb[0];
    hsv[1] = rgb[1];
    hsv[2] = rgb[2];
}

/// `rgb_to_hsl_float` (GIMP `gimp_rgb_to_hsl`).
pub fn rgb_to_hsl_float(rgb: &mut [f32; 3]) {
    let (r, g, b) = (clamp(rgb[0], 0.0, 1.0), clamp(rgb[1], 0.0, 1.0), clamp(rgb[2], 0.0, 1.0));

    // C uses doubles for max/min/delta here.
    let max: f64 = max3(r, g, b) as f64;
    let min: f64 = min3(r, g, b) as f64;

    let l = ((max + min) / 2.0) as f32;

    let (mut h, mut s) = (0.0f32, 0.0f32);
    if max == min {
        s = 0.0;
        h = 0.0;
    } else {
        if l as f64 <= 0.5 {
            s = (((max - min) / (max + min)) as f32);
        } else {
            s = (((max - min) / (2.0 - max - min)) as f32);
        }

        let mut delta = max - min;
        if delta == 0.0 {
            delta = 1.0;
        }

        let mut h_local: f32;
        if r as f64 == max {
            h_local = (((g as f64) - (b as f64)) / delta) as f32;
        } else if g as f64 == max {
            h_local = (2.0 + ((b as f64) - (r as f64)) / delta) as f32;
        } else {
            h_local = (4.0 + ((r as f64) - (g as f64)) / delta) as f32;
        }
        h_local /= 6.0;
        if h_local < 0.0 {
            h_local += 1.0;
        }
        h = h_local;
    }
    rgb[0] = h;
    rgb[1] = s;
    rgb[2] = l;
}

/// `hsl_value` (double math).
fn hsl_value(n1: f64, n2: f64, mut hue: f64) -> f64 {
    let val;
    if hue > 6.0 {
        hue -= 6.0;
    } else if hue < 0.0 {
        hue += 6.0;
    }
    if hue < 1.0 {
        val = n1 + (n2 - n1) * hue;
    } else if hue < 3.0 {
        val = n2;
    } else if hue < 4.0 {
        val = n1 + (n2 - n1) * (4.0 - hue);
    } else {
        val = n1;
    }
    val
}

/// `hsl_to_rgb_float`.
pub fn hsl_to_rgb_float(hsl: &mut [f32; 3]) {
    let (h, s, l) = (hsl[0], hsl[1], hsl[2]);

    let h = h - h.floor();
    let s = clamp(s, 0.0, 1.0);
    let l = clamp(l, 0.0, 1.0);

    let rgb = if s == 0.0 {
        [l, l, l]
    } else {
        let (m1, m2): (f64, f64);
        if l as f64 <= 0.5 {
            m2 = l as f64 * (1.0 + s as f64);
        } else {
            m2 = l as f64 + s as f64 - (l as f64) * (s as f64);
        }
        let m1 = 2.0 * (l as f64) - m2;
        let m2 = m2;
        let m1 = m1; // shadow to double context below

        let m1 = 2.0 * (l as f64) - m2;
        [
            hsl_value(m1, m2, (h as f64) * 6.0 + 2.0) as f32,
            hsl_value(m1, m2, (h as f64) * 6.0) as f32,
            hsl_value(m1, m2, (h as f64) * 6.0 - 2.0) as f32,
        ]
    };
    hsl[0] = rgb[0];
    hsl[1] = rgb[1];
    hsl[2] = rgb[2];
}

/// `rgb_to_spectral` — upsample linear sRGB to a 10-bin reflectance.
pub fn rgb_to_spectral(r: f32, g: f32, b: f32, spectral: &mut [f32; 10]) {
    let offset = 1.0 - WGM_EPSILON;
    let r = r * offset + WGM_EPSILON;
    let g = g * offset + WGM_EPSILON;
    let b = b * offset + WGM_EPSILON;
    for i in 0..10 {
        spectral[i] += SPECTRAL_R_SMALL[i] * r + SPECTRAL_G_SMALL[i] * g + SPECTRAL_B_SMALL[i] * b;
    }
}

/// `spectral_to_rgb`.
pub fn spectral_to_rgb(spectral: &[f32; 10], rgb: &mut [f32; 3]) {
    let offset = 1.0 - WGM_EPSILON;
    let mut tmp = [0.0f32; 3];
    for i in 0..10 {
        tmp[0] += T_MATRIX_SMALL[0][i] * spectral[i];
        tmp[1] += T_MATRIX_SMALL[1][i] * spectral[i];
        tmp[2] += T_MATRIX_SMALL[2][i] * spectral[i];
    }
    for i in 0..3 {
        rgb[i] = clamp((tmp[i] - WGM_EPSILON) / offset, 0.0, 1.0);
    }
}

/// `mix_colors` — WGM blend of two RGBA colors; `paint_mode` selects
/// spectral vs. additive weighting. Returns the mixed [r, g, b, a]
/// (the C returns a static buffer; here a fresh value).
pub fn mix_colors(a: &[f32; 4], b: &[f32; 4], fac: f32, paint_mode: f32) -> [f32; 4] {
    let mut result = [0.0f32; 4];
    let opa_a = fac;
    let opa_b = 1.0 - opa_a;
    result[3] = clamp(opa_a * a[3] + opa_b * b[3], 0.0, 1.0);
    // Guard against NaN from division by zero
    let sfac_a = if a[3] == 0.0 { 0.0 } else { opa_a * a[3] / (a[3] + b[3] * opa_b) };
    let sfac_b = 1.0 - sfac_a;

    if paint_mode > 0.0 {
        let mut spec_a = [0.0f32; 10];
        let mut spec_b = [0.0f32; 10];
        rgb_to_spectral(a[0], a[1], a[2], &mut spec_a);
        rgb_to_spectral(b[0], b[1], b[2], &mut spec_b);

        let mut spectralmix = [0.0f32; 10];
        for i in 0..10 {
            // The C uses real powf here (not the fast approximation).
            spectralmix[i] = powf(spec_a[i], sfac_a) * powf(spec_b[i], sfac_b);
        }

        let mut rgb_result = [0.0f32; 3];
        spectral_to_rgb(&spectralmix, &mut rgb_result);
        for i in 0..3 {
            result[i] = rgb_result[i];
        }
    }

    if paint_mode < 1.0 {
        for i in 0..3 {
            result[i] = result[i] * paint_mode + (1.0 - paint_mode) * (a[i] * opa_a + b[i] * opa_b);
        }
    }
    result
}

/// `fastlog2` (fastapprox).
#[inline]
pub fn fastlog2(x: f32) -> f32 {
    let vx: u32 = x.to_bits();
    let mx: f32 = f32::from_bits((vx & 0x007F_FFFF) | 0x3f00_0000);
    // C: `float y = vx.i;` — the BIT PATTERN reinterpreted as an integer
    // value, not the float itself.
    let mut y = vx as f32;
    y *= 1.192_092_895_507_812_5e-7;

    y - 124.225_51_5f32 - 1.498_030_302f32 * mx - 1.725_879_99f32 / (0.352_088_706_8f32 + mx)
}

/// `fastpow2` (fastapprox fastexp.h) — exact bit-level port.
#[inline]
pub fn fastpow2(p: f32) -> f32 {
    let offset: f32 = if p < 0.0 { 1.0 } else { 0.0 };
    let clipp: f32 = if p < -126.0 { -126.0 } else { p };
    let w = clipp as i32;
    let z = clipp - w as f32 + offset;
    let v: u32 = ((1u32 << 23) as f32
        * (clipp + 121.274_057_5f32 + 27.728_023_3f32 / (4.842_525_68f32 - z) - 1.490_129_07f32 * z))
        as u32;
    f32::from_bits(v)
}

/// `fastpow` (fastapprox fastpow.h).
#[inline]
pub fn fastpow(x: f32, p: f32) -> f32 {
    fastpow2(p * fastlog2(x))
}

/// `SQR` macro from helpers.h.
#[inline]
pub fn sqr(x: f32) -> f32 {
    x * x
}

#[inline]
pub fn clamp(x: f32, low: f32, high: f32) -> f32 {
    if x > high {
        high
    } else if x < low {
        low
    } else {
        x
    }
}

#[inline]
fn max3(a: f32, b: f32, c: f32) -> f32 {
    if a > b { a.max(c) } else { b.max(c) }
}

#[inline]
fn min3(a: f32, b: f32, c: f32) -> f32 {
    if a < b { a.min(c) } else { b.min(c) }
}

// f32::powf exists; wrap for clarity where the C says powf.
#[inline]
fn powf(x: f32, p: f32) -> f32 {
    f32::powf(x, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fastpow_close_to_powf() {
        // Sanity only — the parity oracle pins exact bits.
        for &(x, p) in &[(0.5f32, 0.5f32), (0.1, 0.3), (0.99, 0.7), (0.3, 0.7)] {
            let a = fastpow(x, p);
            let b = f32::powf(x, p);
            assert!((a - b).abs() < 0.01, "{x}^{p}: {a} vs {b}");
        }
    }
}
