// Safe wrappers for the meshoptimizer C API.
//
// All functions take Rust slices (`&[u32]`, `&[u8]`, `&[f32]`) and return
// owned `Vec`s — no manual memory management, no unsafe on the caller side.
//
// The API is uniform: each category has a clear function group, and buffer
// sizing is handled internally (using the `Bound` functions from the C API).

use std::os::raw::{c_int, c_void};

use crate::ffi;

// ========================================================================
// REMAP — deduplicate vertices
// ========================================================================

/// Deduplicate vertices and generate a remap table.
///
/// Returns `(remap, unique_vertex_count)`. The remap table maps old vertex
/// indices → new indices. Use [`remap_vertex_buffer`] and [`remap_index_buffer`]
/// to apply it.
pub fn generate_vertex_remap(
    indices: &[u32],
    vertices: &[u8],
    vertex_size: usize,
) -> (Vec<u32>, usize) {
    let vertex_count = vertices.len() / vertex_size;
    let mut remap = vec![0u32; vertex_count];
    let unique = unsafe {
        ffi::meshopt_generateVertexRemap(
            remap.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertices.as_ptr() as *const c_void,
            vertex_count,
            vertex_size,
        )
    };
    (remap, unique)
}

/// Apply a remap table to a vertex buffer.
pub fn remap_vertex_buffer(vertices: &[u8], vertex_size: usize, remap: &[u32]) -> Vec<u8> {
    let unique_count = *remap.iter().max().unwrap_or(&0) as usize + 1;
    let mut out = vec![0u8; unique_count * vertex_size];
    unsafe {
        ffi::meshopt_remapVertexBuffer(
            out.as_mut_ptr() as *mut c_void,
            vertices.as_ptr() as *const c_void,
            remap.len(),
            vertex_size,
            remap.as_ptr(),
        );
    }
    out
}

/// Apply a remap table to an index buffer.
pub fn remap_index_buffer(indices: &[u32], remap: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; indices.len()];
    unsafe {
        ffi::meshopt_remapIndexBuffer(out.as_mut_ptr(), indices.as_ptr(), indices.len(), remap.as_ptr());
    }
    out
}

// ========================================================================
// OPTIMIZE — vertex cache, overdraw, vertex fetch
// ========================================================================

/// Reorder triangles for GPU vertex cache efficiency (FIFO cache).
/// Call this after [`generate_vertex_remap`] for best results.
pub fn optimize_vertex_cache(indices: &[u32], vertex_count: usize) -> Vec<u32> {
    let mut out = vec![0u32; indices.len()];
    unsafe {
        ffi::meshopt_optimizeVertexCache(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_count,
        );
    }
    out
}

/// Reorder triangles to reduce overdraw. `threshold` is typically 1.05.
/// Requires vertex positions (3 floats per vertex).
pub fn optimize_overdraw(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
    threshold: f32,
) -> Vec<u32> {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    let mut out = vec![0u32; indices.len()];
    unsafe {
        ffi::meshopt_optimizeOverdraw(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
            threshold,
        );
    }
    out
}

/// Reorder vertices for memory locality (minimize GPU fetch). Returns
/// `(reordered_vertices, indices)`. The index buffer is modified in-place.
pub fn optimize_vertex_fetch(
    indices: &mut [u32],
    vertices: &[u8],
    vertex_size: usize,
) -> Vec<u8> {
    let vertex_count = vertices.len() / vertex_size;
    let mut out = vec![0u8; vertex_count * vertex_size];
    unsafe {
        ffi::meshopt_optimizeVertexFetch(
            out.as_mut_ptr() as *mut c_void,
            indices.as_mut_ptr(),
            indices.len(),
            vertices.as_ptr() as *const c_void,
            vertex_count,
            vertex_size,
        );
    }
    out
}

// ========================================================================
// ENCODE / DECODE — compress index + vertex buffers
// ========================================================================

/// Compress an index buffer into compact bytes.
pub fn encode_index_buffer(indices: &[u32], vertex_count: usize) -> Vec<u8> {
    let bound = unsafe { ffi::meshopt_encodeIndexBufferBound(indices.len(), vertex_count) };
    let mut out = vec![0u8; bound];
    let len = unsafe {
        ffi::meshopt_encodeIndexBuffer(
            out.as_mut_ptr(),
            out.len(),
            indices.as_ptr(),
            indices.len(),
        )
    };
    out.truncate(len);
    out
}

/// Decompress an index buffer. `index_count` is the number of indices (not bytes).
pub fn decode_index_buffer(buffer: &[u8], index_count: usize) -> Vec<u32> {
    let mut out = vec![0u32; index_count];
    let rc = unsafe {
        ffi::meshopt_decodeIndexBuffer(
            out.as_mut_ptr() as *mut c_void,
            index_count,
            std::mem::size_of::<u32>(),
            buffer.as_ptr(),
            buffer.len(),
        )
    };
    if rc != 0 { return Vec::new(); }
    out
}

/// Compress a vertex buffer into compact bytes.
pub fn encode_vertex_buffer(vertices: &[u8], vertex_size: usize) -> Vec<u8> {
    let vertex_count = vertices.len() / vertex_size;
    let bound = unsafe { ffi::meshopt_encodeVertexBufferBound(vertex_count, vertex_size) };
    let mut out = vec![0u8; bound];
    let len = unsafe {
        ffi::meshopt_encodeVertexBuffer(
            out.as_mut_ptr(),
            out.len(),
            vertices.as_ptr() as *const c_void,
            vertex_count,
            vertex_size,
        )
    };
    out.truncate(len);
    out
}

/// Decompress a vertex buffer.
pub fn decode_vertex_buffer(buffer: &[u8], vertex_count: usize, vertex_size: usize) -> Vec<u8> {
    let mut out = vec![0u8; vertex_count * vertex_size];
    let rc = unsafe {
        ffi::meshopt_decodeVertexBuffer(
            out.as_mut_ptr() as *mut c_void,
            vertex_count,
            vertex_size,
            buffer.as_ptr(),
            buffer.len(),
        )
    };
    if rc != 0 { return Vec::new(); }
    out
}

// ========================================================================
// SIMPLIFY — LOD generation
// ========================================================================

/// Simplify a mesh to `target_index_count` indices. Returns
/// `(simplified_indices, actual_index_count, result_error)`.
///
/// `target_error` is the maximum allowed error (e.g. 0.01 = 1% of mesh scale).
/// Uses [`simplify_scale`] to normalize the error.
pub fn simplify(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
    target_index_count: usize,
    target_error: f32,
) -> (Vec<u32>, usize, f32) {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    let mut out = vec![0u32; indices.len()];
    let mut result_error = 0.0f32;
    let actual = unsafe {
        ffi::meshopt_simplify(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
            target_index_count,
            target_error,
            ffi::meshopt_Simplify_None,
            &mut result_error,
        )
    };
    out.truncate(actual);
    (out, actual, result_error)
}

/// Simplify a mesh with attribute-awareness (e.g., UV preservation).
///
/// `vertex_attributes` is interleaved attribute data (e.g., UVs: [u,v, u,v, ...]).
/// `attribute_weights` controls how much each attribute channel matters
/// (e.g., [1.0, 1.0] for equal weight on u and v).
///
/// The simplifier prefers collapsing vertices with similar attributes,
/// reducing texture distortion at LOD boundaries.
pub fn simplify_with_attributes(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
    vertex_attributes: &[f32],
    vertex_attributes_stride: usize,
    attribute_weights: &[f32],
    target_index_count: usize,
    target_error: f32,
) -> (Vec<u32>, usize, f32) {
    simplify_with_attributes_locked(
        indices, vertex_positions, vertex_positions_stride, vertex_attributes,
        vertex_attributes_stride, attribute_weights, &[], target_index_count, target_error,
    )
}

/// Attribute-aware simplification with optional per-vertex collapse locks.
/// Locks preserve discontinuities such as incompatible skin-joint influence
/// sets while continuous weights and morph deltas participate in error.
pub fn simplify_with_attributes_locked(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
    vertex_attributes: &[f32],
    vertex_attributes_stride: usize,
    attribute_weights: &[f32],
    vertex_lock: &[u8],
    target_index_count: usize,
    target_error: f32,
) -> (Vec<u32>, usize, f32) {
    assert!(vertex_positions_stride > 0 && vertex_positions_stride % 4 == 0);
    assert!(vertex_attributes_stride > 0 && vertex_attributes_stride % 4 == 0);
    let position_components = vertex_positions_stride / 4;
    let attribute_components = vertex_attributes_stride / 4;
    assert!(attribute_components <= 16);
    assert_eq!(vertex_positions.len() % position_components, 0);
    let vertex_count = vertex_positions.len() / position_components;
    assert_eq!(vertex_attributes.len(), vertex_count * attribute_components);
    assert_eq!(attribute_weights.len(), attribute_components);
    assert!(vertex_lock.is_empty() || vertex_lock.len() == vertex_count);
    assert!(indices.iter().all(|&index| (index as usize) < vertex_count));
    let mut out = vec![0u32; indices.len()];
    let mut result_error = 0.0f32;
    let lock = if vertex_lock.is_empty() { std::ptr::null() } else { vertex_lock.as_ptr() };
    let actual = unsafe {
        ffi::meshopt_simplifyWithAttributes(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
            vertex_attributes.as_ptr(),
            vertex_attributes_stride,
            attribute_weights.as_ptr(),
            attribute_weights.len(),
            lock,
            target_index_count,
            target_error,
            ffi::meshopt_Simplify_None,
            &mut result_error,
        )
    };
    out.truncate(actual);
    (out, actual, result_error)
}

/// Fast, less accurate simplification. Good for aggressive LOD reduction.
pub fn simplify_sloppy(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
    target_index_count: usize,
    target_error: f32,
) -> (Vec<u32>, usize, f32) {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    let mut out = vec![0u32; indices.len()];
    let mut result_error = 0.0f32;
    let actual = unsafe {
        ffi::meshopt_simplifySloppy(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
            std::ptr::null(),
            target_index_count,
            target_error,
            &mut result_error,
        )
    };
    out.truncate(actual);
    (out, actual, result_error)
}

/// Compute the scale of the mesh (used to normalize error for `simplify`).
pub fn simplify_scale(vertex_positions: &[f32], vertex_positions_stride: usize) -> f32 {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    unsafe { ffi::meshopt_simplifyScale(vertex_positions.as_ptr(), vertex_count, vertex_positions_stride) }
}

// ========================================================================
// STRIPIFY — triangle list ↔ strip
// ========================================================================

/// Convert a triangle list to a triangle strip. Uses a restart index
/// (typically `0xffffffff`).
pub fn stripify(indices: &[u32], vertex_count: usize, restart_index: u32) -> Vec<u32> {
    let bound = unsafe { ffi::meshopt_stripifyBound(indices.len()) };
    let mut out = vec![0u32; bound];
    let len = unsafe {
        ffi::meshopt_stripify(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_count,
            restart_index,
        )
    };
    out.truncate(len);
    out
}

/// Convert a triangle strip back to a triangle list.
pub fn unstripify(strip: &[u32], restart_index: u32) -> Vec<u32> {
    let bound = unsafe { ffi::meshopt_unstripifyBound(strip.len()) };
    let mut out = vec![0u32; bound];
    let len = unsafe {
        ffi::meshopt_unstripify(out.as_mut_ptr(), strip.as_ptr(), strip.len(), restart_index)
    };
    out.truncate(len);
    out
}

// ========================================================================
// ANALYZE — statistics
// ========================================================================

/// Vertex cache statistics (ACMR, ATVR).
pub fn analyze_vertex_cache(
    indices: &[u32],
    vertex_count: usize,
    cache_size: u32,
    warp_size: u32,
    primgroup_size: u32,
) -> ffi::meshopt_VertexCacheStatistics {
    unsafe {
        ffi::meshopt_analyzeVertexCache(
            indices.as_ptr(),
            indices.len(),
            vertex_count,
            cache_size,
            warp_size,
            primgroup_size,
        )
    }
}

/// Vertex fetch statistics (overfetch ratio).
pub fn analyze_vertex_fetch(indices: &[u32], vertex_count: usize, vertex_size: usize) -> ffi::meshopt_VertexFetchStatistics {
    unsafe {
        ffi::meshopt_analyzeVertexFetch(indices.as_ptr(), indices.len(), vertex_count, vertex_size)
    }
}

/// Overdraw statistics (pixels shaded vs covered).
pub fn analyze_overdraw(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
) -> ffi::meshopt_OverdrawStatistics {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    unsafe {
        ffi::meshopt_analyzeOverdraw(
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
        )
    }
}

// ========================================================================
// MESHLETS — GPU-driven rendering
// ========================================================================

/// Build meshlets from a mesh. Returns `(meshlets, meshlet_vertices, meshlet_triangles)`.
///
/// `max_vertices` is typically 64-256, `max_triangles` is typically 64-512
/// (must be ≤512). `cone_weight` (0.0-1.0) controls backface culling cone quality.
pub fn build_meshlets(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
    max_vertices: usize,
    max_triangles: usize,
    cone_weight: f32,
) -> (Vec<ffi::meshopt_Meshlet>, Vec<u32>, Vec<u8>) {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    let meshlet_bound = unsafe {
        ffi::meshopt_buildMeshletsBound(indices.len(), max_vertices, max_triangles)
    };
    let mut meshlets = vec![ffi::meshopt_Meshlet::default(); meshlet_bound];
    let mut meshlet_vertices = vec![0u32; meshlet_bound * max_vertices];
    let mut meshlet_triangles = vec![0u8; meshlet_bound * max_triangles * 3];
    let count = unsafe {
        ffi::meshopt_buildMeshlets(
            meshlets.as_mut_ptr(),
            meshlet_vertices.as_mut_ptr(),
            meshlet_triangles.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
            max_vertices,
            max_triangles,
            cone_weight,
        )
    };
    meshlets.truncate(count);
    (meshlets, meshlet_vertices, meshlet_triangles)
}

/// Compute bounding sphere + normal cone for a cluster of triangles.
pub fn compute_cluster_bounds(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
) -> ffi::meshopt_Bounds {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    unsafe {
        ffi::meshopt_computeClusterBounds(
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
        )
    }
}

// ========================================================================
// SPATIAL — sort + cluster
// ========================================================================

/// Spatially sort vertices (generates a remap table).
pub fn spatial_sort_remap(
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
) -> Vec<u32> {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    let mut remap = vec![0u32; vertex_count];
    unsafe {
        ffi::meshopt_spatialSortRemap(
            remap.as_mut_ptr(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
        );
    }
    remap
}

/// Spatially sort triangles (reorders the index buffer).
pub fn spatial_sort_triangles(
    indices: &[u32],
    vertex_positions: &[f32],
    vertex_positions_stride: usize,
) -> Vec<u32> {
    let vertex_count = vertex_positions.len() / (vertex_positions_stride / 4);
    let mut out = vec![0u32; indices.len()];
    unsafe {
        ffi::meshopt_spatialSortTriangles(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_positions.as_ptr(),
            vertex_count,
            vertex_positions_stride,
        );
    }
    out
}

// ========================================================================
// QUANTIZE — float precision reduction
// ========================================================================

/// Convert a float to a 16-bit half float.
pub fn quantize_half(v: f32) -> u16 {
    unsafe { ffi::meshopt_quantizeHalf(v) }
}

/// Convert a half float back to f32.
pub fn dequantize_half(h: u16) -> f32 {
    unsafe { ffi::meshopt_dequantizeHalf(h) }
}

/// Quantize a float to N bits of precision (round to nearest).
pub fn quantize_float(v: f32, n: i32) -> f32 {
    unsafe { ffi::meshopt_quantizeFloat(v, n as c_int) }
}

// ========================================================================
// FILTERS — encode/decode vertex attributes
// ========================================================================

/// Decode octahedral-encoded normals back to floats (in-place).
pub fn decode_filter_oct(buffer: &mut [u8], count: usize, stride: usize) {
    unsafe {
        ffi::meshopt_decodeFilterOct(
            buffer.as_mut_ptr() as *mut c_void,
            count,
            stride,
        );
    }
}

/// Decode quaternion-encoded data back to floats (in-place).
pub fn decode_filter_quat(buffer: &mut [u8], count: usize, stride: usize) {
    unsafe {
        ffi::meshopt_decodeFilterQuat(
            buffer.as_mut_ptr() as *mut c_void,
            count,
            stride,
        );
    }
}

/// Decode exponential-encoded data back to floats (in-place).
pub fn decode_filter_exp(buffer: &mut [u8], count: usize, stride: usize) {
    unsafe {
        ffi::meshopt_decodeFilterExp(
            buffer.as_mut_ptr() as *mut c_void,
            count,
            stride,
        );
    }
}

/// Encode normals using octahedral encoding (float → i8 × N bits).
pub fn encode_filter_oct(count: usize, stride: usize, bits: i32, data: &[f32]) -> Vec<u8> {
    let out_size = count * stride;
    let mut out = vec![0u8; out_size];
    unsafe {
        ffi::meshopt_encodeFilterOct(
            out.as_mut_ptr() as *mut c_void,
            count,
            stride,
            bits as c_int,
            data.as_ptr(),
        );
    }
    out
}
