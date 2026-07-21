//! Resident (non-virtual) texture cooking: 8-bit displacement quantization and
//! blue-noise tile generation for POM ray-start dithering.

use crate::TextureFormat;

/// Deterministically quantize a 16-bit luma source to 8-bit R8 samples.
///
/// Each normalized `u16` (0..=65535) maps to `u8` via `(sample + 128) / 257`,
/// the standard round-to-nearest 16→8 mapping. This is a deliberate, exact
/// cook-time quantization — not the silent browser PNG truncation that
/// `docs/api/pom.md` forbids.
pub fn quantize_luma16_to_r8(pixels: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len());
    for &sample in pixels {
        // (sample + 128) / 257 rounds 0..=65535 onto 0..=255 with <=0.5 lsb error.
        out.push(((sample as u32 + 128) / 257) as u8);
    }
    out
}

/// Load a single-channel displacement source and quantize it to R8 bytes.
///
/// Accepts:
/// - an `AGR16LE` payload (`.r16`) produced by `height-r16` — decoded
///   losslessly and quantized 16->8, reusing the exact already-downsampled
///   source (no 8K reprocessing).
/// - an 8-bit or 16-bit grayscale PNG — 16-bit sources are decoded via
///   `into_luma16` so no precision is lost before the deliberate 8-bit step.
pub fn load_displacement_r8(path: &std::path::Path) -> Result<(u32, u32, Vec<u8>), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    if bytes.len() >= 8 && &bytes[..8] == crate::HEIGHT_R16_MAGIC {
        let (width, height, pixels) = crate::decode_height_r16(&bytes)?;
        return Ok((width, height, quantize_luma16_to_r8(&pixels)));
    }
    let image = image::open(path)
        .map_err(|e| format!("failed to decode {}: {e}", path.display()))?;
    let luma = image.into_luma16();
    let (width, height) = luma.dimensions();
    Ok((width, height, quantize_luma16_to_r8(luma.as_raw())))
}

/// Generate an `size × size` blue-noise tile as R8 bytes in [0, 255].
///
/// Uses the void-and-cluster algorithm (Ulichney 1993): seed a uniform initial
/// binary pattern, then iteratively swap the most-clustered "on" pixel with the
/// least-clustered "off" pixel until convergence. The result is a low-discrepancy
/// binary mask; thresholding it at the requested fill ratio yields a blue-noise
/// dither value per cell. Here we emit the rank-ordered dither array (0..255),
/// the standard form for POM ray-start jitter.
pub fn generate_blue_noise_tile(size: u32) -> Vec<u8> {
    assert!(size > 0 && size.is_power_of_two(), "blue-noise size must be a power of two");
    let n = size as usize;
    let count = n * n;

    // Initial binary pattern: deterministic pseudo-random sprinkle (~10% density).
    // A fixed LCG makes the cook reproducible across machines.
    let mut state: u32 = 0x9E3779B9;
    let mut rng = || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        state
    };
    let initial_on = (count / 10).max(1);
    let mut pattern = vec![false; count];
    let mut filled = 0;
    while filled < initial_on {
        let idx = (rng() as usize) % count;
        if !pattern[idx] {
            pattern[idx] = true;
            filled += 1;
        }
    }

    // Gaussian energy contribution from an "on" pixel at toroidal distance d.
    // Sigma ~1.5 cells; evaluated on a torus (wrap) so the tile is tileable.
    let sigma: f32 = 1.5;
    let two_sigma_sq = 2.0 * sigma * sigma;
    let radius = (3.0 * sigma).ceil() as i32;
    let kernel_half = radius.min(n as i32 - 1).max(1);
    // 1D toroidal gaussian: distance wraps around the tile.
    let gauss_1d = |delta: i32| -> f32 {
        let raw = delta.abs().min(n as i32 - delta.abs()) as f32;
        (-(raw * raw) / two_sigma_sq).exp()
    };

    // Energy at each cell = sum of gaussian contributions from all on-pixels (toroidal).
    let mut energy = vec![0.0f32; count];
    let add_energy = |energy: &mut [f32], pattern: &[bool], sign: f32| {
        for y in 0..n {
            for x in 0..n {
                if !pattern[y * n + x] {
                    continue;
                }
                for dy in -kernel_half..=kernel_half {
                    let gy = (y as i32 + dy).rem_euclid(n as i32) as usize;
                    let gy_dist = dy;
                    for dx in -kernel_half..=kernel_half {
                        let gx = (x as i32 + dx).rem_euclid(n as i32) as usize;
                        energy[gy * n + gx] += sign * gauss_1d(dx) * gauss_1d(gy_dist);
                    }
                }
            }
        }
    };
    add_energy(&mut energy, &pattern, 1.0);

    // Void-and-cluster: repeatedly find the most clustered "on" and least
    // clustered "off", swap them, until no improvement. Then assign rank order.
    for _iteration in 0..(count * 4) {
        // Most clustered ON: highest energy among on-pixels.
        let mut worst_on = usize::MAX;
        let mut worst_on_e = f32::NEG_INFINITY;
        for (i, &on) in pattern.iter().enumerate() {
            if on && energy[i] > worst_on_e {
                worst_on_e = energy[i];
                worst_on = i;
            }
        }
        // Least clustered OFF: lowest energy among off-pixels.
        let mut best_off = usize::MAX;
        let mut best_off_e = f32::INFINITY;
        for (i, &on) in pattern.iter().enumerate() {
            if !on && energy[i] < best_off_e {
                best_off_e = energy[i];
                best_off = i;
            }
        }
        if worst_on == usize::MAX || best_off == usize::MAX {
            break;
        }
        // Swap improves total clustering only if removing the worst on and adding
        // at the best off reduces the max energy. Use a simple accept rule.
        if worst_on_e - best_off_e < 1e-6 {
            break;
        }
        // Apply swap: remove energy from worst_on, add at best_off.
        let (wy, wx) = (worst_on / n, worst_on % n);
        let (by, bx) = (best_off / n, best_off % n);
        for dy in -kernel_half..=kernel_half {
            let gy_on = (wy as i32 + dy).rem_euclid(n as i32) as usize;
            let gy_off = (by as i32 + dy).rem_euclid(n as i32) as usize;
            for dx in -kernel_half..=kernel_half {
                let gx_on = (wx as i32 + dx).rem_euclid(n as i32) as usize;
                let gx_off = (bx as i32 + dx).rem_euclid(n as i32) as usize;
                let g = gauss_1d(dx) * gauss_1d(dy);
                energy[gy_on * n + gx_on] -= g;
                energy[gy_off * n + gx_off] += g;
            }
        }
        pattern[worst_on] = false;
        pattern[best_off] = true;
    }

    // Rank-order the cells: assign dither value by successive "turn off the most
    // clustered on-pixel" passes. The first removed cell gets the highest value.
    let mut ranks = vec![0u32; count];
    let mut current_rank = (count - 1) as u32;
    // Recompute energy from the converged pattern.
    let mut e = vec![0.0f32; count];
    add_energy(&mut e, &pattern, 1.0);
    let mut p = pattern.clone();
    let mut on_count = p.iter().filter(|&&b| b).count();
    // Remove from the clustered end.
    while on_count > 0 {
        let mut worst = usize::MAX;
        let mut worst_e = f32::NEG_INFINITY;
        for (i, &on) in p.iter().enumerate() {
            if on && e[i] > worst_e {
                worst_e = e[i];
                worst = i;
            }
        }
        if worst == usize::MAX {
            break;
        }
        ranks[worst] = current_rank;
        current_rank = current_rank.saturating_sub(1);
        let (wy, wx) = (worst / n, worst % n);
        for dy in -kernel_half..=kernel_half {
            let gy = (wy as i32 + dy).rem_euclid(n as i32) as usize;
            for dx in -kernel_half..=kernel_half {
                let gx = (wx as i32 + dx).rem_euclid(n as i32) as usize;
                e[gy * n + gx] -= gauss_1d(dx) * gauss_1d(dy);
            }
        }
        p[worst] = false;
        on_count -= 1;
    }
    // Now fill from the void end: add at the least clustered off-pixel.
    let mut e2 = vec![0.0f32; count];
    while current_rank > 0 {
        let mut best = usize::MAX;
        let mut best_e = f32::INFINITY;
        for (i, &on) in p.iter().enumerate() {
            if !on && e2[i] < best_e {
                best_e = e2[i];
                best = i;
            }
        }
        if best == usize::MAX {
            break;
        }
        ranks[best] = current_rank;
        current_rank = current_rank.saturating_sub(1);
        let (by, bx) = (best / n, best % n);
        for dy in -kernel_half..=kernel_half {
            let gy = (by as i32 + dy).rem_euclid(n as i32) as usize;
            for dx in -kernel_half..=kernel_half {
                let gx = (bx as i32 + dx).rem_euclid(n as i32) as usize;
                e2[gy * n + gx] += gauss_1d(dx) * gauss_1d(dy);
            }
        }
        p[best] = true;
    }

    // Scale ranks 0..(count-1) to 0..255.
    let max_rank = (count - 1) as u32;
    ranks
        .iter()
        .map(|&r| ((r as u64 * 255 + max_rank as u64 / 2) / max_rank as u64) as u8)
        .collect()
}

/// Pack a generated blue-noise tile into a resident R8 texture payload.
pub fn blue_noise_resident_payload(size: u32) -> (u32, u32, TextureFormat, Vec<u8>) {
    let bytes = generate_blue_noise_tile(size);
    (size, size, TextureFormat::R8, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_maps_extremes_round_nearest() {
        let out = quantize_luma16_to_r8(&[0, 128, 255, 256, 32_768, 65_279, 65_280, 65_535]);
        // (sample + 128) / 257: round-to-nearest 16->8.
        // 0->0, 128->0, 255->1, 256->1, 32768->128,
        // 65279->254, 65280->254, 65535->255.
        assert_eq!(out, vec![0, 0, 1, 1, 128, 254, 254, 255]);
    }

    #[test]
    fn blue_noise_tile_is_correct_size_and_range() {
        let (w, h, fmt, bytes) = blue_noise_resident_payload(16);
        assert_eq!((w, h), (16, 16));
        assert_eq!(fmt, TextureFormat::R8);
        assert_eq!(bytes.len(), 256);
        // Every value in [0,255], and the tile contains many distinct values
        // (a degenerate all-equal tile would indicate a broken generator).
        let distinct = std::collections::HashSet::<u8>::from_iter(bytes.iter().copied()).len();
        assert!(distinct > 64, "blue-noise tile only had {distinct} distinct values");
        assert!(bytes.iter().all(|&v| v <= 255u8));
    }

    #[test]
    fn blue_noise_is_deterministic() {
        let a = generate_blue_noise_tile(16);
        let b = generate_blue_noise_tile(16);
        assert_eq!(a, b, "blue-noise generation must be reproducible");
    }
}
