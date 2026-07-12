// C++ wrapper for the basisu_transcoder class.
// This exposes the C++ class methods as C functions so Rust can call them.
//
// Compile with: clang++ -I vendor/transcoder -c wrapper.cpp -o wrapper.o
// Also compile: basisu_transcoder.cpp in the same build.

#include "basisu_transcoder.h"
#include <cstdint>

using namespace basist;

extern "C" {

void afterglow_basisu_transcoder_init() {
    basisu_transcoder_init();
}

uint32_t afterglow_basisu_compute_transcoded_image_size(uint32_t target_format, uint32_t orig_width, uint32_t orig_height) {
    return basis_compute_transcoded_image_size_in_bytes(static_cast<transcoder_texture_format>(target_format), orig_width, orig_height);
}

uint32_t afterglow_basisu_get_bytes_per_block_or_pixel(uint32_t fmt) {
    return basis_get_bytes_per_block_or_pixel(static_cast<transcoder_texture_format>(fmt));
}

void* afterglow_basisu_transcoder_new() {
    return new basisu_transcoder();
}

void afterglow_basisu_transcoder_delete(void* tc) {
    delete static_cast<basisu_transcoder*>(tc);
}

uint32_t afterglow_basisu_get_total_image_levels(
    const void* tc, const uint8_t* data, uint32_t data_size, uint32_t image_index
) {
    auto* transcoder = static_cast<const basisu_transcoder*>(tc);
    return transcoder->get_total_image_levels(data, data_size, image_index);
}

int afterglow_basisu_get_image_level_desc(
    const void* tc, const uint8_t* data, uint32_t data_size,
    uint32_t image_index, uint32_t level_index,
    uint32_t* out_width, uint32_t* out_height, uint32_t* out_total_blocks
) {
    auto* transcoder = static_cast<const basisu_transcoder*>(tc);
    return transcoder->get_image_level_desc(
        data, data_size, image_index, level_index,
        *out_width, *out_height, *out_total_blocks
    ) ? 1 : 0;
}

int afterglow_basisu_transcode_image_level(
    const void* tc, const uint8_t* data, uint32_t data_size,
    uint32_t image_index, uint32_t level_index,
    uint8_t* output, uint32_t output_size,
    uint32_t target_format, uint32_t decode_flags
) {
    auto* transcoder = static_cast<const basisu_transcoder*>(tc);
    return transcoder->transcode_image_level(
        data, data_size, image_index, level_index,
        output, output_size,
        static_cast<transcoder_texture_format>(target_format),
        decode_flags
    ) ? 1 : 0;
}

} // extern "C"
