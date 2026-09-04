//! Pixel mask generation for dabs — port of libmypaint's
//! `mypaint-tiled-surface.c` dab rendering (`render_dab_mask` and its
//! `calculate_*` helpers), including the LRE (run-length encoded) mask
//! format consumed by the blend modes.
//!
//! Conventions (matching the C exactly):
//! - Dab position `(x, y)` is in tile-local pixel space (f32, sub-pixel).
//! - `rr = (distance_from_center / radius)²`, computed with pixel centers.
//! - The mask holds `u16` opacities in `[0, 2^15]`; zero-opacity pixels are
//!   skipped via `(0, skip*4)` LRE marker pairs.
//!
//! Float order of operations is preserved verbatim from the C so the output
//! is bit-identical on the same target.

/// Tile edge length; libmypaint tiles are 64×64.
pub const TILE_SIZE: usize = 64;

/// `CLAMP(x, low, high)` from helpers.h.
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

/// `calculate_r_sample` — returns the *squared* elliptical distance from the
/// dab center in unnormalized pixel coordinates.
#[inline]
fn calculate_r_sample(x: f32, y: f32, aspect_ratio: f32, sn: f32, cs: f32) -> f32 {
    let yyr = (y * cs - x * sn) * aspect_ratio;
    let xxr = y * sn + x * cs;
    yyr * yyr + xxr * xxr
}

/// `calculate_rr` — squared normalized distance for the non-AA path.
#[inline]
fn calculate_rr(
    xp: i32,
    yp: i32,
    x: f32,
    y: f32,
    aspect_ratio: f32,
    sn: f32,
    cs: f32,
    one_over_radius2: f32,
) -> f32 {
    let yy = yp as f32 + 0.5 - y;
    let xx = xp as f32 + 0.5 - x;
    let yyr = (yy * cs - xx * sn) * aspect_ratio;
    let xxr = yy * sn + xx * cs;
    (yyr * yyr + xxr * xxr) * one_over_radius2
}

#[inline]
fn sign_point_in_line(px: f32, py: f32, vx: f32, vy: f32) -> f32 {
    (px - vx) * (-vy) - vx * (py - vy)
}

#[inline]
fn closest_point_to_line(lx: f32, ly: f32, px: f32, py: f32) -> (f32, f32) {
    let l2 = lx * lx + ly * ly;
    let ltp_dot = px * lx + py * ly;
    let t = ltp_dot / l2;
    (lx * t, ly * t)
}

/// `calculate_rr_antialiased` — visibility at the nearest point divided by
/// `1.0 + delta` (see the C comment for the geometry).
#[inline]
fn calculate_rr_antialiased(
    xp: i32,
    yp: i32,
    x: f32,
    y: f32,
    aspect_ratio: f32,
    sn: f32,
    cs: f32,
    one_over_radius2: f32,
    r_aa_start: f32,
) -> f32 {
    let pixel_right = x - xp as f32;
    let pixel_bottom = y - yp as f32;
    let pixel_center_x = pixel_right - 0.5;
    let pixel_center_y = pixel_bottom - 0.5;
    let pixel_left = pixel_right - 1.0;
    let pixel_top = pixel_bottom - 1.0;

    let (mut nearest_x, mut nearest_y) = (0.0f32, 0.0f32);
    let rr_near;
    // Dab's center is inside the pixel?
    if pixel_left < 0.0 && pixel_right > 0.0 && pixel_top < 0.0 && pixel_bottom > 0.0 {
        rr_near = 0.0;
    } else {
        let (nx, ny) = closest_point_to_line(cs, sn, pixel_center_x, pixel_center_y);
        nearest_x = clamp(nx, pixel_left, pixel_right);
        nearest_y = clamp(ny, pixel_top, pixel_bottom);
        rr_near = calculate_r_sample(nearest_x, nearest_y, aspect_ratio, sn, cs) * one_over_radius2;
    }

    // Out of dab's reach?
    if rr_near > 1.0 {
        return rr_near;
    }

    // Check on which side of the dab's line the pixel center is.
    let center_sign = sign_point_in_line(pixel_center_x, pixel_center_y, cs, -sn);

    // Radius of a circle with area = 1: r = sqrt(1/pi).
    // C: sqrtf(1.0f/M_PI) — 1.0f/M_PI promotes to double, then truncates to
    // float for the sqrtf argument.
    let rad_area_1 = ((1.0f64 / core::f64::consts::PI) as f32).sqrt();

    let (farthest_x, farthest_y) = if center_sign < 0.0 {
        (nearest_x - sn * rad_area_1, nearest_y + cs * rad_area_1)
    } else {
        (nearest_x + sn * rad_area_1, nearest_y - cs * rad_area_1)
    };

    let r_far = calculate_r_sample(farthest_x, farthest_y, aspect_ratio, sn, cs);
    let rr_far = r_far * one_over_radius2;

    // Skip heavier AA when we can.
    // C checks the RAW squared distance r_far (not rr_far) against
    // r_aa_start when deciding to skip the heavier AA.
    if r_far < r_aa_start {
        return (rr_far + rr_near) * 0.5;
    }

    let visibility_near = 1.0 - rr_near;
    let delta = rr_far - rr_near;
    let delta2 = 1.0 + delta;
    let visibility_near = visibility_near / delta2;
    1.0 - visibility_near
}

/// `calculate_opa` — two-segment hardness falloff over `rr`.
#[inline]
fn calculate_opa(
    rr: f32,
    hardness: f32,
    segment1_offset: f32,
    segment1_slope: f32,
    segment2_offset: f32,
    segment2_slope: f32,
) -> f32 {
    let fac = if rr <= hardness {
        segment1_slope
    } else {
        segment2_slope
    };
    let mut opa = if rr <= hardness {
        segment1_offset
    } else {
        segment2_offset
    };
    opa += rr * fac;
    if rr > 1.0 {
        opa = 0.0;
    }
    opa
}

/// Return one pixel from `render_dab_mask` without making the full mask.
/// This uses the same operation order as the full-mask path.
#[allow(clippy::too_many_arguments)]
pub fn render_dab_mask_pixel(
    x: f32,
    y: f32,
    radius: f32,
    hardness: f32,
    softness: f32,
    mut aspect_ratio: f32,
    angle: f32,
    xp: i32,
    yp: i32,
) -> u16 {
    let hardness = clamp(hardness, 0.0, 1.0);
    if aspect_ratio < 1.0 {
        aspect_ratio = 1.0;
    }
    debug_assert!(hardness != 0.0);

    let r_fringe = radius + 1.0;
    let x0 = (x - r_fringe).floor().max(0.0) as i32;
    let y0 = (y - r_fringe).floor().max(0.0) as i32;
    let x1 = ((x + r_fringe).floor() as i32).min(TILE_SIZE as i32 - 1);
    let y1 = ((y + r_fringe).floor() as i32).min(TILE_SIZE as i32 - 1);
    if xp < x0 || xp > x1 || yp < y0 || yp > y1 {
        return 0;
    }

    let segment1_offset = 1.0 * (1.0 - softness);
    let segment1_slope = -(1.0 / hardness - 1.0) * (1.0 - softness);
    let segment2_offset = hardness / (1.0 - hardness) * (1.0 - softness);
    let segment2_slope = -hardness / (1.0 - hardness) * (1.0 - softness);
    let angle_rad = (((angle / 360.0) * 2.0) as f64 * std::f64::consts::PI) as f32;
    let cs = (angle_rad as f64).cos() as f32;
    let sn = (angle_rad as f64).sin() as f32;
    let one_over_radius2 = 1.0 / (radius * radius);
    let rr = if radius < 3.0 {
        let aa_border = 1.0f32;
        let mut r_aa_start = if radius > aa_border {
            radius - aa_border
        } else {
            0.0
        };
        r_aa_start *= r_aa_start / aspect_ratio;
        calculate_rr_antialiased(
            xp,
            yp,
            x,
            y,
            aspect_ratio,
            sn,
            cs,
            one_over_radius2,
            r_aa_start,
        )
    } else {
        calculate_rr(xp, yp, x, y, aspect_ratio, sn, cs, one_over_radius2)
    };
    let opa = calculate_opa(
        rr,
        hardness,
        segment1_offset,
        segment1_slope,
        segment2_offset,
        segment2_slope,
    );
    (opa as f64 * (1u32 << 15) as f64) as u16
}

/// Scratch buffer sized as in the C (`TILE_SIZE*TILE_SIZE + 2*TILE_SIZE`
/// floats is enough for the precomputed rr values).

/// `render_dab_mask` — fills `mask` with the LRE-encoded dab opacity.
///
/// The output format: alternating runs of nonzero `u16` opacities and
/// `(0, skip*4)` skip markers, terminated by `(0, 0)`. `skip` is in pixels;
/// the stored count is `skip*4` because consumers advance RGBA by `u16`
/// quads.
pub fn render_dab_mask(
    mask: &mut [u16],
    x: f32,
    y: f32,
    radius: f32,
    hardness: f32,
    softness: f32,
    aspect_ratio: f32,
    angle: f32,
    scratch: &mut [f32],
) -> usize {
    render_dab_mask_rows(
        mask,
        x,
        y,
        radius,
        hardness,
        softness,
        aspect_ratio,
        angle,
        scratch,
        0,
        TILE_SIZE,
    )
}

/// Render only the specified tile rows. The result keeps absolute tile
/// pixel positions, so the existing blend functions can process it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_dab_mask_rows(
    mask: &mut [u16],
    x: f32,
    y: f32,
    radius: f32,
    hardness: f32,
    softness: f32,
    aspect_ratio: f32,
    angle: f32,
    scratch: &mut [f32],
    row_start: usize,
    row_end: usize,
) -> usize {
    let hardness = clamp(hardness, 0.0, 1.0);
    let mut aspect_ratio = aspect_ratio;
    if aspect_ratio < 1.0 {
        aspect_ratio = 1.0;
    }
    debug_assert!(hardness != 0.0); // assured by the caller

    let segment1_offset = 1.0 * (1.0 - softness);
    let segment1_slope = -(1.0 / hardness - 1.0) * (1.0 - softness);
    let segment2_offset = hardness / (1.0 - hardness) * (1.0 - softness);
    let segment2_slope = -hardness / (1.0 - hardness) * (1.0 - softness);
    // For hardness == 1.0, segment2 is never used.

    // C: `float angle_rad = angle/360*2*M_PI;` — angle/360*2 stays float,
    // the multiply by double M_PI promotes the product to double, and
    // cos/sin run in double before the float cast.
    let angle_rad = (((angle / 360.0) * 2.0) as f64 * std::f64::consts::PI) as f32;
    let cs = (angle_rad as f64).cos() as f32;
    let sn = (angle_rad as f64).sin() as f32;

    let r_fringe = radius + 1.0; // +1.0 should not be required, only to be sure
    let mut x0 = (x - r_fringe).floor() as i32;
    let mut y0 = (y - r_fringe).floor() as i32;
    let mut x1 = (x + r_fringe).floor() as i32;
    let mut y1 = (y + r_fringe).floor() as i32;
    if x0 < 0 {
        x0 = 0;
    }
    if y0 < 0 {
        y0 = 0;
    }
    if x1 > TILE_SIZE as i32 - 1 {
        x1 = TILE_SIZE as i32 - 1;
    }
    if y1 > TILE_SIZE as i32 - 1 {
        y1 = TILE_SIZE as i32 - 1;
    }
    y0 = y0.max(row_start.min(TILE_SIZE) as i32);
    y1 = y1.min(row_end.min(TILE_SIZE) as i32 - 1);
    if x0 > x1 || y0 > y1 {
        mask[0] = 0;
        mask[1] = 0;
        return 2;
    }
    let one_over_radius2 = 1.0 / (radius * radius);

    // the caller owns a fixed-capacity scratch sized MASK_LEN

    if radius < 3.0 {
        let aa_border = 1.0f32;
        let mut r_aa_start = if radius > aa_border {
            radius - aa_border
        } else {
            0.0
        };
        r_aa_start *= r_aa_start / aspect_ratio;

        for yp in y0..=y1 {
            for xp in x0..=x1 {
                let rr = calculate_rr_antialiased(
                    xp,
                    yp,
                    x,
                    y,
                    aspect_ratio,
                    sn,
                    cs,
                    one_over_radius2,
                    r_aa_start,
                );
                scratch[(yp as usize) * TILE_SIZE + xp as usize] = rr;
            }
        }
    } else {
        for yp in y0..=y1 {
            for xp in x0..=x1 {
                let rr = calculate_rr(xp, yp, x, y, aspect_ratio, sn, cs, one_over_radius2);
                scratch[(yp as usize) * TILE_SIZE + xp as usize] = rr;
            }
        }
    }

    // Run-length encoding: if opacity is zero, the next mask value is the
    // number of pixels that can be skipped. Fixed-capacity slice, written
    // through an index cursor (the caller owns a MASK_LEN buffer).
    let mut mp = 0usize;
    let mut skip: i32 = 0;

    skip += y0 * TILE_SIZE as i32;
    for yp in y0..=y1 {
        skip += x0;

        let mut xp = x0;
        while xp <= x1 {
            let rr = scratch[(yp as usize) * TILE_SIZE + xp as usize];
            let opa = calculate_opa(
                rr,
                hardness,
                segment1_offset,
                segment1_slope,
                segment2_offset,
                segment2_slope,
            );
            // C: `const uint16_t opa_ = opa * (1<<15);` — (1<<15) promotes
            // the product to double; truncate that double to u16.
            let opa_ = (opa as f64 * (1u32 << 15) as f64) as u16;
            if opa_ == 0 {
                skip += 1;
            } else {
                if skip != 0 {
                    mask[mp] = 0;
                    mask[mp + 1] = (skip * 4) as u16;
                    mp += 2;
                    skip = 0;
                }
                mask[mp] = opa_;
                mp += 1;
            }
            xp += 1;
        }
        skip += TILE_SIZE as i32 - xp;
    }
    mask[mp] = 0;
    mask[mp + 1] = 0;
    mp + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_pixel_path(
        x: f32,
        y: f32,
        radius: f32,
        hardness: f32,
        softness: f32,
        aspect_ratio: f32,
        angle: f32,
    ) {
        const MASK_LEN: usize = TILE_SIZE * TILE_SIZE + 2 * TILE_SIZE;
        let mut mask = [0u16; MASK_LEN];
        let mut scratch = [0.0f32; MASK_LEN];
        render_dab_mask(
            &mut mask,
            x,
            y,
            radius,
            hardness,
            softness,
            aspect_ratio,
            angle,
            &mut scratch,
        );
        let mut decoded = [0u16; TILE_SIZE * TILE_SIZE];
        let mut mi = 0usize;
        let mut pi = 0usize;
        loop {
            while mask[mi] != 0 {
                decoded[pi] = mask[mi];
                mi += 1;
                pi += 1;
            }
            if mask[mi + 1] == 0 {
                break;
            }
            pi += mask[mi + 1] as usize / 4;
            mi += 2;
        }
        for (pi, expected) in decoded.into_iter().enumerate() {
            assert_eq!(
                render_dab_mask_pixel(
                    x,
                    y,
                    radius,
                    hardness,
                    softness,
                    aspect_ratio,
                    angle,
                    (pi % TILE_SIZE) as i32,
                    (pi / TILE_SIZE) as i32,
                ),
                expected,
                "pixel {pi}",
            );
        }
    }

    #[test]
    fn pixel_path_matches_full_mask() {
        check_pixel_path(31.25, 29.75, 1.5, 0.4, 0.2, 1.0, 0.0);
        check_pixel_path(4.25, 63.5, 18.0, 0.6, 0.1, 2.5, 37.0);
        check_pixel_path(-5.0, 10.0, 12.0, 1.0, 0.4, 1.0, 91.0);
    }
}
