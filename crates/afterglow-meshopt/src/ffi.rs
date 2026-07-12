// FFI declarations — raw bindings to the meshoptimizer C API.
//
// All functions are `extern "C"` and match the signatures in meshoptimizer.h
// (version 1.2). These are unsafe — the safe wrappers in `safe.rs` provide
// the public API.

#![allow(non_camel_case_types, non_snake_case)]

use std::os::raw::{c_int, c_uchar, c_uint, c_void};

// --- structs (from meshoptimizer.h) --------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_Stream {
    pub data: *const c_void,
    pub size: usize,
    pub stride: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_VertexCacheStatistics {
    pub transformed_vertices: c_uint,
    pub misspelled_vertices: c_uint, // typo in upstream; kept for ABI compat
    pub acmr: f32,
    pub atvr: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_VertexFetchStatistics {
    pub overfetch: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_OverdrawStatistics {
    pub pixels_shaded: f32,
    pub pixels_covered: f32,
    pub overdraw: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_CoverageStatistics {
    pub coverage: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_Meshlet {
    pub vertex_offset: c_uint,
    pub triangle_offset: c_uint,
    pub vertex_count: c_uint,
    pub triangle_count: c_uint,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct meshopt_Bounds {
    pub center: [f32; 3],
    pub radius: f32,
    pub normal: [f32; 3],
    pub cone_apex: [f32; 3],
    pub cone_axis: [f32; 3],
    pub cone_cutoff: f32,
    pub cone_error_spherical: c_int,
    pub bounds_error_spherical: c_int,
}

// --- options enum ---------------------------------------------------------

pub const meshopt_Simplify_None: c_uint = 0;
pub const meshopt_Simplify_LockBorder: c_uint = 1;
pub const meshopt_Simplify_Sparse: c_uint = 2;
pub const meshopt_Simplify_ErrorAbsolute: c_uint = 4;
pub const meshopt_Simplify_Prune: c_uint = 8;
pub const meshopt_Simplify_Permissive: c_uint = 16;

// --- encode exp mode enum ------------------------------------------------

pub const meshopt_EncodeExpMode_Cluster: c_int = 0;
pub const meshopt_EncodeExpMode_Component: c_int = 1;
pub const meshopt_EncodeExpMode_Group: c_int = 2;
pub const meshopt_EncodeExpMode_Device: c_int = 3;

// --- FFI function declarations -------------------------------------------

unsafe extern "C" {
    // Vertex remap
    pub fn meshopt_generateVertexRemap(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
    ) -> usize;

    pub fn meshopt_generateVertexRemapMulti(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        streams: *const meshopt_Stream,
        stream_count: usize,
    ) -> usize;

    pub fn meshopt_remapVertexBuffer(
        destination: *mut c_void,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
        remap: *const c_uint,
    );

    pub fn meshopt_remapIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        remap: *const c_uint,
    );

    // Shadow index buffer
    pub fn meshopt_generateShadowIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
        vertex_stride: usize,
    );

    pub fn meshopt_generateShadowIndexBufferMulti(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        streams: *const meshopt_Stream,
        stream_count: usize,
    );

    pub fn meshopt_generatePositionRemap(
        destination: *mut c_uint,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    );

    // Adjacency / tessellation
    pub fn meshopt_generateAdjacencyIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    );

    pub fn meshopt_generateTessellationIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    );

    // Optimization
    pub fn meshopt_optimizeVertexCache(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    );

    pub fn meshopt_optimizeVertexCacheStrip(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    );

    pub fn meshopt_optimizeVertexCacheFifo(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        cache_size: c_uint,
    );

    pub fn meshopt_optimizeOverdraw(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
        threshold: f32,
    );

    pub fn meshopt_optimizeVertexFetch(
        destination: *mut c_void,
        indices: *mut c_uint,
        index_count: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
    ) -> usize;

    pub fn meshopt_optimizeVertexFetchRemap(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    ) -> usize;

    // Index encoding
    pub fn meshopt_encodeIndexBuffer(
        buffer: *mut c_uchar,
        buffer_size: usize,
        indices: *const c_uint,
        index_count: usize,
    ) -> usize;

    pub fn meshopt_encodeIndexBufferBound(index_count: usize, vertex_count: usize) -> usize;

    pub fn meshopt_encodeIndexVersion(version: c_int);

    pub fn meshopt_decodeIndexBuffer(
        destination: *mut c_void,
        index_count: usize,
        index_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;

    // Index sequence encoding
    pub fn meshopt_encodeIndexSequence(
        buffer: *mut c_uchar,
        buffer_size: usize,
        indices: *const c_uint,
        index_count: usize,
    ) -> usize;

    pub fn meshopt_encodeIndexSequenceBound(index_count: usize, vertex_count: usize) -> usize;

    pub fn meshopt_decodeIndexSequence(
        destination: *mut c_void,
        index_count: usize,
        index_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;

    // Vertex encoding
    pub fn meshopt_encodeVertexBuffer(
        buffer: *mut c_uchar,
        buffer_size: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
    ) -> usize;

    pub fn meshopt_encodeVertexBufferBound(vertex_count: usize, vertex_size: usize) -> usize;

    pub fn meshopt_encodeVertexBufferLevel(
        buffer: *mut c_uchar,
        buffer_size: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
        level: c_int,
        version: c_int,
    ) -> usize;

    pub fn meshopt_encodeVertexVersion(version: c_int);

    pub fn meshopt_decodeVertexBuffer(
        destination: *mut c_void,
        vertex_count: usize,
        vertex_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;

    // Vertex filters
    pub fn meshopt_decodeFilterOct(buffer: *mut c_void, count: usize, stride: usize);
    pub fn meshopt_decodeFilterQuat(buffer: *mut c_void, count: usize, stride: usize);
    pub fn meshopt_decodeFilterExp(buffer: *mut c_void, count: usize, stride: usize);
    pub fn meshopt_decodeFilterColor(buffer: *mut c_void, count: usize, stride: usize);

    pub fn meshopt_encodeFilterOct(
        destination: *mut c_void, count: usize, stride: usize, bits: c_int, data: *const f32,
    );
    pub fn meshopt_encodeFilterQuat(
        destination: *mut c_void, count: usize, stride: usize, bits: c_int, data: *const f32,
    );
    pub fn meshopt_encodeFilterExp(
        destination: *mut c_void, count: usize, stride: usize, bits: c_int, data: *const f32,
        mode: c_int,
    );
    pub fn meshopt_encodeFilterColor(
        destination: *mut c_void, count: usize, stride: usize, bits: c_int, data: *const f32,
    );

    // Simplification
    pub fn meshopt_simplify(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
        target_index_count: usize,
        target_error: f32,
        options: c_uint,
        result_error: *mut f32,
    ) -> usize;

    pub fn meshopt_simplifyWithAttributes(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_attributes: *const f32,
        vertex_attributes_stride: usize,
        attribute_weights: *const f32,
        attribute_count: usize,
        vertex_lock: *const c_uchar,
        target_index_count: usize,
        target_error: f32,
        options: c_uint,
        result_error: *mut f32,
    ) -> usize;

    pub fn meshopt_simplifySloppy(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_lock: *const c_uchar,
        target_index_count: usize,
        target_error: f32,
        result_error: *mut f32,
    ) -> usize;

    pub fn meshopt_simplifyScale(
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    ) -> f32;

    // Stripification
    pub fn meshopt_stripify(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        restart_index: c_uint,
    ) -> usize;

    pub fn meshopt_stripifyBound(index_count: usize) -> usize;

    pub fn meshopt_unstripify(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        restart_index: c_uint,
    ) -> usize;

    pub fn meshopt_unstripifyBound(index_count: usize) -> usize;

    // Analysis
    pub fn meshopt_analyzeVertexCache(
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        cache_size: c_uint,
        warp_size: c_uint,
        primgroup_size: c_uint,
    ) -> meshopt_VertexCacheStatistics;

    #[cfg(not(target_arch = "wasm32"))]
    pub fn meshopt_analyzeVertexFetch(
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        vertex_size: usize,
    ) -> meshopt_VertexFetchStatistics;

    // On WASM, 4-byte structs are returned as scalars, not via hidden pointer.
    #[cfg(target_arch = "wasm32")]
    pub fn meshopt_analyzeVertexFetch(
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        vertex_size: usize,
    ) -> f32;

    pub fn meshopt_analyzeOverdraw(
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    ) -> meshopt_OverdrawStatistics;

    // Meshlets
    pub fn meshopt_buildMeshlets(
        meshlets: *mut meshopt_Meshlet,
        meshlet_vertices: *mut c_uint,
        meshlet_triangles: *mut c_uchar,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
        max_vertices: usize,
        max_triangles: usize,
        cone_weight: f32,
    ) -> usize;

    pub fn meshopt_buildMeshletsBound(
        index_count: usize,
        max_vertices: usize,
        max_triangles: usize,
    ) -> usize;

    pub fn meshopt_optimizeMeshlet(
        meshlet_vertices: *mut c_uint,
        meshlet_triangles: *mut c_uchar,
        triangle_count: usize,
        vertex_count: usize,
    );

    pub fn meshopt_optimizeMeshletLevel(
        meshlet_vertices: *mut c_uint,
        vertex_count: usize,
        meshlet_triangles: *mut c_uchar,
        triangle_count: usize,
        level: c_int,
    );

    pub fn meshopt_computeClusterBounds(
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    ) -> meshopt_Bounds;

    pub fn meshopt_computeMeshletBounds(
        meshlet_vertices: *const c_uint,
        meshlet_triangles: *const c_uchar,
        triangle_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    ) -> meshopt_Bounds;

    pub fn meshopt_computeSphereBounds(
        positions: *const f32,
        count: usize,
        positions_stride: usize,
        radii: *const f32,
        radii_stride: usize,
    ) -> meshopt_Bounds;

    // Spatial
    pub fn meshopt_spatialSortRemap(
        destination: *mut c_uint,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    );

    pub fn meshopt_spatialSortTriangles(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
    );

    pub fn meshopt_spatialClusterPoints(
        destination: *mut c_uint,
        vertex_positions: *const f32,
        vertex_count: usize,
        vertex_positions_stride: usize,
        cluster_size: usize,
    );

    // Meshlet encoding
    pub fn meshopt_encodeMeshlet(
        buffer: *mut c_uchar,
        buffer_size: usize,
        vertices: *const c_uint,
        vertex_count: usize,
        triangles: *const c_uchar,
        triangle_count: usize,
    ) -> usize;

    pub fn meshopt_encodeMeshletBound(max_vertices: usize, max_triangles: usize) -> usize;

    pub fn meshopt_decodeMeshlet(
        vertices: *mut c_void,
        vertex_count: usize,
        vertex_size: usize,
        triangles: *mut c_void,
        triangle_count: usize,
        triangle_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;

    // Quantization
    pub fn meshopt_quantizeHalf(v: f32) -> u16;
    pub fn meshopt_quantizeFloat(v: f32, n: c_int) -> f32;
    pub fn meshopt_dequantizeHalf(h: u16) -> f32;

    // Allocator
    pub fn meshopt_setAllocator(
        allocate: Option<extern "C" fn(usize) -> *mut c_void>,
        deallocate: Option<extern "C" fn(*mut c_void)>,
    );
}
