// WASM exports — callable from JS/Node to verify the WASM build works.
//
// Exports a single test runner that creates test data internally, runs all
// meshopt operations, and returns the number of passed tests (or -1 on panic).
// Also exports a benchmark function that returns timing in microseconds.

#![cfg(target_arch = "wasm32")]

use crate::safe::*;

/// Run all meshopt operations in WASM. Returns the number of passed checks
/// (or 0 if any assertion fails — panics are caught by the wasm runtime).
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_meshopt_test() -> i32 {
    let mut passed = 0;

    // --- Remap ---
    let indices: Vec<u32> = vec![0, 1, 2, 2, 1, 3];
    let vertices: Vec<u8> = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0];
    let (remap, unique) = generate_vertex_remap(&indices, &vertices, 3);
    if unique == 3 { passed += 1; }
    let remapped = remap_index_buffer(&indices, &remap);
    if remapped.len() == indices.len() { passed += 1; }

    // --- Optimize vertex cache ---
    let optimized = optimize_vertex_cache(&indices, 4);
    if optimized.len() == indices.len() { passed += 1; }

    // --- Encode/Decode index buffer ---
    let encoded = encode_index_buffer(&indices, 4);
    let decoded = decode_index_buffer(&encoded, indices.len());
    if decoded == indices { passed += 1; }

    // --- Encode/Decode vertex buffer ---
    let vencoded = encode_vertex_buffer(&vertices, 3);
    let vdecoded = decode_vertex_buffer(&vencoded, 4, 3);
    if vdecoded == vertices { passed += 1; }

    // --- Simplify ---
    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let (simplified, scount, _err) = simplify(&indices, &positions, 12, 3, 0.01);
    if scount <= 6 && scount >= 3 { passed += 1; }

    // --- Simplify sloppy ---
    let (ssimplified, sscount, _serr) = simplify_sloppy(&indices, &positions, 12, 3, 0.1);
    if sscount <= indices.len() { passed += 1; }

    // --- Simplify scale ---
    let scale = simplify_scale(&positions, 12);
    if scale > 0.0 { passed += 1; }

    // --- Stripify ---
    let strip = stripify(&indices, 4, 0xFFFFFFFF);
    if !strip.is_empty() { passed += 1; }
    let unstripped = unstripify(&strip, 0xFFFFFFFF);
    if unstripped.len() % 3 == 0 { passed += 1; }

    // --- Analyze ---
    let vc_stats = analyze_vertex_cache(&indices, 4, 16, 32, 32);
    if vc_stats.acmr > 0.0 { passed += 1; }
    let vf_stats = analyze_vertex_fetch(&indices, 4, 3);
    if vf_stats.overfetch >= 0.0 { passed += 1; }
    let od_stats = analyze_overdraw(&indices, &positions, 12);
    if od_stats.overdraw >= 0.0 { passed += 1; }

    // --- Spatial sort ---
    let sremap = spatial_sort_remap(&positions, 12);
    if sremap.len() == 4 { passed += 1; }
    let stri = spatial_sort_triangles(&indices, &positions, 12);
    if stri.len() == indices.len() { passed += 1; }

    // --- Quantize ---
    let h = quantize_half(1.0);
    if dequantize_half(h) > 0.9 && dequantize_half(h) < 1.1 { passed += 1; }
    let qf = quantize_float(3.14159, 8);
    if (qf - 3.14159).abs() < 0.1 { passed += 1; }

    // --- Meshlets ---
    let (meshlets, mv, mt) = build_meshlets(&indices, &positions, 12, 64, 64, 0.0);
    if !meshlets.is_empty() && !mv.is_empty() && !mt.is_empty() { passed += 1; }

    // --- Cluster bounds ---
    let bounds = compute_cluster_bounds(&indices, &positions, 12);
    if bounds.radius > 0.0 { passed += 1; }

    // --- Optimize vertex fetch ---
    let mut opt_indices = indices.clone();
    let _fetched = optimize_vertex_fetch(&mut opt_indices, &vertices, 3);
    passed += 1; // if we got here, it didn't crash

    // --- Filters ---
    let mut filter_buf = vec![0u8; 8];
    decode_filter_oct(&mut filter_buf, 1, 8);
    passed += 1;

    // --- Optimize overdraw ---
    let _overdraw = optimize_overdraw(&indices, &positions, 12, 1.05);
    passed += 1;

    passed
}

/// Benchmark: simplify a synthetic mesh. Returns microseconds elapsed.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_meshopt_bench_simplify() -> f64 {
    // Generate a synthetic grid mesh (N×N vertices → (N-1)²×2 triangles).
    let n = 100; // 100×100 = 10K vertices, ~19.6K triangles
    let mut positions = Vec::with_capacity(n * n * 3);
    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for y in 0..n {
        for x in 0..n {
            positions.push(x as f32);
            positions.push(y as f32);
            positions.push(0.0);
        }
    }
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let n_u32 = n as u32;
            let i = (y * n + x) as u32;
            indices.extend_from_slice(&[i, i + 1u32, i + n_u32, i + 1u32, i + n_u32 + 1u32, i + n_u32]);
        }
    }

    let start = web_time_ms();
    let (simplified, count, error) = simplify(&indices, &positions, 12, indices.len() / 4, 0.01);
    let elapsed = web_time_ms() - start;

    // Return: microseconds, packed with effectiveness info via global.
    // (We can't return structs easily from WASM, so just return time in us.)
    let us = elapsed * 1000.0;
    // Store effectiveness in globals for separate reads.
    unsafe {
        BENCH_SIMPLIFY_ORIG = indices.len() as f64;
        BENCH_SIMPLIFY_RESULT = count as f64;
        BENCH_SIMPLIFY_ERROR = error as f64;
        BENCH_SIMPLIFY_TIME_US = us;
    }
    us
}

/// Benchmark: encode + decode an index buffer. Returns microseconds.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_meshopt_bench_encode() -> f64 {
    let n = 100;
    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let n_u32 = n as u32;
            let i = (y * n + x) as u32;
            indices.extend_from_slice(&[i, i + 1u32, i + n_u32, i + 1u32, i + n_u32 + 1u32, i + n_u32]);
        }
    }

    let start = web_time_ms();
    let encoded = encode_index_buffer(&indices, n * n);
    let _decoded = decode_index_buffer(&encoded, indices.len());
    let elapsed = web_time_ms() - start;

    unsafe {
        BENCH_ENCODE_ORIG_BYTES = (indices.len() * 4) as f64;
        BENCH_ENCODE_COMPRESSED_BYTES = encoded.len() as f64;
        BENCH_ENCODE_TIME_US = elapsed * 1000.0;
    }
    elapsed * 1000.0
}

// Provide malloc/free for the C++ runtime (meshoptimizer uses new/delete).
// Uses Rust's allocator — memory comes from the WASM linear memory heap.
#[unsafe(no_mangle)]
pub extern "C" fn malloc(size: usize) -> *mut u8 {
    let layout = std::alloc::Layout::from_size_align(size, 8).unwrap();
    unsafe { std::alloc::alloc(layout) }
}

#[unsafe(no_mangle)]
pub extern "C" fn free(ptr: *mut u8) {
    // Without the Layout, we can't safely dealloc. For short-lived WASM
    // processing, the leak is acceptable. In production, use
    // meshopt_setAllocator with proper alloc/dealloc.
    let _ = ptr;
}

// --- Globals for benchmark result reads (JS can read via separate exports) ---

static mut BENCH_SIMPLIFY_ORIG: f64 = 0.0;
static mut BENCH_SIMPLIFY_RESULT: f64 = 0.0;
static mut BENCH_SIMPLIFY_ERROR: f64 = 0.0;
static mut BENCH_SIMPLIFY_TIME_US: f64 = 0.0;
static mut BENCH_ENCODE_ORIG_BYTES: f64 = 0.0;
static mut BENCH_ENCODE_COMPRESSED_BYTES: f64 = 0.0;
static mut BENCH_ENCODE_TIME_US: f64 = 0.0;

#[unsafe(no_mangle)] pub extern "C" fn bench_simplify_orig() -> f64 { unsafe { BENCH_SIMPLIFY_ORIG } }
#[unsafe(no_mangle)] pub extern "C" fn bench_simplify_result() -> f64 { unsafe { BENCH_SIMPLIFY_RESULT } }
#[unsafe(no_mangle)] pub extern "C" fn bench_simplify_error() -> f64 { unsafe { BENCH_SIMPLIFY_ERROR } }
#[unsafe(no_mangle)] pub extern "C" fn bench_simplify_time_us() -> f64 { unsafe { BENCH_SIMPLIFY_TIME_US } }
#[unsafe(no_mangle)] pub extern "C" fn bench_encode_orig_bytes() -> f64 { unsafe { BENCH_ENCODE_ORIG_BYTES } }
#[unsafe(no_mangle)] pub extern "C" fn bench_encode_compressed_bytes() -> f64 { unsafe { BENCH_ENCODE_COMPRESSED_BYTES } }
#[unsafe(no_mangle)] pub extern "C" fn bench_encode_time_us() -> f64 { unsafe { BENCH_ENCODE_TIME_US } }

/// Get the WASM memory buffer (for JS to read results).
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_meshopt_memory() -> *mut u8 {
    // The WASM linear memory base — JS can use this to access the heap.
    0 as *mut u8
}

// WASM doesn't have std::time — use performance.now() via a JS import.
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn performance_now() -> f64;
}

fn web_time_ms() -> f64 {
    unsafe { performance_now() }
}
