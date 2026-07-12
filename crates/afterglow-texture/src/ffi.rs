// FFI declarations for the Basis Universal transcoder.
//
// The transcoder is a single .cpp file (basisu_transcoder.cpp) with no deps.
// It decodes Basis Universal / KTX2 textures to GPU-native formats.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::os::raw::{c_int, c_uint, c_void};

// --- transcoder_texture_format enum (subset of the most useful formats) ---

pub const cTFETC1_RGB: c_uint = 0;
pub const cTFETC2_RGBA: c_uint = 1;
pub const cTFBC1_RGB: c_uint = 2;
pub const cTFBC3_RGBA: c_uint = 3;
pub const cTFBC4_R: c_uint = 4;
pub const cTFBC5_RG: c_uint = 5;
pub const cTFBC7_RGBA: c_uint = 6;
pub const cTFASTC_LDR_4x4_RGBA: c_uint = 10;
pub const cTFRGBA32: c_uint = 13;
pub const cTFBC6H: c_uint = 22;
pub const cTFASTC_HDR_4x4_RGBA: c_uint = 23;

// --- free functions ---

unsafe extern "C" {
    /// Must be called once before any transcoder functions.
    pub fn afterglow_basisu_transcoder_init();

    /// Compute the output size in bytes for a transcoded image.
    pub fn afterglow_basisu_compute_transcoded_image_size(
        target_format: c_uint,
        orig_width: c_uint,
        orig_height: c_uint,
    ) -> c_uint;

    /// Get bytes per block or pixel for a format.
    pub fn afterglow_basisu_get_bytes_per_block_or_pixel(fmt: c_uint) -> c_uint;
}

// --- basisu_transcoder class (C++ — we use a C wrapper) ---
// The class methods are accessed via C wrapper functions in a small .cpp file.

unsafe extern "C" {
    /// Create a basisu_transcoder instance. Returns an opaque pointer.
    pub fn afterglow_basisu_transcoder_new() -> *mut c_void;

    /// Destroy a basisu_transcoder instance.
    pub fn afterglow_basisu_transcoder_delete(tc: *mut c_void);

    /// Get the number of mip levels for an image in a .basis file.
    pub fn afterglow_basisu_get_total_image_levels(
        tc: *const c_void,
        data: *const u8,
        data_size: u32,
        image_index: u32,
    ) -> u32;

    /// Get level description (width, height, total_blocks).
    /// Returns true on success. Outputs are written to the pointers.
    pub fn afterglow_basisu_get_image_level_desc(
        tc: *const c_void,
        data: *const u8,
        data_size: u32,
        image_index: u32,
        level_index: u32,
        out_width: *mut u32,
        out_height: *mut u32,
        out_total_blocks: *mut u32,
    ) -> c_int;

    /// Transcode a single image level to the target format.
    /// Returns true on success.
    pub fn afterglow_basisu_transcode_image_level(
        tc: *const c_void,
        data: *const u8,
        data_size: u32,
        image_index: u32,
        level_index: u32,
        output: *mut u8,
        output_size: u32,
        target_format: c_uint,
        decode_flags: u32,
    ) -> c_int;
}
