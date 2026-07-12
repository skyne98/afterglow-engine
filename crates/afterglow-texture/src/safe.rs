// Safe wrappers for the Basis Universal transcoder.
//
// The transcoder decodes Basis Universal / KTX2 textures to GPU-native
// formats (BC7, ASTC, ETC2, etc.) at load time.

use std::os::raw::c_void;
use crate::ffi;

/// Ensure the transcoder lookup tables are initialized.
/// Called automatically by [`transcode`] on first use.
static INITIALIZED: std::sync::Once = std::sync::Once::new();

fn ensure_init() {
    INITIALIZED.call_once(|| {
        unsafe { ffi::afterglow_basisu_transcoder_init() };
    });
}

/// Transcode a Basis/KTX2 texture to a GPU-native format.
///
/// `target_format` is a `transcoder_texture_format` constant:
/// - 6 = BC7_RGBA (desktop — best quality)
/// - 10 = ASTC_LDR_4x4_RGBA (mobile)
/// - 13 = RGBA32 (uncompressed, for fallback)
///
/// Returns the transcoded GPU-compressed texture data.
pub fn transcode(data: &[u8], target_format: u32) -> Result<Vec<u8>, String> {
    ensure_init();

    // Create a transcoder instance.
    let tc = unsafe { ffi::afterglow_basisu_transcoder_new() };
    if tc.is_null() {
        return Err("failed to create transcoder".into());
    }

    // Clean up on exit.
    struct Dropper(*mut c_void);
    impl Drop for Dropper {
        fn drop(&mut self) {
            unsafe { ffi::afterglow_basisu_transcoder_delete(self.0) };
        }
    }
    let _dropper = Dropper(tc);

    // Image 0 (most .basis files have one image).
    let image_index = 0u32;

    // Get level 0 (highest resolution).
    let level_index = 0u32;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut total_blocks = 0u32;

    let ok = unsafe {
        ffi::afterglow_basisu_get_image_level_desc(
            tc, data.as_ptr(), data.len() as u32,
            image_index, level_index,
            &mut width, &mut height, &mut total_blocks,
        )
    };
    if ok == 0 {
        return Err("failed to get image level desc".into());
    }

    // Compute output size.
    let output_size = unsafe {
        ffi::afterglow_basisu_compute_transcoded_image_size(target_format, width, height)
    };

    // Transcode.
    let mut output = vec![0u8; output_size as usize];
    let ok = unsafe {
        ffi::afterglow_basisu_transcode_image_level(
            tc, data.as_ptr(), data.len() as u32,
            image_index, level_index,
            output.as_mut_ptr(), output.len() as u32,
            target_format, 0,
        )
    };
    if ok == 0 {
        return Err("transcode_image_level failed".into());
    }

    Ok(output)
}

/// Get the number of mip levels in a Basis/KTX2 texture.
pub fn get_mip_count(data: &[u8]) -> u32 {
    ensure_init();
    let tc = unsafe { ffi::afterglow_basisu_transcoder_new() };
    if tc.is_null() {
        return 0;
    }
    let count = unsafe {
        ffi::afterglow_basisu_get_total_image_levels(tc, data.as_ptr(), data.len() as u32, 0)
    };
    unsafe { ffi::afterglow_basisu_transcoder_delete(tc) };
    count
}

/// Get the dimensions of a specific mip level.
pub fn get_level_dimensions(data: &[u8], level_index: u32) -> Option<(u32, u32)> {
    ensure_init();
    let tc = unsafe { ffi::afterglow_basisu_transcoder_new() };
    if tc.is_null() {
        return None;
    }
    let mut width = 0u32;
    let mut height = 0u32;
    let mut total_blocks = 0u32;
    let ok = unsafe {
        ffi::afterglow_basisu_get_image_level_desc(
            tc, data.as_ptr(), data.len() as u32,
            0, level_index,
            &mut width, &mut height, &mut total_blocks,
        )
    };
    unsafe { ffi::afterglow_basisu_transcoder_delete(tc) };
    if ok == 0 { None } else { Some((width, height)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcode_invalid_data_returns_error() {
        let result = transcode(&[0; 10], 6);
        assert!(result.is_err());
    }

    #[test]
    fn mip_count_invalid_data_returns_zero() {
        let count = get_mip_count(&[0; 10]);
        assert_eq!(count, 0);
    }
}
