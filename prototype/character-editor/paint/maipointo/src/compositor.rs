//! Layer composition — port of the demo's `layer-compositor.c`.
//!
//! Tiles are RGBA `u16` fix15 (`[0, 2^15]`), **premultiplied** alpha, exactly
//! like [`crate::brushmodes`]. All blend math is fixed-point u15 integer
//! arithmetic transcribed bit-for-bit from the C (including the fix15
//! integer sqrt used by Soft-light), so layer stacks match the reference
//! compositor byte for byte.

use crate::helpers::{fastpow, rgb_to_spectral, spectral_to_rgb};

pub const U15_ONE: u32 = 32768;

/// Layer/group blend modes — order matches the demo's `WEB_MODE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendMode {
    Normal = 0,
    Multiply = 1,
    Screen = 2,
    Overlay = 3,
    Darken = 4,
    Lighten = 5,
    HardLight = 6,
    SoftLight = 7,
    ColorBurn = 8,
    ColorDodge = 9,
    Difference = 10,
    Exclusion = 11,
    Hue = 12,
    Saturation = 13,
    Color = 14,
    Luminosity = 15,
    Plus = 16,
    DestinationIn = 17,
    DestinationOut = 18,
    SourceAtop = 19,
    DestinationAtop = 20,
    Pigment = 21,
}

pub const BLEND_MODE_COUNT: usize = 22;

impl BlendMode {
    /// The C switch works on raw ints; unknown values fall back to Normal.
    pub fn from_int(v: i32) -> BlendMode {
        if (0..=(BLEND_MODE_COUNT as i32 - 1)).contains(&v) {
            // SAFETY: every value in 0..22 has a variant (repr check below
            // via the match in from_int_raw).
            Self::from_int_raw(v)
        } else {
            BlendMode::Normal
        }
    }

    const fn from_int_raw(v: i32) -> BlendMode {
        match v {
            0 => BlendMode::Normal,
            1 => BlendMode::Multiply,
            2 => BlendMode::Screen,
            3 => BlendMode::Overlay,
            4 => BlendMode::Darken,
            5 => BlendMode::Lighten,
            6 => BlendMode::HardLight,
            7 => BlendMode::SoftLight,
            8 => BlendMode::ColorBurn,
            9 => BlendMode::ColorDodge,
            10 => BlendMode::Difference,
            11 => BlendMode::Exclusion,
            12 => BlendMode::Hue,
            13 => BlendMode::Saturation,
            14 => BlendMode::Color,
            15 => BlendMode::Luminosity,
            16 => BlendMode::Plus,
            17 => BlendMode::DestinationIn,
            18 => BlendMode::DestinationOut,
            19 => BlendMode::SourceAtop,
            20 => BlendMode::DestinationAtop,
            _ => BlendMode::Pigment,
        }
    }

    pub fn as_int(self) -> i32 {
        self as i32
    }
}

type U15 = u32;
type I15 = i32;

#[inline]
fn u15_clamp(value: I15) -> U15 {
    if value <= 0 {
        0
    } else if value as U15 >= U15_ONE {
        U15_ONE
    } else {
        value as U15
    }
}

#[inline]
fn u15_mul(a: U15, b: U15) -> U15 {
    (a.wrapping_mul(b)) >> 15
}

#[inline]
fn u15_sumprods(a1: U15, a2: U15, b1: U15, b2: U15) -> U15 {
    // C: (a1*a2 + b1*b2) in uint32 (wrapping); the sum can exceed 2^32
    ((a1.wrapping_mul(a2)).wrapping_add(b1.wrapping_mul(b2))) >> 15
}

#[inline]
fn u15_div(a: U15, b: U15) -> U15 {
    if b == 0 {
        U15_ONE
    } else {
        (a << 15) / b
    }
}

fn u15_opacity(opacity: f32) -> U15 {
    if !opacity.is_finite() || opacity <= 0.0 {
        return 0;
    }
    if opacity >= 1.0 {
        return U15_ONE;
    }
    (opacity * U15_ONE as f32) as U15
}

/// Integer sqrt in fix15, transcribing MyPaint's `fix15_sqrt` (table +
/// Babylon) bit-for-bit, so Soft-light matches the reference layer stack.
fn u15_sqrt(value: U15) -> U15 {
    if value == 0 || value == U15_ONE {
        return value;
    }
    const APPROX16: [u16; 16] = [
        16383, 23169, 28376, 32767, 36634, 40131, 43346, 46339, 49151, 51809, 54338, 56754,
        59072, 61302, 63453, 65535,
    ];
    // One extra bit of precision for working; 1.0 would overflow, so the
    // caller guarantees value < 1.0 here (value == U15_ONE returns early).
    let s = (value << 1) as u32;
    const FRACBITS: u32 = 16;
    let mut n = APPROX16[(s >> 12) as usize] as u32;
    for _ in 0..15 {
        let n_old = n;
        n += (s << FRACBITS) / n;
        n >>= 1;
        if n == n_old || (n > n_old && n - 1 == n_old) || (n < n_old && n + 1 == n_old) {
            break;
        }
    }
    n >> 1
}

fn blend_channel(source: U15, backdrop: U15, mode: BlendMode) -> U15 {
    match mode {
        BlendMode::Normal => source,
        BlendMode::Multiply => u15_mul(source, backdrop),
        BlendMode::Screen => backdrop + source - u15_mul(backdrop, source),
        BlendMode::Overlay => {
            let two_backdrop = backdrop << 1;
            if two_backdrop <= U15_ONE {
                u15_mul(source, two_backdrop)
            } else {
                let remainder = two_backdrop - U15_ONE;
                source + remainder - u15_mul(source, remainder)
            }
        }
        BlendMode::Darken => {
            if source < backdrop {
                source
            } else {
                backdrop
            }
        }
        BlendMode::Lighten => {
            if source > backdrop {
                source
            } else {
                backdrop
            }
        }
        BlendMode::HardLight => {
            let two_source = source << 1;
            if two_source <= U15_ONE {
                u15_mul(backdrop, two_source)
            } else {
                let remainder = two_source - U15_ONE;
                backdrop + remainder - u15_mul(backdrop, remainder)
            }
        }
        BlendMode::SoftLight => {
            let two_source = source << 1;
            if two_source <= U15_ONE {
                let result = U15_ONE - u15_mul(U15_ONE - two_source, U15_ONE - backdrop);
                u15_mul(result, backdrop)
            } else {
                let four_backdrop = backdrop << 2;
                let d;
                if four_backdrop <= U15_ONE {
                    let backdrop_squared = u15_mul(backdrop, backdrop);
                    d = four_backdrop + 16 * u15_mul(backdrop_squared, backdrop)
                        - 12 * backdrop_squared;
                } else {
                    d = u15_sqrt(backdrop);
                }
                backdrop + u15_mul(two_source - U15_ONE, d - backdrop)
            }
        }
        BlendMode::ColorBurn => {
            if source > 0 {
                let value = u15_div(U15_ONE - backdrop, source);
                if value < U15_ONE {
                    return U15_ONE - value;
                }
            }
            0
        }
        BlendMode::ColorDodge => {
            if source < U15_ONE {
                let value = u15_div(backdrop, U15_ONE - source);
                if value < U15_ONE {
                    return value;
                }
            }
            U15_ONE
        }
        BlendMode::Difference => {
            if source >= backdrop {
                source - backdrop
            } else {
                backdrop - source
            }
        }
        BlendMode::Exclusion => backdrop + source - (u15_mul(backdrop, source) << 1),
        _ => source,
    }
}

#[inline]
fn channel_min(r: I15, g: I15, b: I15) -> I15 {
    let result = if r < g { r } else { g };
    if result < b {
        result
    } else {
        b
    }
}

#[inline]
fn channel_max(r: I15, g: I15, b: I15) -> I15 {
    let result = if r > g { r } else { g };
    if result > b {
        result
    } else {
        b
    }
}

#[inline]
fn luminance(r: I15, g: I15, b: I15) -> I15 {
    const LUM_R: u32 = 9830;
    const LUM_G: u32 = 19333;
    const LUM_B: u32 = 3604;
    ((r as u32).wrapping_mul(LUM_R)
        .wrapping_add((g as u32).wrapping_mul(LUM_G))
        .wrapping_add((b as u32).wrapping_mul(LUM_B))
        / U15_ONE) as I15
}

fn clip_color(r: &mut I15, g: &mut I15, b: &mut I15) {
    let lum = luminance(*r, *g, *b);
    let minimum = channel_min(*r, *g, *b);
    let maximum = channel_max(*r, *g, *b);
    if minimum < 0 {
        let denominator = lum - minimum;
        *r = lum + ((*r - lum) * lum) / denominator;
        *g = lum + ((*g - lum) * lum) / denominator;
        *b = lum + ((*b - lum) * lum) / denominator;
    }
    if maximum > U15_ONE as I15 {
        let denominator = maximum - lum;
        let one_minus_lum = U15_ONE as I15 - lum;
        *r = lum + ((*r - lum) * one_minus_lum) / denominator;
        *g = lum + ((*g - lum) * one_minus_lum) / denominator;
        *b = lum + ((*b - lum) * one_minus_lum) / denominator;
    }
}

fn set_luminance(r: &mut I15, g: &mut I15, b: &mut I15, lum: I15) {
    let difference = lum - luminance(*r, *g, *b);
    *r += difference;
    *g += difference;
    *b += difference;
    clip_color(r, g, b);
}

#[inline]
fn saturation(r: I15, g: I15, b: I15) -> I15 {
    channel_max(r, g, b) - channel_min(r, g, b)
}

fn set_saturation(r: &mut I15, g: &mut I15, b: &mut I15, sat: I15) {
    // The C sorts three POINTERS (top=&b, middle=&g, bottom=&r) by value and
    // writes back through the sorted pointers. Track the pointer identity.
    #[derive(Clone, Copy, PartialEq)]
    enum Slot {
        R,
        G,
        B,
    }
    let mut vals = [*r, *g, *b];
    let (mut top, mut middle, mut bottom) = (Slot::B, Slot::G, Slot::R);
    if vals[top as usize] < vals[middle as usize] {
        std::mem::swap(&mut top, &mut middle);
    }
    if vals[top as usize] < vals[bottom as usize] {
        std::mem::swap(&mut top, &mut bottom);
    }
    if vals[middle as usize] < vals[bottom as usize] {
        std::mem::swap(&mut middle, &mut bottom);
    }
    if vals[top as usize] > vals[bottom as usize] {
        vals[middle as usize] =
            (vals[middle as usize] - vals[bottom as usize]) * sat
                / (vals[top as usize] - vals[bottom as usize]);
        vals[top as usize] = sat;
    } else {
        vals[top as usize] = 0;
        vals[middle as usize] = 0;
    }
    vals[bottom as usize] = 0;
    *r = vals[0];
    *g = vals[1];
    *b = vals[2];
}

#[allow(clippy::too_many_arguments)]
fn nonseparable_color(
    source_r: U15,
    source_g: U15,
    source_b: U15,
    backdrop_r: U15,
    backdrop_g: U15,
    backdrop_b: U15,
    mode: BlendMode,
    out_r: &mut U15,
    out_g: &mut U15,
    out_b: &mut U15,
) {
    let backdrop_lum = luminance(
        backdrop_r as I15,
        backdrop_g as I15,
        backdrop_b as I15,
    );
    let (mut r, mut g, mut b);
    match mode {
        BlendMode::Hue => {
            r = source_r as I15;
            g = source_g as I15;
            b = source_b as I15;
            set_saturation(
                &mut r,
                &mut g,
                &mut b,
                saturation(
                    backdrop_r as I15,
                    backdrop_g as I15,
                    backdrop_b as I15,
                ),
            );
            set_luminance(&mut r, &mut g, &mut b, backdrop_lum);
        }
        BlendMode::Saturation => {
            r = backdrop_r as I15;
            g = backdrop_g as I15;
            b = backdrop_b as I15;
            set_saturation(
                &mut r,
                &mut g,
                &mut b,
                saturation(source_r as I15, source_g as I15, source_b as I15),
            );
            set_luminance(&mut r, &mut g, &mut b, backdrop_lum);
        }
        BlendMode::Color => {
            r = source_r as I15;
            g = source_g as I15;
            b = source_b as I15;
            set_luminance(&mut r, &mut g, &mut b, backdrop_lum);
        }
        _ => {
            r = backdrop_r as I15;
            g = backdrop_g as I15;
            b = backdrop_b as I15;
            set_luminance(
                &mut r,
                &mut g,
                &mut b,
                luminance(source_r as I15, source_g as I15, source_b as I15),
            );
        }
    }
    *out_r = u15_clamp(r);
    *out_g = u15_clamp(g);
    *out_b = u15_clamp(b);
}

fn pigment_blend(dst: &mut [u16], src: &[u16], source_alpha: U15, opacity: U15) {
    let backdrop_alpha = dst[3] as U15;
    let one_minus_source = U15_ONE - source_alpha;
    if backdrop_alpha == 0 || source_alpha == 0 || source_alpha == U15_ONE {
        dst[0] = u15_clamp(
            u15_sumprods(src[0] as U15, opacity, one_minus_source, dst[0] as U15) as I15,
        ) as u16;
        dst[1] = u15_clamp(
            u15_sumprods(src[1] as U15, opacity, one_minus_source, dst[1] as U15) as I15,
        ) as u16;
        dst[2] = u15_clamp(
            u15_sumprods(src[2] as U15, opacity, one_minus_source, dst[2] as U15) as I15,
        ) as u16;
        dst[3] = u15_clamp(
            (source_alpha + u15_mul(backdrop_alpha, one_minus_source)) as I15,
        ) as u16;
        return;
    }

    let denominator = (source_alpha
        + ((one_minus_source * backdrop_alpha) / U15_ONE)) as f32;
    let source_factor = source_alpha as f32 / denominator;
    let backdrop_factor = 1.0f32 - source_factor;
    let mut source_spectral = [0.0f32; 10];
    let mut backdrop_spectral = [0.0f32; 10];
    rgb_to_spectral(
        src[0] as f32 / src[3] as f32,
        src[1] as f32 / src[3] as f32,
        src[2] as f32 / src[3] as f32,
        &mut source_spectral,
    );
    rgb_to_spectral(
        dst[0] as f32 / dst[3] as f32,
        dst[1] as f32 / dst[3] as f32,
        dst[2] as f32 / dst[3] as f32,
        &mut backdrop_spectral,
    );
    let mut result_spectral = [0.0f32; 10];
    for i in 0..10 {
        result_spectral[i] = fastpow(source_spectral[i], source_factor)
            * fastpow(backdrop_spectral[i], backdrop_factor);
    }
    let mut rgb = [0.0f32; 3];
    spectral_to_rgb(&result_spectral, &mut rgb);
    let out_alpha =
        u15_clamp((source_alpha + u15_mul(backdrop_alpha, one_minus_source)) as I15);
    dst[0] = (rgb[0] * (out_alpha as f32 + 0.5)) as u16;
    dst[1] = (rgb[1] * (out_alpha as f32 + 0.5)) as u16;
    dst[2] = (rgb[2] * (out_alpha as f32 + 0.5)) as u16;
    dst[3] = out_alpha as u16;
}

/// `afterglow_layer_blend_over` — blend one premultiplied fix15 pixel of a
/// source layer onto a destination tile pixel.
pub fn layer_blend_over(dst: &mut [u16], src: &[u16], opacity: f32, mode: BlendMode) {
    let source_opacity = u15_opacity(opacity);
    let source_alpha = u15_mul(src[3] as U15, source_opacity);
    let backdrop_alpha = dst[3] as U15;
    let source_r = if src[3] != 0 {
        u15_clamp(u15_div(src[0] as U15, src[3] as U15) as I15)
    } else {
        0
    };
    let source_g = if src[3] != 0 {
        u15_clamp(u15_div(src[1] as U15, src[3] as U15) as I15)
    } else {
        0
    };
    let source_b = if src[3] != 0 {
        u15_clamp(u15_div(src[2] as U15, src[3] as U15) as I15)
    } else {
        0
    };
    let backdrop_r = if dst[3] != 0 {
        u15_clamp(u15_div(dst[0] as U15, dst[3] as U15) as I15)
    } else {
        0
    };
    let backdrop_g = if dst[3] != 0 {
        u15_clamp(u15_div(dst[1] as U15, dst[3] as U15) as I15)
    } else {
        0
    };
    let backdrop_b = if dst[3] != 0 {
        u15_clamp(u15_div(dst[2] as U15, dst[3] as U15) as I15)
    } else {
        0
    };
    let one_minus_source = U15_ONE - source_alpha;

    if mode == BlendMode::Pigment {
        pigment_blend(dst, src, source_alpha, source_opacity);
        return;
    }
    if mode == BlendMode::Normal {
        dst[0] = u15_clamp(u15_sumprods(
            src[0] as U15,
            source_opacity,
            one_minus_source,
            dst[0] as U15,
        ) as I15) as u16;
        dst[1] = u15_clamp(u15_sumprods(
            src[1] as U15,
            source_opacity,
            one_minus_source,
            dst[1] as U15,
        ) as I15) as u16;
        dst[2] = u15_clamp(u15_sumprods(
            src[2] as U15,
            source_opacity,
            one_minus_source,
            dst[2] as U15,
        ) as I15) as u16;
        dst[3] = u15_clamp(
            (source_alpha + u15_mul(backdrop_alpha, one_minus_source)) as I15,
        ) as u16;
        return;
    }
    if mode == BlendMode::Plus {
        dst[0] = u15_clamp((u15_mul(source_r, source_alpha) + dst[0] as U15) as I15) as u16;
        dst[1] = u15_clamp((u15_mul(source_g, source_alpha) + dst[1] as U15) as I15) as u16;
        dst[2] = u15_clamp((u15_mul(source_b, source_alpha) + dst[2] as U15) as I15) as u16;
        dst[3] = u15_clamp((backdrop_alpha + source_alpha) as I15) as u16;
        return;
    }
    if mode == BlendMode::DestinationIn || mode == BlendMode::DestinationOut {
        let factor = if mode == BlendMode::DestinationIn {
            source_alpha
        } else {
            one_minus_source
        };
        dst[0] = u15_mul(dst[0] as U15, factor) as u16;
        dst[1] = u15_mul(dst[1] as U15, factor) as u16;
        dst[2] = u15_mul(dst[2] as U15, factor) as u16;
        dst[3] = u15_mul(dst[3] as U15, factor) as u16;
        return;
    }
    if mode == BlendMode::SourceAtop {
        let source_red = u15_mul(src[0] as U15, source_opacity);
        let source_green = u15_mul(src[1] as U15, source_opacity);
        let source_blue = u15_mul(src[2] as U15, source_opacity);
        dst[0] = u15_clamp(u15_sumprods(
            source_red,
            backdrop_alpha,
            dst[0] as U15,
            one_minus_source,
        ) as I15) as u16;
        dst[1] = u15_clamp(u15_sumprods(
            source_green,
            backdrop_alpha,
            dst[1] as U15,
            one_minus_source,
        ) as I15) as u16;
        dst[2] = u15_clamp(u15_sumprods(
            source_blue,
            backdrop_alpha,
            dst[2] as U15,
            one_minus_source,
        ) as I15) as u16;
        return;
    }
    if mode == BlendMode::DestinationAtop {
        let source_red = u15_mul(src[0] as U15, source_opacity);
        let source_green = u15_mul(src[1] as U15, source_opacity);
        let source_blue = u15_mul(src[2] as U15, source_opacity);
        let one_minus_backdrop = U15_ONE - backdrop_alpha;
        dst[0] = u15_clamp(u15_sumprods(
            source_red,
            one_minus_backdrop,
            dst[0] as U15,
            source_alpha,
        ) as I15) as u16;
        dst[1] = u15_clamp(u15_sumprods(
            source_green,
            one_minus_backdrop,
            dst[1] as U15,
            source_alpha,
        ) as I15) as u16;
        dst[2] = u15_clamp(u15_sumprods(
            source_blue,
            one_minus_backdrop,
            dst[2] as U15,
            source_alpha,
        ) as I15) as u16;
        dst[3] = source_alpha as u16;
        return;
    }

    let (mut blend_r, mut blend_g, mut blend_b) = (source_r, source_g, source_b);
    if (BlendMode::Hue as u8) <= (mode as u8) && (mode as u8) <= (BlendMode::Luminosity as u8)
    {
        nonseparable_color(
            source_r,
            source_g,
            source_b,
            backdrop_r,
            backdrop_g,
            backdrop_b,
            mode,
            &mut blend_r,
            &mut blend_g,
            &mut blend_b,
        );
    } else {
        blend_r = blend_channel(source_r, backdrop_r, mode);
        blend_g = blend_channel(source_g, backdrop_g, mode);
        blend_b = blend_channel(source_b, backdrop_b, mode);
    }

    let one_minus_backdrop = U15_ONE - backdrop_alpha;
    let composite_r = u15_sumprods(one_minus_backdrop, source_r, backdrop_alpha, blend_r);
    let composite_g = u15_sumprods(one_minus_backdrop, source_g, backdrop_alpha, blend_g);
    let composite_b = u15_sumprods(one_minus_backdrop, source_b, backdrop_alpha, blend_b);
    dst[0] = u15_clamp(u15_sumprods(
        source_alpha,
        composite_r,
        one_minus_source,
        dst[0] as U15,
    ) as I15) as u16;
    dst[1] = u15_clamp(u15_sumprods(
        source_alpha,
        composite_g,
        one_minus_source,
        dst[1] as U15,
    ) as I15) as u16;
    dst[2] = u15_clamp(u15_sumprods(
        source_alpha,
        composite_b,
        one_minus_source,
        dst[2] as U15,
    ) as I15) as u16;
    dst[3] = u15_clamp(
        (source_alpha + u15_mul(backdrop_alpha, one_minus_source)) as I15,
    ) as u16;
}
