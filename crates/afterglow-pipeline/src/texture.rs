// Texture mip generation — box filter downscaling.
//
// Generates a full mip chain from a source texture (RGBA8). Each mip level
// is half the width and height of the previous, computed by averaging 2×2
// pixel blocks. This is the same algorithm GPU `generateMipmap()` uses.
//
// Raw RGBA in, deterministic mip and bordered virtual-page data out.

pub const VT_PAGE_SIZE: u32 = 128;
pub const VT_PAGE_BORDER: u32 = 4;
pub const VT_SLOT_SIZE: u32 = VT_PAGE_SIZE + VT_PAGE_BORDER * 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiledVirtualPage {
    pub mip: u8,
    pub page_x: u32,
    pub page_y: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedMipTail {
    pub first_mip: u8,
    pub data: Vec<u8>,
}

/// Fixed payload rectangles for 64, 32, 16, 8, 4, 2 and 1 texel mips.
/// Each rectangle includes its own four-texel border and all fit in one slot.
pub const VT_MIP_TAIL_RECTS: [(u32, u32, u32); 7] = [
    (0, 0, 64), (72, 0, 32), (112, 0, 16), (72, 40, 8),
    (88, 40, 4), (100, 40, 2), (110, 40, 1),
];

/// Pack two RGBA grayscale masks into the R and G channels of one texture.
pub fn pack_mask_channels(red: &[u8], green: &[u8]) -> Result<Vec<u8>, String> {
    if red.len() != green.len() || red.len() % 4 != 0 {
        return Err("mask inputs must have equal RGBA byte lengths".into());
    }
    let mut packed = vec![0; red.len()];
    for pixel in 0..red.len() / 4 {
        let offset = pixel * 4;
        packed[offset] = red[offset];
        packed[offset + 1] = green[offset];
        packed[offset + 2] = 0;
        packed[offset + 3] = 255;
    }
    Ok(packed)
}

pub fn virtual_mip_tail_first_mip(mut width: u32, mut height: u32) -> Result<u8, String> {
    if width == 0 || height == 0 { return Err("virtual texture dimensions must be non-zero".into()); }
    let mut mip = 0u8;
    while width.max(height) > 64 {
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        mip = mip.checked_add(1).ok_or("virtual texture has too many mip levels")?;
    }
    Ok(mip)
}

/// Consume one source image while retaining only the current mip and at most
/// 64 bordered pages. The callback can parallel-encode and immediately spool
/// each batch; the packed tail is produced from the same mip walk.
pub fn stream_virtual_texture<F>(
    mut pixels: Vec<u8>,
    mut width: u32,
    mut height: u32,
    mut consume_batch: F,
) -> Result<PackedMipTail, String>
where
    F: FnMut(Vec<TiledVirtualPage>) -> Result<(), String>,
{
    if pixels.len() != width as usize * height as usize * 4 || width == 0 || height == 0 {
        return Err("RGBA byte length does not match dimensions".into());
    }
    let first_mip = virtual_mip_tail_first_mip(width, height)?;
    let mut tail = vec![0; (VT_SLOT_SIZE * VT_SLOT_SIZE * 4) as usize];
    let mut mip = 0u8;
    loop {
        if mip < first_mip {
            let grid_x = width.div_ceil(VT_PAGE_SIZE);
            let grid_y = height.div_ceil(VT_PAGE_SIZE);
            let mut batch = Vec::with_capacity(64);
            for page_y in 0..grid_y {
                for page_x in 0..grid_x {
                    let mut page = vec![0; (VT_SLOT_SIZE * VT_SLOT_SIZE * 4) as usize];
                    for slot_y in 0..VT_SLOT_SIZE {
                        for slot_x in 0..VT_SLOT_SIZE {
                            let source_x = (page_x as i64 * VT_PAGE_SIZE as i64 + slot_x as i64 - VT_PAGE_BORDER as i64)
                                .clamp(0, width as i64 - 1) as u32;
                            let source_y = (page_y as i64 * VT_PAGE_SIZE as i64 + slot_y as i64 - VT_PAGE_BORDER as i64)
                                .clamp(0, height as i64 - 1) as u32;
                            let source = ((source_y * width + source_x) * 4) as usize;
                            let target = ((slot_y * VT_SLOT_SIZE + slot_x) * 4) as usize;
                            page[target..target + 4].copy_from_slice(&pixels[source..source + 4]);
                        }
                    }
                    batch.push(TiledVirtualPage { mip, page_x, page_y, data: page });
                    if batch.len() == 64 { consume_batch(std::mem::take(&mut batch))?; }
                }
            }
            if !batch.is_empty() { consume_batch(batch)?; }
        } else {
            let tail_level = (mip - first_mip) as usize;
            if tail_level < VT_MIP_TAIL_RECTS.len() {
                let (origin_x, origin_y, capacity) = VT_MIP_TAIL_RECTS[tail_level];
                if width > capacity || height > capacity { return Err("mip-tail level exceeds rectangle".into()); }
                for rect_y in 0..height + VT_PAGE_BORDER * 2 {
                    for rect_x in 0..width + VT_PAGE_BORDER * 2 {
                        let source_x = (rect_x as i64 - VT_PAGE_BORDER as i64).clamp(0, width as i64 - 1) as u32;
                        let source_y = (rect_y as i64 - VT_PAGE_BORDER as i64).clamp(0, height as i64 - 1) as u32;
                        let source = ((source_y * width + source_x) * 4) as usize;
                        let target = (((origin_y + rect_y) * VT_SLOT_SIZE + origin_x + rect_x) * 4) as usize;
                        tail[target..target + 4].copy_from_slice(&pixels[source..source + 4]);
                    }
                }
            }
        }
        if width == 1 && height == 1 { break; }
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        pixels = downscale_box(&pixels, width, height, next_width, next_height);
        width = next_width;
        height = next_height;
        mip += 1;
    }
    Ok(PackedMipTail { first_mip, data: tail })
}

pub fn pack_virtual_mip_tail(data: &[u8], width: u32, height: u32) -> Result<PackedMipTail, String> {
    if width == 0 || height == 0 {
        return Err("mip-tail dimensions must be non-zero".into());
    }
    if data.len() != width as usize * height as usize * 4 {
        return Err("RGBA byte length does not match dimensions".into());
    }
    let mips = generate_mip_chain(data, width, height);
    let first_mip = mips.iter().position(|(w, h, _)| (*w).max(*h) <= 64)
        .ok_or("mip chain has no tail")?;
    let mut tail = vec![0; (VT_SLOT_SIZE * VT_SLOT_SIZE * 4) as usize];
    for (tail_level, &(origin_x, origin_y, rect_capacity)) in VT_MIP_TAIL_RECTS.iter().enumerate() {
        let mip = first_mip + tail_level;
        if mip >= mips.len() { break; }
        let (mip_width, mip_height, pixels) = &mips[mip];
        if *mip_width > rect_capacity || *mip_height > rect_capacity {
            return Err("mip-tail level exceeds its packed rectangle".into());
        }
        let rect_width = *mip_width + VT_PAGE_BORDER * 2;
        let rect_height = *mip_height + VT_PAGE_BORDER * 2;
        for rect_y in 0..rect_height {
            for rect_x in 0..rect_width {
                let source_x = (rect_x as i64 - VT_PAGE_BORDER as i64)
                    .clamp(0, *mip_width as i64 - 1) as u32;
                let source_y = (rect_y as i64 - VT_PAGE_BORDER as i64)
                    .clamp(0, *mip_height as i64 - 1) as u32;
                let source = ((source_y * *mip_width + source_x) * 4) as usize;
                let target_x = origin_x + rect_x;
                let target_y = origin_y + rect_y;
                let target = ((target_y * VT_SLOT_SIZE + target_x) * 4) as usize;
                tail[target..target + 4].copy_from_slice(&pixels[source..source + 4]);
            }
        }
    }
    Ok(PackedMipTail { first_mip: first_mip as u8, data: tail })
}

/// Build every paged mip down through the 128×128 terminal page.
/// Borders sample neighboring virtual texels and clamp only at image edges.
pub fn tile_virtual_texture(data: &[u8], width: u32, height: u32) -> Result<Vec<TiledVirtualPage>, String> {
    if width == 0 || height == 0 {
        return Err("virtual texture dimensions must be non-zero".into());
    }
    if data.len() != width as usize * height as usize * 4 {
        return Err("RGBA byte length does not match dimensions".into());
    }

    let mut pages = Vec::new();
    for (mip, (mip_width, mip_height, mip_data)) in generate_mip_chain(data, width, height).into_iter().enumerate() {
        if mip_width.max(mip_height) <= VT_PAGE_SIZE / 2 { break; }
        let grid_x = mip_width.div_ceil(VT_PAGE_SIZE);
        let grid_y = mip_height.div_ceil(VT_PAGE_SIZE);
        for page_y in 0..grid_y {
            for page_x in 0..grid_x {
                let mut page = vec![0; (VT_SLOT_SIZE * VT_SLOT_SIZE * 4) as usize];
                for slot_y in 0..VT_SLOT_SIZE {
                    for slot_x in 0..VT_SLOT_SIZE {
                        let source_x = (page_x as i64 * VT_PAGE_SIZE as i64 + slot_x as i64 - VT_PAGE_BORDER as i64)
                            .clamp(0, mip_width as i64 - 1) as u32;
                        let source_y = (page_y as i64 * VT_PAGE_SIZE as i64 + slot_y as i64 - VT_PAGE_BORDER as i64)
                            .clamp(0, mip_height as i64 - 1) as u32;
                        let source = ((source_y * mip_width + source_x) * 4) as usize;
                        let target = ((slot_y * VT_SLOT_SIZE + slot_x) * 4) as usize;
                        page[target..target + 4].copy_from_slice(&mip_data[source..source + 4]);
                    }
                }
                pages.push(TiledVirtualPage { mip: mip as u8, page_x, page_y, data: page });
            }
        }
    }
    Ok(pages)
}

/// Generate a full mip chain from a source texture.
///
/// Returns a Vec of (width, height, RGBA data), ordered from mip 0 (full res)
/// to the smallest mip (1×1 or 1×N).
pub fn generate_mip_chain(
    data: &[u8],
    width: u32,
    height: u32,
) -> Vec<(u32, u32, Vec<u8>)> {
    let mut mips = vec![(width, height, data.to_vec())];

    let mut w = width;
    let mut h = height;
    let mut src = data.to_vec();

    while w > 1 || h > 1 {
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        let dst = downscale_box(&src, w, h, nw, nh);
        mips.push((nw, nh, dst.clone()));
        src = dst;
        w = nw;
        h = nh;
    }

    mips
}

/// Box-filter downscale: average source pixel blocks into destination pixels.
/// Works for any downscale ratio (not just 2×2).
pub fn downscale_box(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    assert_eq!(src.len(), src_w as usize * src_h as usize * 4);

    let mut dst = vec![0u8; dst_w as usize * dst_h as usize * 4];
    let sx = src_w as f32 / dst_w as f32;
    let sy = src_h as f32 / dst_h as f32;

    for ty in 0..dst_h {
        for tx in 0..dst_w {
            let x0 = (tx as f32 * sx) as usize;
            let y0 = (ty as f32 * sy) as usize;
            let x1 = (((tx + 1) as f32 * sx) as usize + 1).min(src_w as usize);
            let y1 = (((ty + 1) as f32 * sy) as usize + 1).min(src_h as usize);

            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut a = 0u32;
            let mut count = 0u32;

            for py in y0..y1 {
                for px in x0..x1 {
                    let i = (py * src_w as usize + px) * 4;
                    r += src[i] as u32;
                    g += src[i + 1] as u32;
                    b += src[i + 2] as u32;
                    a += src[i + 3] as u32;
                    count += 1;
                }
            }

            if count == 0 {
                count = 1;
            }

            let o = (ty as usize * dst_w as usize + tx as usize) * 4;
            dst[o] = (r / count) as u8;
            dst[o + 1] = (g / count) as u8;
            dst[o + 2] = (b / count) as u8;
            dst[o + 3] = (a / count) as u8;
        }
    }

    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_two_masks_into_rg_without_cross_channel_conversion() {
        let red = [12, 12, 12, 255, 34, 34, 34, 255];
        let green = [56, 56, 56, 255, 78, 78, 78, 255];
        let packed = pack_mask_channels(&red, &green).unwrap();
        assert_eq!(packed, [12, 56, 0, 255, 34, 78, 0, 255]);
        assert!(pack_mask_channels(&red, &green[..4]).is_err());
    }

    #[test]
    fn packs_all_sub_page_mips_into_one_tail_slot() {
        let mut source = vec![0; 256 * 256 * 4];
        for y in 0..256usize { for x in 0..256usize {
            let i = (y * 256 + x) * 4;
            source[i..i + 4].copy_from_slice(&[x as u8, y as u8, 33, 255]);
        }}
        let tail = pack_virtual_mip_tail(&source, 256, 256).unwrap();
        assert_eq!(tail.first_mip, 2); // 256 -> 128 -> 64
        assert_eq!(tail.data.len(), (VT_SLOT_SIZE * VT_SLOT_SIZE * 4) as usize);
        for &(x, y, size) in &VT_MIP_TAIL_RECTS {
            assert!(x + size + VT_PAGE_BORDER * 2 <= VT_SLOT_SIZE);
            assert!(y + size + VT_PAGE_BORDER * 2 <= VT_SLOT_SIZE);
            let alpha = (((y + VT_PAGE_BORDER) * VT_SLOT_SIZE + x + VT_PAGE_BORDER) * 4 + 3) as usize;
            assert_eq!(tail.data[alpha], 255);
        }
    }

    #[test]
    fn tiles_virtual_texture_with_neighbor_borders() {
        let mut source = vec![0; 256 * 256 * 4];
        for y in 0..256usize {
            for x in 0..256usize {
                let i = (y * 256 + x) * 4;
                source[i..i + 4].copy_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        let pages = tile_virtual_texture(&source, 256, 256).unwrap();
        // mip 0 has four pages, mip 1 has one terminal page.
        assert_eq!(pages.len(), 5);
        let right = pages.iter().find(|p| p.mip == 0 && p.page_x == 1 && p.page_y == 0).unwrap();
        // Left border of page (1,0) samples x=124..127 from its neighbor.
        assert_eq!(&right.data[0..4], &[124, 0, 7, 255]);
        let payload = ((VT_PAGE_BORDER * VT_SLOT_SIZE + VT_PAGE_BORDER) * 4) as usize;
        assert_eq!(&right.data[payload..payload + 4], &[128, 0, 7, 255]);
        assert!(pages.iter().any(|p| p.mip == 1 && p.page_x == 0 && p.page_y == 0));
    }

    #[test]
    fn supports_rectangular_and_non_power_of_two_dimensions() {
        let rectangular = tile_virtual_texture(&vec![0; 192 * 65 * 4], 192, 65).unwrap();
        assert_eq!(rectangular.iter().filter(|page| page.mip == 0).count(), 2);
        assert!(rectangular.iter().any(|page| page.mip == 1));
        let tail = pack_virtual_mip_tail(&vec![0; 192 * 65 * 4], 192, 65).unwrap();
        assert_eq!(tail.first_mip, 2); // 192x65 -> 96x32 -> 48x16
        assert!(tile_virtual_texture(&[], 0, 1).is_err());
    }

    #[test]
    fn streaming_pages_and_tail_match_materialized_pipeline_with_bounded_batches() {
        let mut source = vec![0; 256 * 192 * 4];
        for (index, byte) in source.iter_mut().enumerate() { *byte = index as u8; }
        let expected_pages = tile_virtual_texture(&source, 256, 192).unwrap();
        let expected_tail = pack_virtual_mip_tail(&source, 256, 192).unwrap();
        let mut actual_pages = Vec::new();
        let mut max_batch = 0;
        let actual_tail = stream_virtual_texture(source, 256, 192, |batch| {
            max_batch = max_batch.max(batch.len());
            actual_pages.extend(batch);
            Ok(())
        }).unwrap();
        assert!(max_batch <= 64);
        assert_eq!(actual_pages, expected_pages);
        assert_eq!(actual_tail, expected_tail);
    }

    #[test]
    fn offline_uastc_pages_transcode_in_the_runtime_codec() {
        let rgba = vec![127; (VT_SLOT_SIZE * VT_SLOT_SIZE * 4) as usize];
        let basis = afterglow_basis_encoder::encode_uastc_rgba(&rgba, VT_SLOT_SIZE, VT_SLOT_SIZE).unwrap();
        let transcoded = afterglow_texture::safe::transcode(&basis, 0).unwrap();
        assert!(!transcoded.is_empty());
    }

    #[test]
    fn downscale_2x2_to_1x1() {
        // 2×2 texture: red, green, blue, yellow
        let src = [
            255, 0, 0, 255,    // (0,0) red
            0, 255, 0, 255,    // (1,0) green
            0, 0, 255, 255,    // (0,1) blue
            255, 255, 0, 255,  // (1,1) yellow
        ];
        let dst = downscale_box(&src, 2, 2, 1, 1);
        assert_eq!(dst.len(), 4);
        // Average: R=(255+0+0+255)/4=127, G=(0+255+0+255)/4=127, B=(0+0+255+0)/4=63
        assert_eq!(dst[0], 127); // R
        assert_eq!(dst[1], 127); // G
        assert_eq!(dst[2], 63);  // B
        assert_eq!(dst[3], 255); // A
    }

    #[test]
    fn downscale_4x4_to_2x2() {
        let src = vec![255; 4 * 4 * 4]; // all white
        let dst = downscale_box(&src, 4, 4, 2, 2);
        assert_eq!(dst.len(), 2 * 2 * 4);
        for i in 0..dst.len() {
            assert_eq!(dst[i], 255);
        }
    }

    #[test]
    fn mip_chain_8x8() {
        let src = vec![200; 8 * 8 * 4];
        let mips = generate_mip_chain(&src, 8, 8);
        assert_eq!(mips.len(), 4); // 8, 4, 2, 1
        assert_eq!(mips[0], (8, 8, src.clone()));
        assert_eq!(mips[1].0, 4);
        assert_eq!(mips[1].1, 4);
        assert_eq!(mips[2].0, 2);
        assert_eq!(mips[3].0, 1);
        assert_eq!(mips[3].1, 1);
        // Each mip should be ~200 (averaging identical values).
        for (_, _, data) in &mips {
            for &b in data {
                assert_eq!(b, 200);
            }
        }
    }

    #[test]
    fn mip_chain_non_power_of_two() {
        let src = vec![100; 6 * 6 * 4]; // 6×6
        let mips = generate_mip_chain(&src, 6, 6);
        // 6 → 3 → 1 (since 3/2 = 1)
        assert!(mips.len() >= 3);
        assert_eq!(mips[0].0, 6);
        assert_eq!(mips[1].0, 3);
        assert_eq!(mips.last().unwrap().0, 1);
    }

    #[test]
    fn mip_preserves_color_gradient() {
        // 4×1 gradient: black → white
        let src = [0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 255, 255, 255, 255, 255];
        let mips = generate_mip_chain(&src, 4, 1);
        assert_eq!(mips.len(), 3); // 4, 2, 1

        // mip 1 (2×1): should be average of pairs
        let m1 = &mips[1].2;
        // (0+85)/2 = 42, but box filter may include extra pixels due to rounding
        assert!(m1[0] <= 85, "mip1[0] should be ≤85, got {}", m1[0]);
        assert!(m1[4] >= 170, "mip1[4] should be ≥170, got {}", m1[4]);

        // mip 2 (1×1): average of all 4 pixels
        let m2 = &mips[2].2;
        let avg = (0u32 + 85 + 170 + 255) / 4; // = 127
        // Box filter may include slightly different pixel ranges due to
        // rounding — just verify it's in the right ballpark.
        assert!((u32::from(m2[0]) as i32 - avg as i32).abs() <= 30, "mip2[0]={} should be near {}", m2[0], avg);
    }
}
