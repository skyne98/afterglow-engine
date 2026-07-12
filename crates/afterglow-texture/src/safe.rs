// Safe wrappers for the Basis Universal transcoder (pure Rust).
//
// Uses `basisu_rs` — pure Rust, `#![no_std]`, `#![forbid(unsafe_code)]`.

use crate::{FORMAT_BC7, FORMAT_ASTC, FORMAT_ETC1, FORMAT_ETC2, FORMAT_RGBA};

/// Transcode a Basis texture to a GPU-native format.
///
/// `target_format` is one of the `FORMAT_*` constants.
/// Returns the transcoded GPU-compressed (or uncompressed) texture data.
pub fn transcode(data: &[u8], target_format: u32) -> Result<Vec<u8>, String> {
    match target_format {
        FORMAT_BC7 => {
            let images = basisu::read_to_bc7(data)
                .map_err(|e| format!("BC7 transcode: {e}"))?;
            Ok(flatten_images(&images))
        }
        FORMAT_ASTC => {
            let images = basisu::read_to_astc(data)
                .map_err(|e| format!("ASTC transcode: {e}"))?;
            Ok(flatten_images(&images))
        }
        FORMAT_ETC1 => {
            let images = basisu::read_to_etc1(data)
                .map_err(|e| format!("ETC1 transcode: {e}"))?;
            Ok(flatten_images(&images))
        }
        FORMAT_ETC2 => {
            let images = basisu::read_to_etc2(data)
                .map_err(|e| format!("ETC2 transcode: {e}"))?;
            Ok(flatten_images(&images))
        }
        FORMAT_RGBA => {
            let (_header, images) = basisu::read_to_rgba(data)
                .map_err(|e| format!("RGBA decode: {e}"))?;
            let mut out = Vec::new();
            for img in &images {
                // Image<u8> with RGBA data: stride bytes per row, h rows.
                let row_len = img.w as usize * 4;
                for y in 0..img.h as usize {
                    let start = y * img.stride as usize;
                    out.extend_from_slice(&img.data[start..start + row_len]);
                }
            }
            Ok(out)
        }
        _ => Err(format!("unknown target format: {target_format}")),
    }
}

/// Flatten a Vec<Image<u8>> into a single byte buffer.
fn flatten_images(images: &[basisu::Image<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for img in images {
        let row_len = img.w as usize * 4;
        for y in 0..img.h as usize {
            let start = y * img.stride as usize;
            out.extend_from_slice(&img.data[start..start + row_len]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcode_invalid_data_returns_error() {
        assert!(transcode(&[0; 10], FORMAT_BC7).is_err());
        assert!(transcode(&[0; 10], FORMAT_ASTC).is_err());
        assert!(transcode(&[0; 10], FORMAT_ETC1).is_err());
        assert!(transcode(&[0; 10], FORMAT_ETC2).is_err());
        assert!(transcode(&[0; 10], FORMAT_RGBA).is_err());
    }

    #[test]
    fn unknown_format_returns_error() {
        let result = transcode(&[0; 10], 99);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown"));
    }
}
