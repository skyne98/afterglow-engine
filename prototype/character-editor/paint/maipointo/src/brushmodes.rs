//! Pixel blending — port of libmypaint's `brushmodes.c` (non-spectral set).
//!
//! Tile pixels are RGBA `u16` fix15 (`[0, 2^15]`), **premultiplied** alpha.
//! The `mask` is the LRE-encoded dab shape produced by [`crate::mask`].
//!
//! The spectral `paint` modes (`*_Paint`, `rgb_to_spectral`,
//! `spectral_to_rgb`) are **deferred** — they need the fastapprox `fastpow`
//! port and are tracked as follow-up work. Sampling with `paint > 0`
//! panics until then.
//!
//! We are manipulating pixels with premultiplied alpha directly. This is an
//! "over" operation (opa = topAlpha); topColor is assumed premultiplied:
//!
//! ```text
//! resultAlpha = topAlpha + (1.0 - topAlpha) * bottomAlpha
//! resultColor = topColor + (1.0 - topAlpha) * bottomColor
//! ```

/// One pixel of a tile: RGBA, premultiplied fix15.
pub type Pixel = [u16; 4];

/// LRE walk helper: yields `(opa, pixel_index)` pairs for covered pixels.
/// `pixel_index` indexes 64×64 tile pixels (each pixel is 4 × `u16`).
macro_rules! for_each_masked_pixel {
    ($mask:expr, $mi:ident, $pi:ident, $body:block) => {{
        let mut $mi = 0usize;
        let mut $pi = 0usize;
        loop {
            while $mask[$mi] != 0 {
                { $body }
                $mi += 1;
                $pi += 1;
            }
            if $mask[$mi + 1] == 0 {
                break;
            }
            $pi += $mask[$mi + 1] as usize / 4;
            $mi += 2;
        }
    }};
}

/// `draw_dab_pixels_BlendMode_Normal` — plain "over".
pub fn draw_dab_normal(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    opacity: u16,
) {
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let opa_a = (mask[mi] as u32 * opacity as u32) >> 15; // topAlpha
        let opa_b = (1 << 15) - opa_a; // bottomAlpha
        rgba[p + 3] = (opa_a + ((opa_b * rgba[p + 3] as u32) >> 15)) as u16;
        rgba[p] = ((opa_a * color_r as u32 + opa_b * rgba[p] as u32) >> 15) as u16;
        rgba[p + 1] = ((opa_a * color_g as u32 + opa_b * rgba[p + 1] as u32) >> 15) as u16;
        rgba[p + 2] = ((opa_a * color_b as u32 + opa_b * rgba[p + 2] as u32) >> 15) as u16;
    });
}

/// `draw_dab_pixels_BlendMode_Normal_and_Eraser` — used for smudging and
/// erasing. Smudging "drags" transparency around as if it were a color:
/// smudging over a 60%-opaque region stays 60% opaque (`color_a = 0.6`).
/// For normal erasing `color_a = 0.0` (r/g/b ignored). With `color_a = 1.0`
/// this is exactly [`draw_dab_normal`].
pub fn draw_dab_normal_and_eraser(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    color_a: u16,
    opacity: u16,
) {
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let mut opa_a = (mask[mi] as u32 * opacity as u32) >> 15; // topAlpha
        let opa_b = (1 << 15) - opa_a; // bottomAlpha
        opa_a = (opa_a * color_a as u32) >> 15;
        rgba[p + 3] = (opa_a + ((opa_b * rgba[p + 3] as u32) >> 15)) as u16;
        rgba[p] = ((opa_a * color_r as u32 + opa_b * rgba[p] as u32) >> 15) as u16;
        rgba[p + 1] = ((opa_a * color_g as u32 + opa_b * rgba[p + 1] as u32) >> 15) as u16;
        rgba[p + 2] = ((opa_a * color_b as u32 + opa_b * rgba[p + 2] as u32) >> 15) as u16;
    });
}

/// `draw_dab_pixels_BlendMode_LockAlpha` — normal blending with a locked
/// alpha channel (no new opacity can be added to empty regions).
pub fn draw_dab_lock_alpha(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    opacity: u16,
) {
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let mut opa_a = (mask[mi] as u32 * opacity as u32) >> 15; // topAlpha
        let opa_b = (1 << 15) - opa_a; // bottomAlpha
        opa_a = opa_a * rgba[p + 3] as u32 >> 15;
        rgba[p] = ((opa_a * color_r as u32 + opa_b * rgba[p] as u32) >> 15) as u16;
        rgba[p + 1] = ((opa_a * color_g as u32 + opa_b * rgba[p + 1] as u32) >> 15) as u16;
        rgba[p + 2] = ((opa_a * color_b as u32 + opa_b * rgba[p + 2] as u32) >> 15) as u16;
    });
}

/// `draw_dab_pixels_BlendMode_Posterize` — GIMP-style posterize, blended in
/// by `opacity`; alpha unaffected.
pub fn draw_dab_posterize(mask: &[u16], rgba: &mut [u16], opacity: u16, posterize_num: u16) {
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let r = rgba[p] as f32 / (1 << 15) as f32;
        let g = rgba[p + 1] as f32 / (1 << 15) as f32;
        let b = rgba[p + 2] as f32 / (1 << 15) as f32;

        // C ROUND(x) == (int)(x + 0.5)
        let round = |x: f32| (x + 0.5) as i32;
        let post_r = ((1 << 15) as u32 * round(r * posterize_num as f32) as u32)
            / posterize_num as u32;
        let post_g = ((1 << 15) as u32 * round(g * posterize_num as f32) as u32)
            / posterize_num as u32;
        let post_b = ((1 << 15) as u32 * round(b * posterize_num as f32) as u32)
            / posterize_num as u32;

        let opa_a = (mask[mi] as u32 * opacity as u32) >> 15; // topAlpha
        let opa_b = (1 << 15) - opa_a; // bottomAlpha
        rgba[p] = ((opa_a * post_r + opa_b * rgba[p] as u32) >> 15) as u16;
        rgba[p + 1] = ((opa_a * post_g + opa_b * rgba[p + 1] as u32) >> 15) as u16;
        rgba[p + 2] = ((opa_a * post_b + opa_b * rgba[p + 2] as u32) >> 15) as u16;
    });
}

// ---- Colorize (PDF "Color" non-separable blend mode; Rec. BT.601 luma) ----

const LUMA_RED_COEFF: f32 = 0.2126 * (1 << 15) as f32;
const LUMA_GREEN_COEFF: f32 = 0.7152 * (1 << 15) as f32;
const LUMA_BLUE_COEFF: f32 = 0.0722 * (1 << 15) as f32;

#[inline]
fn luma(r: i32, g: i32, b: i32) -> f32 {
    r as f32 * LUMA_RED_COEFF + g as f32 * LUMA_GREEN_COEFF + b as f32 * LUMA_BLUE_COEFF
}

/// Spec: `SetLum()` + `ClipColor()`. Inputs/outputs are straight (not
/// premultiplied) scaled ints.
fn set_rgb16_lum_from_rgb16(
    topr: u16,
    topg: u16,
    topb: u16,
    botr: &mut u16,
    botg: &mut u16,
    botb: &mut u16,
) {
    // C: uint16_t botlum = LUMA(...)/(1<<15); — float math, then u16 truncation.
    let botlum = (luma(*botr as i32, *botg as i32, *botb as i32) / (1 << 15) as f32) as u16;
    let toplum = (luma(topr as i32, topg as i32, topb as i32) / (1 << 15) as f32) as u16;
    // C: int16_t diff = botlum - toplum; (int promotion, then i16 wrap)
    let diff = ((botlum as i32 - toplum as i32) as i16) as i32;
    let mut r = topr as i32 + diff;
    let mut g = topg as i32 + diff;
    let mut b = topb as i32 + diff;

    // C: int32_t lum = LUMA(r,g,b)/(1<<15); — f32 math, then i32 truncation.
    let lum = (luma(r, g, b) / (1 << 15) as f32) as i32;
    let cmin = r.min(g).min(b);
    let cmax = r.max(g).max(b);
    if cmin < 0 {
        r = lum + ((r - lum) * lum) / (lum - cmin);
        g = lum + ((g - lum) * lum) / (lum - cmin);
        b = lum + ((b - lum) * lum) / (lum - cmin);
    }
    if cmax > (1 << 15) {
        r = lum + ((r - lum) * ((1 << 15) - lum)) / (cmax - lum);
        g = lum + ((g - lum) * ((1 << 15) - lum)) / (cmax - lum);
        b = lum + ((b - lum) * ((1 << 15) - lum)) / (cmax - lum);
    }
    *botr = r as u16;
    *botg = g as u16;
    *botb = b as u16;
}

/// `draw_dab_pixels_BlendMode_Color` — Adobe PDF addendum "Color" mode:
/// apply the source hue/saturation, retain target luminance.
pub fn draw_dab_colorize(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    opacity: u16,
) {
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        // De-premultiply
        let a = rgba[p + 3];
        let (mut r, mut g, mut b) = (0u16, 0u16, 0u16);
        if rgba[p + 3] != 0 {
            r = (((1 << 15) as u32 * rgba[p] as u32) / a as u32) as u16;
            g = (((1 << 15) as u32 * rgba[p + 1] as u32) / a as u32) as u16;
            b = (((1 << 15) as u32 * rgba[p + 2] as u32) / a as u32) as u16;
        }

        // Apply luminance
        set_rgb16_lum_from_rgb16(color_r, color_g, color_b, &mut r, &mut g, &mut b);

        // Re-premultiply
        let r = (r as u32 * a as u32 >> 15) as u16;
        let g = (g as u32 * a as u32 >> 15) as u16;
        let b = (b as u32 * a as u32 >> 15) as u16;

        // Combine as normal
        let opa_a = (mask[mi] as u32 * opacity as u32) >> 15;
        let opa_b = (1 << 15) - opa_a;
        rgba[p] = ((opa_a * r as u32 + opa_b * rgba[p] as u32) >> 15) as u16;
        rgba[p + 1] = ((opa_a * g as u32 + opa_b * rgba[p + 1] as u32) >> 15) as u16;
        rgba[p + 2] = ((opa_a * b as u32 + opa_b * rgba[p + 2] as u32) >> 15) as u16;
    });
}

/// `get_color_pixels_legacy` — masked color pickup (weights are `u32` per
/// tile; results accumulate in `f32` across tiles).
pub fn get_color_legacy(
    mask: &[u16],
    rgba: &[u16],
    sums: &mut ColorSums,
) {
    let mut weight = 0u32;
    let mut r = 0u32;
    let mut g = 0u32;
    let mut b = 0u32;
    let mut a = 0u32;
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let opa = mask[mi] as u32;
        weight += opa;
        r += opa * rgba[p] as u32 >> 15;
        g += opa * rgba[p + 1] as u32 >> 15;
        b += opa * rgba[p + 2] as u32 >> 15;
        a += opa * rgba[p + 3] as u32 >> 15;
    });
    sums.weight += weight as f32;
    sums.r += r as f32;
    sums.g += g as f32;
    sums.b += b as f32;
    sums.a += a as f32;
}

/// Accumulator for [`get_color_legacy`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ColorSums {
    pub weight: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// `get_color_pixels_accumulate` — spectral `paint` sampling remains
/// deferred (paint must be <= 0; smudge sampling with paint_mode > 0 is the
/// last spectral gap).
///
/// Pixel sampling uses the injectable [`RandomSource`] — the parity oracle
/// passes the glibc-compatible stream, production uses the portable one.
pub fn get_color_accumulate(
    mask: &[u16],
    rgba: &[u16],
    sums: &mut ColorSums,
    paint: f32,
    sample_interval: u16,
    random_sample_rate: f32,
    random: &mut dyn crate::random::RandomSource,
) {
    if paint < 0.0 {
        get_color_legacy(mask, rgba, sums);
        return;
    }
    debug_assert!(paint <= 0.0, "spectral paint sampling deferred");

    // C keeps local accumulators seeded from the sums and writes back at the
    // end; replicate that exactly.
    let mut avg_rgb = [sums.r, sums.g, sums.b];
    let mut interval_counter: u16 = 0;
    let random_sample_threshold =
        (random_sample_rate * random.rand_max() as f32) as i32;
    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        if interval_counter == 0 || random.next() < random_sample_threshold {
            let a = mask[mi] as f32 * rgba[p + 3] as f32 / (1u32 << 30) as f32;
            let alpha_sums = a + sums.a;
            sums.weight += mask[mi] as f32 / (1 << 15) as f32;
            let mut fac_a = 1.0f32;
            let mut fac_b = 1.0f32;
            if alpha_sums > 0.0 {
                fac_a = a / alpha_sums;
                fac_b = 1.0 - fac_a;
            }
            if rgba[p + 3] > 0 {
                // C: avg_rgb[i] = rgba[i]*fac_a/rgba[3] + avg_rgb[i]*fac_b;
                for i in 0..3 {
                    avg_rgb[i] =
                        rgba[p + i] as f32 * fac_a / rgba[p + 3] as f32 + avg_rgb[i] * fac_b;
                }
            }
            sums.a += a;
        }
        interval_counter = (interval_counter + 1) % sample_interval;
    });
    // paint == 0: sum = avg_rgb (spectral term absent).
    sums.r = avg_rgb[0];
    sums.g = avg_rgb[1];
    sums.b = avg_rgb[2];
}

// ---- Spectral paint modes (brushmodes.c `*_Paint`) ----
//
// libmypaint's `Normal_Paint`, `Normal_and_Eraser_Paint`, and
// `LockAlpha_Paint` — spectral (subtractive WGM) blending using fastapprox
// `fastpow`. Needed because this NG version defaults `paint_mode` to 1.0.

/// `spectral_blend_factor` — smooth additive↔spectral transition.
#[inline]
fn spectral_blend_factor(x: f32) -> f32 {
    const VER_FAC: f32 = 1.65;
    const HOR_FAC: f32 = 8.0;
    const HOR_OFFS: f32 = 3.0;
    let b = x * HOR_FAC - HOR_OFFS;
    0.5 + b / (1.0 + f32::abs(b) * VER_FAC)
}

/// `draw_dab_pixels_BlendMode_Normal_Paint`.
pub fn draw_dab_normal_paint(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    mut opacity: u16,
) {
    use crate::helpers::{fastpow, rgb_to_spectral, spectral_to_rgb, WGM_EPSILON};
    let mut spectral_a = [0.0f32; 10];
    rgb_to_spectral(
        color_r as f32 / (1 << 15) as f32,
        color_g as f32 / (1 << 15) as f32,
        color_b as f32 / (1 << 15) as f32,
        &mut spectral_a,
    );
    // pigment-mode dislikes very low opacity (int→float rounding); enforce a
    // minimum, as the C does.
    opacity = opacity.max(150);

    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let opa_a = (mask[mi] as u32 * opacity as u32) >> 15;
        let opa_b = (1 << 15) - opa_a;
        // nothing to mix with on a transparent background
        if rgba[p + 3] <= 0 {
            rgba[p + 3] = (opa_a + ((opa_b * rgba[p + 3] as u32) >> 15)) as u16;
            rgba[p] = ((opa_a * color_r as u32 + opa_b * rgba[p] as u32) >> 15) as u16;
            rgba[p + 1] = ((opa_a * color_g as u32 + opa_b * rgba[p + 1] as u32) >> 15) as u16;
            rgba[p + 2] = ((opa_a * color_b as u32 + opa_b * rgba[p + 2] as u32) >> 15) as u16;
            continue;
        }
        let fac_a = opa_a as f32 / (opa_a as f32 + opa_b as f32 * rgba[p + 3] as f32 / (1 << 15) as f32);
        let fac_b = 1.0 - fac_a;

        let mut spectral_b = [0.0f32; 10];
        rgb_to_spectral(
            rgba[p] as f32 / rgba[p + 3] as f32,
            rgba[p + 1] as f32 / rgba[p + 3] as f32,
            rgba[p + 2] as f32 / rgba[p + 3] as f32,
            &mut spectral_b,
        );

        let mut spectral_result = [0.0f32; 10];
        for i in 0..10 {
            spectral_result[i] = fastpow(spectral_a[i], fac_a) * fastpow(spectral_b[i], fac_b);
        }

        let mut rgb_result = [0.0f32; 3];
        spectral_to_rgb(&spectral_result, &mut rgb_result);
        rgba[p + 3] = (opa_a + ((opa_b * rgba[p + 3] as u32) >> 15)) as u16;
        for i in 0..3 {
            rgba[p + i] = (rgb_result[i] * rgba[p + 3] as f32 + 0.5) as u16;
        }
    });
    let _ = WGM_EPSILON;
}

/// `draw_dab_pixels_BlendMode_Normal_and_Eraser_Paint`.
pub fn draw_dab_normal_and_eraser_paint(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    color_a: u16,
    opacity: u16,
) {
    use crate::helpers::{fastpow, rgb_to_spectral, spectral_to_rgb};
    let mut spectral_a = [0.0f32; 10];
    rgb_to_spectral(
        color_r as f32 / (1 << 15) as f32,
        color_g as f32 / (1 << 15) as f32,
        color_b as f32 / (1 << 15) as f32,
        &mut spectral_a,
    );

    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let opa_a = (mask[mi] as u32 * opacity as u32) >> 15;
        let opa_b = (1 << 15) - opa_a;
        let opa_a2 = (opa_a * color_a as u32) >> 15;
        let opa_out = opa_a2 + ((opa_b * rgba[p + 3] as u32) >> 15);

        let mut rgb = [0u32; 3];

        let spectral_factor = crate::helpers::clamp(
            spectral_blend_factor(rgba[p + 3] as f32 / (1 << 15) as f32),
            0.0,
            1.0,
        );
        let additive_factor = 1.0 - spectral_factor;

        if additive_factor != 0.0 {
            rgb[0] = (opa_a2 * color_r as u32 + opa_b * rgba[p] as u32) >> 15;
            rgb[1] = (opa_a2 * color_g as u32 + opa_b * rgba[p + 1] as u32) >> 15;
            rgb[2] = (opa_a2 * color_b as u32 + opa_b * rgba[p + 2] as u32) >> 15;
        }

        if spectral_factor != 0.0 && rgba[p + 3] != 0 {
            let mut spectral_b = [0.0f32; 10];
            rgb_to_spectral(
                rgba[p] as f32 / rgba[p + 3] as f32,
                rgba[p + 1] as f32 / rgba[p + 3] as f32,
                rgba[p + 2] as f32 / rgba[p + 3] as f32,
                &mut spectral_b,
            );

            let mut fac_a = opa_a as f32 / (opa_a as f32 + opa_b as f32 * rgba[p + 3] as f32 / (1 << 15) as f32);
            fac_a *= color_a as f32 / (1 << 15) as f32;
            let fac_b = 1.0 - fac_a;

            let mut spectral_result = [0.0f32; 10];
            for i in 0..10 {
                spectral_result[i] = fastpow(spectral_a[i], fac_a) * fastpow(spectral_b[i], fac_b);
            }

            let mut rgb_result = [0.0f32; 3];
            spectral_to_rgb(&spectral_result, &mut rgb_result);

            for i in 0..3 {
                rgb[i] = (additive_factor * rgb[i] as f32 + spectral_factor * rgb_result[i] * opa_out as f32) as u32;
            }
        }

        rgba[p + 3] = opa_out as u16;
        for i in 0..3 {
            rgba[p + i] = rgb[i] as u16;
        }
    });
}

/// `draw_dab_pixels_BlendMode_LockAlpha_Paint`.
pub fn draw_dab_lock_alpha_paint(
    mask: &[u16],
    rgba: &mut [u16],
    color_r: u16,
    color_g: u16,
    color_b: u16,
    opacity: u16,
) {
    use crate::helpers::{fastpow, rgb_to_spectral, spectral_to_rgb};
    let mut spectral_a = [0.0f32; 10];
    rgb_to_spectral(
        color_r as f32 / (1 << 15) as f32,
        color_g as f32 / (1 << 15) as f32,
        color_b as f32 / (1 << 15) as f32,
        &mut spectral_a,
    );
    let opacity = opacity.max(150);

    for_each_masked_pixel!(mask, mi, pi, {
        let p = pi * 4;
        let mut opa_a = (mask[mi] as u32 * opacity as u32) >> 15;
        let opa_b = (1 << 15) - opa_a;
        opa_a = (opa_a * rgba[p + 3] as u32) >> 15;
        if rgba[p + 3] == 0 {
            continue;
        }
        let fac_a = opa_a as f32 / (opa_a as f32 + opa_b as f32 * rgba[p + 3] as f32 / (1 << 15) as f32);
        let fac_b = 1.0 - fac_a;
        let mut spectral_b = [0.0f32; 10];
        rgb_to_spectral(
            rgba[p] as f32 / rgba[p + 3] as f32,
            rgba[p + 1] as f32 / rgba[p + 3] as f32,
            rgba[p + 2] as f32 / rgba[p + 3] as f32,
            &mut spectral_b,
        );

        let mut spectral_result = [0.0f32; 10];
        for i in 0..10 {
            spectral_result[i] = fastpow(spectral_a[i], fac_a) * fastpow(spectral_b[i], fac_b);
        }
        let mut rgb_result = [0.0f32; 3];
        spectral_to_rgb(&spectral_result, &mut rgb_result);

        for i in 0..3 {
            rgba[p + i] = (rgb_result[i] * rgba[p + 3] as f32 + 0.5) as u16;
        }
    });
}
