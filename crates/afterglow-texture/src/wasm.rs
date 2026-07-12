// WASM test runner — runs all texture operations in WASM and returns pass count.

#![cfg(target_arch = "wasm32")]

use crate::*;

/// Run all texture operations in WASM. Returns the number of passed checks.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_texture_test() -> i32 {
    let mut passed = 0;

    // --- generate_mips ---
    let rgba: Vec<u8> = (0..(8 * 8 * 4)).map(|i| (i % 256) as u8).collect();
    let mips = mips::generate_mip_chain(&rgba, 8, 8);
    if mips.len() == 4 { passed += 1; } // 8→4→2→1
    if mips[0].0 == 8 && mips[3].0 == 1 { passed += 1; }

    // --- downscale ---
    let ds = mips::downscale_box(&rgba, 8, 8, 4, 4);
    if ds.len() == 4 * 4 * 4 { passed += 1; }

    // Downscale preserves uniform color.
    let white = vec![255u8; 4 * 4 * 4];
    let ds_white = mips::downscale_box(&white, 4, 4, 2, 2);
    if ds_white.iter().all(|&b| b == 255) { passed += 1; }

    // --- transcode (invalid data) ---
    let r = safe::transcode(&[0; 10], FORMAT_BC7);
    if r.is_err() { passed += 1; }

    let r = safe::transcode(&[0; 10], FORMAT_ASTC);
    if r.is_err() { passed += 1; }

    let r = safe::transcode(&[0; 10], FORMAT_RGBA);
    if r.is_err() { passed += 1; }

    // Unknown format.
    let r = safe::transcode(&[0; 10], 99);
    if r.is_err() && r.unwrap_err().contains("unknown") { passed += 1; }

    // --- non-power-of-two mip chain ---
    let rgba6: Vec<u8> = vec![100; 6 * 6 * 4];
    let mips6 = mips::generate_mip_chain(&rgba6, 6, 6);
    if mips6.len() >= 3 && mips6[0].0 == 6 { passed += 1; }

    // --- downscale non-uniform ---
    let rgba64 = vec![200u8; 64 * 32 * 4];
    let ds_nu = mips::downscale_box(&rgba64, 64, 32, 32, 16);
    if ds_nu.len() == 32 * 16 * 4 { passed += 1; }

    passed
}

/// Benchmark: generate mip chain for a 256×256 texture. Returns microseconds.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_texture_bench_mips() -> f64 {
    let rgba = vec![128u8; 256 * 256 * 4];

    let start = web_time_ms();
    let mips = mips::generate_mip_chain(&rgba, 256, 256);
    let elapsed = web_time_ms() - start;

    unsafe {
        BENCH_MIPS_COUNT = mips.len() as f64;
        BENCH_MIPS_TIME_US = elapsed * 1000.0;
    }
    elapsed * 1000.0
}

static mut BENCH_MIPS_COUNT: f64 = 0.0;
static mut BENCH_MIPS_TIME_US: f64 = 0.0;

#[unsafe(no_mangle)] pub extern "C" fn bench_mips_count() -> f64 { unsafe { BENCH_MIPS_COUNT } }
#[unsafe(no_mangle)] pub extern "C" fn bench_mips_time_us() -> f64 { unsafe { BENCH_MIPS_TIME_US } }

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn performance_now() -> f64;
}

fn web_time_ms() -> f64 {
    unsafe { performance_now() }
}
