//! Offline-only Basis Universal encoder.
//!
//! Runtime transcoding stays in `afterglow-texture` and remains pure Rust. This
//! crate deliberately isolates the official C++ encoder and is never linked
//! into native game or wasm runtime targets.

use basis_universal::{BasisTextureFormat, Compressor, CompressorParams};

/// Encode one tightly packed RGBA8 image as a single-level UASTC `.basis` file.
pub fn encode_uastc_rgba(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 || data.len() != expected {
        return Err(format!("RGBA dimensions {width}x{height} require {expected} bytes, got {}", data.len()));
    }

    let mut params = CompressorParams::new();
    params.set_generate_mipmaps(false);
    params.set_basis_format(BasisTextureFormat::UASTC4x4);
    params.set_uastc_quality_level(basis_universal::UASTC_QUALITY_DEFAULT);
    params.set_print_status_to_stdout(false);
    params.source_image_mut(0).init(data, width, height, 4);

    let mut compressor = Compressor::default();
    // The upstream wrapper requires unsafe for its FFI lifecycle. The unsafe
    // boundary is confined to this offline-only crate.
    unsafe {
        compressor.init(&params);
        compressor.process().map_err(|error| format!("Basis UASTC encode failed: {error:?}"))?;
    }
    Ok(compressor.basis_file().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_rgba_size() {
        assert!(encode_uastc_rgba(&[0; 3], 1, 1).is_err());
        assert!(encode_uastc_rgba(&[], 0, 1).is_err());
    }

    #[test]
    fn encoded_page_transcodes_with_runtime_decoder() {
        let mut rgba = vec![0; 8 * 8 * 4];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[17, 83, 201, 255]);
        }
        let basis = encode_uastc_rgba(&rgba, 8, 8).unwrap();
        let decoded = afterglow_texture::transcode(&basis, afterglow_texture::FORMAT_RGBA).unwrap();
        assert!(!decoded.is_empty());
    }
}
