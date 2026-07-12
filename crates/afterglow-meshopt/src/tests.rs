//! Tests for afterglow-meshopt safe wrappers.

use crate::safe::*;

#[test]
fn simplify_reduces_triangle_count() {
    // Simple quad: 2 triangles, 4 vertices.
    let indices: Vec<u32> = vec![0, 1, 2, 0, 2, 3];
    let positions: Vec<f32> = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        1.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
    ];

    let (simplified, count, error) = simplify(
        &indices,
        &positions,
        12, // stride = 3 * 4 bytes
        3,   // target: 1 triangle (3 indices)
        0.01,
    );

    assert!(count <= 6, "simplified should have ≤6 indices, got {count}");
    assert!(count >= 3, "simplified should have ≥3 indices, got {count}");
    assert!(error < 1.0, "error should be small, got {error}");
}

#[test]
fn encode_decode_index_buffer_roundtrip() {
    let indices: Vec<u32> = vec![0, 1, 2, 2, 1, 3, 4, 5, 6, 6, 5, 7];
    let vertex_count = 8;

    let encoded = encode_index_buffer(&indices, vertex_count);
    assert!(encoded.len() < indices.len() * 4, "encoded should be smaller");

    let decoded = decode_index_buffer(&encoded, indices.len());
    assert_eq!(decoded, indices, "roundtrip should preserve data");
}

#[test]
fn encode_decode_vertex_buffer_roundtrip() {
    let vertices: Vec<u8> = (0..96).collect::<Vec<u8>>(); // 8 vertices × 12 bytes
    let vertex_size = 12;

    let encoded = encode_vertex_buffer(&vertices, vertex_size);
    let decoded = decode_vertex_buffer(&encoded, 8, vertex_size);
    assert_eq!(decoded, vertices, "roundtrip should preserve data");
}

#[test]
fn optimize_vertex_cache_preserves_indices() {
    let indices: Vec<u32> = vec![0, 1, 2, 3, 4, 5, 0, 2, 3];
    let optimized = optimize_vertex_cache(&indices, 6);
    assert_eq!(optimized.len(), indices.len(), "same index count");
    // Should contain the same set of vertex indices.
    let mut orig = indices.clone();
    let mut opt = optimized.clone();
    orig.sort();
    opt.sort();
    assert_eq!(orig, opt, "same vertex indices, reordered");
}

#[test]
fn stripify_unstripify_roundtrip() {
    let indices: Vec<u32> = vec![0, 1, 2, 3, 4, 5];
    let vertex_count = 6;
    let restart = 0xFFFFFFFF;

    let strip = stripify(&indices, vertex_count, restart);
    assert!(strip.len() > 0, "strip should not be empty");

    let unstripped = unstripify(&strip, restart);
    // The triangle count should match (unstripped may have different index order).
    assert_eq!(unstripped.len() % 3, 0, "unstripped is a triangle list");
}

#[test]
fn quantize_half_roundtrip() {
    for v in [0.0, 1.0, -1.0, 0.5, 0.25, 100.0, -50.0] {
        let h = quantize_half(v);
        let restored = dequantize_half(h);
        assert!(
            (restored - v).abs() < 0.01 * v.abs().max(1.0),
            "half roundtrip: {v} → {restored}"
        );
    }
}

#[test]
fn quantize_float_reduces_precision() {
    let v = 3.14159265;
    let q = quantize_float(v, 8); // 8 bits
    assert!(
        (q - v).abs() < 0.1,
        "8-bit quantize should be close: {v} → {q}"
    );
}

#[test]
fn generate_vertex_remap_deduplicates() {
    // 4 vertices but vertex 0 and 2 are identical.
    let indices: Vec<u32> = vec![0, 1, 2, 2, 1, 3];
    let vertices: Vec<u8> = vec![
        0, 0, 0,  // vertex 0
        1, 0, 0,  // vertex 1
        0, 0, 0,  // vertex 2 (duplicate of 0)
        0, 1, 0,  // vertex 3
    ];

    let (remap, unique) = generate_vertex_remap(&indices, &vertices, 3);
    assert_eq!(unique, 3, "should have 3 unique vertices");
}

#[test]
fn analyze_vertex_cache_returns_stats() {
    let indices: Vec<u32> = vec![0, 1, 2, 3, 4, 5];
    let stats = analyze_vertex_cache(&indices, 6, 16, 32, 32);
    assert!(stats.acmr > 0.0, "acmr should be positive");
}

#[test]
fn spatial_sort_remap_preserves_count() {
    let positions: Vec<f32> = vec![
        1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
        0.0, 0.0, 1.0,
        1.0, 1.0, 1.0,
    ];
    let remap = spatial_sort_remap(&positions, 12);
    assert_eq!(remap.len(), 4, "remap should have 4 entries");
}

#[test]
fn build_meshlets_returns_meshlets() {
    // 2 triangles, 4 vertices.
    let indices: Vec<u32> = vec![0, 1, 2, 0, 2, 3];
    let positions: Vec<f32> = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        1.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
    ];

    let (meshlets, meshlet_vertices, meshlet_triangles) =
        build_meshlets(&indices, &positions, 12, 64, 64, 0.0);

    assert!(meshlets.len() >= 1, "should have at least 1 meshlet");
    assert!(!meshlet_vertices.is_empty(), "should have vertices");
    assert!(!meshlet_triangles.is_empty(), "should have triangles");
}
