//! Lossless single-channel 16-bit displacement interchange for runtime upload.

/// `AGR16LE`, followed by format version 1.
pub const HEIGHT_R16_MAGIC: [u8; 8] = *b"AGR16LE\x01";
pub const HEIGHT_R16_HEADER_BYTES: usize = 16;

/// Convert a decoded source image without reducing its 16-bit luma precision.
pub fn encode_height_r16_image(image: image::DynamicImage) -> Result<(u32, u32, Vec<u8>), String> {
    let image = image.into_luma16();
    let (width, height) = image.dimensions();
    let encoded = encode_height_r16(width, height, image.as_raw())?;
    Ok((width, height, encoded))
}

/// Encode normalized `u16` height samples as a versioned little-endian payload.
pub fn encode_height_r16(width: u32, height: u32, pixels: &[u16]) -> Result<Vec<u8>, String> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "R16 height dimensions overflow".to_string())?;
    if width == 0 || height == 0 {
        return Err("R16 height dimensions must be non-zero".into());
    }
    if pixels.len() != count {
        return Err(format!(
            "R16 height sample count mismatch: expected {count}, got {}",
            pixels.len()
        ));
    }
    let payload_bytes = count
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(HEIGHT_R16_HEADER_BYTES))
        .ok_or_else(|| "R16 height payload size overflow".to_string())?;
    let mut output = Vec::with_capacity(payload_bytes);
    output.extend_from_slice(&HEIGHT_R16_MAGIC);
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    for &pixel in pixels {
        output.extend_from_slice(&pixel.to_le_bytes());
    }
    Ok(output)
}

/// Decode an R16 height payload. Intended for pipeline inspection and tests.
pub fn decode_height_r16(bytes: &[u8]) -> Result<(u32, u32, Vec<u16>), String> {
    if bytes.len() < HEIGHT_R16_HEADER_BYTES {
        return Err("R16 height payload is truncated".into());
    }
    if bytes[..8] != HEIGHT_R16_MAGIC {
        return Err("R16 height magic/version mismatch".into());
    }
    let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err("R16 height dimensions must be non-zero".into());
    }
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "R16 height dimensions overflow".to_string())?;
    let expected = count
        .checked_mul(2)
        .and_then(|payload| payload.checked_add(HEIGHT_R16_HEADER_BYTES))
        .ok_or_else(|| "R16 height payload size overflow".to_string())?;
    if bytes.len() != expected {
        return Err(format!(
            "R16 height byte length mismatch: expected {expected}, got {}",
            bytes.len()
        ));
    }
    let mut pixels = Vec::with_capacity(count);
    for sample in bytes[HEIGHT_R16_HEADER_BYTES..].chunks_exact(2) {
        pixels.push(u16::from_le_bytes([sample[0], sample[1]]));
    }
    Ok((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_normalized_extremes_without_precision_loss() {
        let pixels = [0, 1, 255, 256, 32_768, 65_534, 65_535];
        let encoded = encode_height_r16(7, 1, &pixels).unwrap();
        assert_eq!(&encoded[..8], &HEIGHT_R16_MAGIC);
        assert_eq!(encoded.len(), HEIGHT_R16_HEADER_BYTES + pixels.len() * 2);
        assert_eq!(
            decode_height_r16(&encoded).unwrap(),
            (7, 1, pixels.to_vec())
        );
    }

    #[test]
    fn preserves_16_bit_png_decode_instead_of_converting_to_luma8() {
        let pixels = vec![0, 1, 255, 256, 32_768, 65_534, 65_535];
        let source =
            image::ImageBuffer::<image::Luma<u16>, _>::from_raw(7, 1, pixels.clone()).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma16(source)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let decoded = image::load_from_memory(png.get_ref()).unwrap();
        let (width, height, encoded) = encode_height_r16_image(decoded).unwrap();
        assert_eq!((width, height), (7, 1));
        assert_eq!(decode_height_r16(&encoded).unwrap().2, pixels);
    }

    #[test]
    fn rejects_zero_dimensions_and_wrong_sample_count() {
        assert!(encode_height_r16(0, 1, &[]).is_err());
        assert!(encode_height_r16(2, 2, &[0; 3]).is_err());
    }

    #[test]
    fn rejects_corrupt_truncated_and_trailing_payloads() {
        let encoded = encode_height_r16(1, 1, &[42]).unwrap();
        assert!(decode_height_r16(&encoded[..15]).is_err());
        let mut corrupt = encoded.clone();
        corrupt[0] ^= 1;
        assert!(decode_height_r16(&corrupt).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(decode_height_r16(&trailing).is_err());
    }
}
