// Safe wrappers for the Basis Universal transcoder (pure Rust).
//
// Uses `basisu_rs` — pure Rust, `#![no_std]`, `#![forbid(unsafe_code)]`.
//
// Output format for `transcode`:
//   [mip_count: u32 LE][
//     [width: u32 LE][height: u32 LE][data_len: u32 LE][data...]
//   ]...
// Each mip level is one entry. The AssetStore parses this to stream mips
// progressively (lowest-res first).

use crate::{FORMAT_BC7, FORMAT_ASTC, FORMAT_ETC1, FORMAT_ETC2, FORMAT_RGBA};

/// Transcode a Basis texture to a GPU-native format.
///
/// `target_format` is one of the `FORMAT_*` constants.
/// Returns serialized mip data: `[count][w0][h0][len0][data0]...`
pub fn transcode(data: &[u8], target_format: u32) -> Result<Vec<u8>, String> {
    match target_format {
        FORMAT_BC7 => {
            let images = basisu::read_to_bc7(data)
                .map_err(|e| format!("BC7 transcode: {e}"))?;
            Ok(serialize_mips(&images))
        }
        FORMAT_ASTC => {
            let images = basisu::read_to_astc(data)
                .map_err(|e| format!("ASTC transcode: {e}"))?;
            Ok(serialize_mips(&images))
        }
        FORMAT_ETC1 => {
            let images = basisu::read_to_etc1(data)
                .map_err(|e| format!("ETC1 transcode: {e}"))?;
            Ok(serialize_mips(&images))
        }
        FORMAT_ETC2 => {
            let images = basisu::read_to_etc2(data)
                .map_err(|e| format!("ETC2 transcode: {e}"))?;
            Ok(serialize_mips(&images))
        }
        FORMAT_RGBA => {
            let (_header, images) = basisu::read_to_rgba(data)
                .map_err(|e| format!("RGBA decode: {e}"))?;
            Ok(serialize_mips(&images))
        }
        _ => Err(format!("unknown target format: {target_format}")),
    }
}

/// Serialize images into the mip format: `[count][w][h][len][data]...`
///
/// For block-compressed formats (BC7, ASTC, ETC), `Image.data` is already a
/// flat buffer of blocks — just copy it. For RGBA, the data is row-major with
/// a stride that may exceed `w * 4`, so we strip padding.
fn serialize_mips(images: &[basisu::Image<u8>]) -> Vec<u8> {
    let count = images.len() as u32;
    let mut out = Vec::with_capacity(4 + images.len() * 12);
    out.extend_from_slice(&count.to_le_bytes());

    for img in images {
        let w = img.w as u32;
        let h = img.h as u32;
        // For block formats, stride = block_size * num_blocks_x, and data is
        // num_blocks_y rows of that. row_len = stride (no padding to strip).
        // For RGBA, stride = w * 4 (already tight in basisu_rs), so row_len = w * 4.
        let row_len = img.stride as usize;
        let data_len = row_len * (img.h as usize);
        // If stride is tight (RGBA case), data_len == img.data.len().
        // For block formats, stride * h > img.data.len() because h is pixel
        // height, not block-row count. In that case, data is already flat.
        let bytes: &[u8] = if data_len <= img.data.len() {
            &img.data[..data_len]
        } else {
            // Block-compressed: data is already a flat buffer.
            &img.data[..]
        };
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
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

    #[test]
    fn transcode_real_checker_basis_bc7() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../afterglow-web/www/checker.basis");
        let data = std::fs::read(&path)
            .unwrap_or_else(|_| { return Vec::new(); });
        if data.is_empty() { return; } // skip if file not present
        let result = transcode(&data, FORMAT_BC7);
        assert!(result.is_ok(), "BC7 transcode failed: {:?}", result.err());
        let out = result.unwrap();
        // Should start with mip count (≥1)
        let count = u32::from_le_bytes(out[0..4].try_into().unwrap());
        assert!(count >= 1, "expected at least 1 mip, got {count}");
    }

    #[test]
    fn transcode_real_checker_basis_rgba() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../afterglow-web/www/checker.basis");
        let data = std::fs::read(&path)
            .unwrap_or_else(|_| { return Vec::new(); });
        if data.is_empty() { return; } // skip if file not present
        let result = transcode(&data, FORMAT_RGBA);
        assert!(result.is_ok(), "RGBA transcode failed: {:?}", result.err());
        let out = result.unwrap();
        let count = u32::from_le_bytes(out[0..4].try_into().unwrap());
        assert!(count >= 1, "expected at least 1 mip, got {count}");
        // First mip should be 128×128
        let w = u32::from_le_bytes(out[4..8].try_into().unwrap());
        let h = u32::from_le_bytes(out[8..12].try_into().unwrap());
        assert_eq!(w, 128, "expected width 128, got {w}");
        assert_eq!(h, 128, "expected height 128, got {h}");
    }
}
