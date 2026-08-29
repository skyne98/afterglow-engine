//! Arithmetic spot-checks against the exact C semantics of
//! `brushmodes.c` / `mypaint-tiled-surface.c` (premultiplied fix15).

use maipointo::brushmodes::*;
use maipointo::mask::render_dab_mask;
use maipointo::{Tile, TILE_SIZE};

/// Opaque red dab stamped once over transparent tile: over-op with
/// full opacity must produce solid red, exactly.
#[test]
fn normal_dab_solid_red_over_transparent() {
    let mut tile = Tile::new();
    let mut scratch = Vec::new();
    let mut mask = Vec::new();
    render_dab_mask(&mut mask, 32.0, 32.0, 8.0, 0.8, 0.0, 1.0, 0.0, &mut scratch);
    assert!(!mask.is_empty(), "mask must cover the dab");

    let painted_before = mask.iter().filter(|&&v| v != 0).count();
    assert!(painted_before > 0);

    draw_dab_normal(&mask, &mut tile.data, 32768, 0, 0, 32768);
    // Center pixel must be fully opaque red (mask center = max opacity 32768
    // for hardness 0.8: rr=0 → opa=1.0 → 32768).
    let c = tile.pixel(32, 32);
    // Pixel center (32.5, 32.5) is offset 0.5 from the dab center: rr =
    // 0.5/64, so opa = 1 + rr*(-0.25) = 0.998046875 → 32704, not 32768.
    assert_eq!(c, [32704, 0, 0, 32704]);
}

/// C arithmetic check: `opa_a = mask*opacity>>15`, then
/// `a = opa_a + opa_b*bottom/32768` and
/// `c = (opa_a*col + opa_b*bottom)/32768`, all u32 truncating.
#[test]
fn normal_blend_matches_c_integer_math() {
    // mask=16384, opacity=16384 → opa_a = 16384*16384>>15 = 8192.
    let mut rgba = [8192u16, 16384, 24576, 16384]; // premult bottom
    let mask = [16384u16, 0, 0]; // single covered pixel, then terminator
    draw_dab_normal(&mask, &mut rgba, 32768, 16384, 0, 16384);
    let opa_a: u32 = 8192;
    let opa_b: u32 = 32768 - 8192;
    let expect = |bottom: u32, col: u32| -> u16 {
        ((opa_a * col + opa_b * bottom) >> 15) as u16
    };
    assert_eq!(rgba[0], expect(8192, 32768));
    assert_eq!(rgba[1], expect(16384, 16384));
    assert_eq!(rgba[2], expect(24576, 0));
    assert_eq!(rgba[3], (opa_a + (opa_b * 16384u32 >> 15)) as u16);
}

/// Eraser: color_a = 0 must drag alpha toward zero exactly like the C
/// (`opa_a = mask*opacity>>15; opa_a = opa_a*color_a>>15` → 0).
#[test]
fn eraser_reduces_alpha() {
    let mut rgba = [100u16, 100, 100, 32768];
    let mask = [32768u16, 0, 0]; // single fully-covered pixel
    draw_dab_normal_and_eraser(&mask, &mut rgba, 0, 0, 0, 0, 32768);
    // opa_a = 32768*32768>>15 = 32768; then *0>>15 = 0; bottom alpha kept:
    // a = 0 + 0*32768>>15 → wait: opa_b = 32768-32768 = 0 → alpha = 0.
    assert_eq!(rgba[3], 0);
    assert_eq!(rgba[0], 0);
}

/// Smudge semantics: color_a = 0.6*32768 ≈ 19660 keeps 60% opacity target.
#[test]
fn smudge_partial_color_a() {
    let mut rgba = [0u16, 0, 0, 0];
    let mask = [32768u16, 0, 0];
    let color_a = 19660u16; // 0.6 * 32768 = 19660.8 → 19660
    draw_dab_normal_and_eraser(&mask, &mut rgba, 32768, 0, 0, color_a, 32768);
    let opa_a = ((32768u32 * 32768u32) >> 15) * color_a as u32 >> 15;
    assert_eq!(rgba[3], opa_a as u16);
    assert_eq!(rgba[3], ((32768u32 * 19660u32) >> 15) as u16);
}

/// Lock alpha: stamping color over a transparent pixel must NOT add alpha.
#[test]
fn lock_alpha_preserves_transparency() {
    let mut rgba = [0u16, 0, 0, 0];
    let mask = [32768u16, 0, 0];
    draw_dab_lock_alpha(&mask, &mut rgba, 32768, 32768, 32768, 32768);
    // opa_a = 32768 * 0 >> 15 = 0 → colors unchanged (0).
    assert_eq!(rgba, [0, 0, 0, 0]);

    let mut rgba2 = [0u16, 0, 0, 16384]; // 50% opaque bottom
    draw_dab_lock_alpha(&mask, &mut rgba2, 32768, 0, 0, 32768);
    // opa_a = 32768*16384>>15 = 16384; r = (16384*32768 + 16384*0)>>15 = 16384.
    assert_eq!(rgba2, [16384, 0, 0, 16384]);
}

/// Posterize with num=2 must snap channels to {0, 32768} at full opacity.
#[test]
fn posterize_snaps_channels() {
    let mut rgba = [12000u16, 20000, 30000, 32768];
    let mask = [32768u16, 0, 0];
    draw_dab_posterize(&mask, &mut rgba, 32768, 2);
    // ROUND(12000/32768*2)=ROUND(0.732)=1 → 32768*1/2 = 16384
    // ROUND(20000/32768*2)=ROUND(1.220)=1 → 16384
    // ROUND(30000/32768*2)=ROUND(1.831)=2 → 32768
    assert_eq!(rgba[0], 16384);
    assert_eq!(rgba[1], 16384);
    assert_eq!(rgba[2], 32768);
    assert_eq!(rgba[3], 32768); // alpha untouched
}

/// get_color legacy: sums opa-weighted straight color over the mask.
#[test]
fn get_color_legacy_sums() {
    let rgba = [32768u16, 0, 0, 32768]; // opaque red
    let mask = [16384u16, 0, 0]; // half-opacity mask pixel
    let mut sums = ColorSums::default();
    get_color_legacy(&mask, &rgba, &mut sums);
    // opa=16384: weight += 16384; r += 16384*32768>>15 = 16384.
    assert_eq!(sums.weight, 16384.0);
    assert_eq!(sums.r, 16384.0);
    assert_eq!(sums.a, 16384.0);
}

/// LRE walking: a run, then a `(0, skip*4)` marker, then another run.
/// After painting pixel L the walker sits at L+1; skip s pixels lands at
/// L+1+s (skip counts only the zero pixels in between) — pixel 0 + skip 10
/// → pixel 11.
#[test]
fn lre_skip_offsets_pixels() {
    let mask = [32768u16, 0, 40, 32768u16, 0, 0];
    let mut tile = Tile::new();
    draw_dab_normal(&mask, &mut tile.data, 32768, 0, 0, 32768);
    assert_eq!(tile.pixel(0, 0), [32768, 0, 0, 32768]);
    assert_eq!(tile.pixel(11, 0), [32768, 0, 0, 32768]);
    assert_eq!(tile.pixel(1, 0), [0, 0, 0, 0]);
    assert_eq!(tile.pixel(10, 0), [0, 0, 0, 0]);
}

/// Mask of a small round dab: covered-pixel count is stable and the mask
/// is properly LRE-terminated with (0, 0).
#[test]
fn mask_encoding_terminated() {
    let mut scratch = Vec::new();
    let mut mask = Vec::new();
    render_dab_mask(&mut mask, 32.0, 32.0, 4.0, 0.5, 0.0, 1.0, 0.0, &mut scratch);
    let n = mask.len();
    assert!(n >= 2);
    assert_eq!(mask[n - 2], 0);
    assert_eq!(mask[n - 1], 0);
}

/// Colorize keeps the bottom luminance: white bottom + red top → white.
#[test]
fn colorize_retains_luminance() {
    // Bottom: 50% gray, opaque. Top color: pure red (32768, 0, 0).
    // Luma(red) < luma(gray); set_lum must clip to stay near gray luma.
    let mut rgba = [16384u16, 16384, 16384, 32768];
    let mask = [32768u16, 0, 0];
    draw_dab_colorize(&mask, &mut rgba, 32768, 0, 0, 32768);
    // Result luminance must stay at the bottom's luma (16384-ish per channel
    // after clipping); red channel dominates only if hue kept — with equal
    // luma the clip path forces near-gray.
    let c = rgba;
    let lum_after = 0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32;
    let lum_before = 0.2126 * 16384.0 + 0.7152 * 16384.0 + 0.0722 * 16384.0;
    assert!((lum_after - lum_before).abs() < 64.0, "luma drifted: {:?}", c);
}
