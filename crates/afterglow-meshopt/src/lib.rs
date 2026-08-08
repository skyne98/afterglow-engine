//! # afterglow-meshopt
//!
//! Rust + WASM wrappers for [meshoptimizer](https://github.com/zeux/meshoptimizer)
//! v1.2, exposed as an async `#[rpc]` worker.
//!
//! The worker runs on a background thread (native) or Web Worker (WASM) via
//! the poll model — mesh optimization never blocks the render loop.
//!
//! ## Usage (native)
//!
//! ```ignore
//! use afterglow_meshopt::{MeshoptClient, MeshoptWorker};
//!
//! let (client, _events) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
//! let fut = client.simplify(indices, positions, 12, 1000, 0.01).unwrap();
//! client.poll(); // each frame
//! let result = fut.await.unwrap();
//! ```
//!
//! ## API categories
//!
//! - **Simplify**: LOD generation (`simplify`, `simplify_sloppy`)
//! - **Optimize**: vertex cache, overdraw (`optimize_vertex_cache`, `optimize_overdraw`)
//! - **Compress**: encode/decode index + vertex buffers
//! - **Remap**: deduplicate vertices
//! - **Stripify**: triangle list ↔ strip
//! - **Meshlets**: GPU-driven meshlet building
//! - **Analyze**: vertex cache stats
//! - **Quantize**: float → half precision

#![allow(clippy::missing_safety_doc)]

pub mod ffi;
pub mod safe;
pub mod tests;
pub mod worker_tests;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use safe::*;

use afterglow_rpc::{RpcResult, ServeFuture};
use afterglow_rpc_macros::rpc;

/// Mesh optimization worker. All methods run asynchronously on a background
/// thread/worker — the render loop is never blocked.
#[rpc(worker = MeshoptWorker)]
pub trait Meshopt {
    // --- Simplification (LOD generation) ---

    /// Simplify a mesh to `target_index_count` indices while preserving shape.
    /// `target_error` is the max allowed error (e.g. 0.01 = 1% of mesh scale).
    /// Returns the simplified index buffer.
    async fn simplify(
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        target_index_count: u32,
        target_error: f32,
    ) -> RpcResult<Vec<u32>>;

    /// Fast, less accurate simplification. Good for aggressive LOD reduction.
    async fn simplify_sloppy(
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        target_index_count: u32,
        target_error: f32,
    ) -> RpcResult<Vec<u32>>;

    /// Simplify a mesh with UV-awareness — preserves UV boundaries.
    /// `uv_weight` controls how much UVs matter (0.0 = ignore, 1.0 = equal to position).
    async fn simplify_with_uvs(
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        uvs: Vec<f32>,
        uv_stride: u32,
        uv_weight: f32,
        target_index_count: u32,
        target_error: f32,
    ) -> RpcResult<Vec<u32>>;

    /// Simplify against arbitrary continuous vertex attributes. `vertex_lock`
    /// is empty or one byte per vertex and preserves discrete seams such as
    /// incompatible skin-joint influence sets.
    async fn simplify_with_attributes(
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        attributes: Vec<f32>,
        attribute_stride: u32,
        attribute_weights: Vec<f32>,
        vertex_lock: Vec<u8>,
        target_index_count: u32,
        target_error: f32,
    ) -> RpcResult<Vec<u32>>;

    // --- Optimization ---

    /// Reorder triangles for GPU vertex cache efficiency (FIFO cache).
    async fn optimize_vertex_cache(
        indices: Vec<u32>,
        vertex_count: u32,
    ) -> RpcResult<Vec<u32>>;

    /// Reorder triangles to reduce overdraw. `threshold` is typically 1.05.
    async fn optimize_overdraw(
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        threshold: f32,
    ) -> RpcResult<Vec<u32>>;

    // --- Compression ---

    /// Compress an index buffer into compact bytes.
    async fn encode_index_buffer(
        indices: Vec<u32>,
        vertex_count: u32,
    ) -> RpcResult<Vec<u8>>;

    /// Decompress an index buffer.
    async fn decode_index_buffer(
        buffer: Vec<u8>,
        index_count: u32,
    ) -> RpcResult<Vec<u32>>;

    /// Compress a vertex buffer into compact bytes.
    async fn encode_vertex_buffer(
        vertices: Vec<u8>,
        vertex_size: u32,
    ) -> RpcResult<Vec<u8>>;

    /// Decompress a vertex buffer.
    async fn decode_vertex_buffer(
        buffer: Vec<u8>,
        vertex_count: u32,
        vertex_size: u32,
    ) -> RpcResult<Vec<u8>>;

    // --- Remap (deduplication) ---

    /// Generate a vertex remap table (deduplicates vertices).
    async fn generate_vertex_remap(
        indices: Vec<u32>,
        vertices: Vec<u8>,
        vertex_size: u32,
    ) -> RpcResult<Vec<u32>>;

    // --- Stripify ---

    /// Convert a triangle list to a triangle strip.
    async fn stripify(
        indices: Vec<u32>,
        vertex_count: u32,
        restart_index: u32,
    ) -> RpcResult<Vec<u32>>;

    // --- Meshlets ---

    /// Build meshlets from a mesh (for GPU-driven rendering).
    /// Returns serialized meshlet data as raw bytes.
    async fn build_meshlets(
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        max_vertices: u32,
        max_triangles: u32,
        cone_weight: f32,
    ) -> RpcResult<Vec<u8>>;

    // --- Analysis ---

    /// Analyze vertex cache efficiency. Returns [acmr, atvr, transformed_vertices, misspelled_vertices].
    async fn analyze_vertex_cache(
        indices: Vec<u32>,
        vertex_count: u32,
    ) -> RpcResult<Vec<f32>>;

    // --- Quantization ---

    /// Quantize a float to 16-bit half precision.
    async fn quantize_half(value: f32) -> RpcResult<u16>;
}

/// The concrete worker implementation. Stateless — all operations take
/// input data and return results.
#[derive(Default)]
pub struct MeshoptWorker;

impl MeshoptServer for MeshoptWorker {
    fn simplify(
        &self,
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        target_index_count: u32,
        target_error: f32,
    ) -> ServeFuture {
        Box::pin(async move {
            let (simplified, _, _) = safe::simplify(
                &indices,
                &positions,
                position_stride as usize,
                target_index_count as usize,
                target_error,
            );
            afterglow_rpc::encode(&simplified)
        })
    }

    fn simplify_sloppy(
        &self,
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        target_index_count: u32,
        target_error: f32,
    ) -> ServeFuture {
        Box::pin(async move {
            let (simplified, _, _) = safe::simplify_sloppy(
                &indices,
                &positions,
                position_stride as usize,
                target_index_count as usize,
                target_error,
            );
            afterglow_rpc::encode(&simplified)
        })
    }

    fn simplify_with_uvs(
        &self,
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        uvs: Vec<f32>,
        uv_stride: u32,
        uv_weight: f32,
        target_index_count: u32,
        target_error: f32,
    ) -> ServeFuture {
        Box::pin(async move {
            let weights = vec![uv_weight; (uv_stride / 4) as usize];
            let (simplified, _, _) = safe::simplify_with_attributes(
                &indices,
                &positions,
                position_stride as usize,
                &uvs,
                uv_stride as usize,
                &weights,
                target_index_count as usize,
                target_error,
            );
            afterglow_rpc::encode(&simplified)
        })
    }

    fn simplify_with_attributes(
        &self,
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        attributes: Vec<f32>,
        attribute_stride: u32,
        attribute_weights: Vec<f32>,
        vertex_lock: Vec<u8>,
        target_index_count: u32,
        target_error: f32,
    ) -> ServeFuture {
        Box::pin(async move {
            if position_stride == 0 || position_stride % 4 != 0 ||
                attribute_stride == 0 || attribute_stride % 4 != 0
            {
                return Err(afterglow_rpc::RpcError::Server(
                    "position and attribute strides must be positive multiples of four".into(),
                ));
            }
            let position_components = position_stride as usize / 4;
            let attribute_components = attribute_stride as usize / 4;
            let vertex_count = positions.len() / position_components;
            if positions.len() % position_components != 0 || attribute_components > 16 ||
                attributes.len() != vertex_count * attribute_components ||
                attribute_weights.len() != attribute_components ||
                (!vertex_lock.is_empty() && vertex_lock.len() != vertex_count) ||
                indices.len() % 3 != 0 || indices.iter().any(|&index| index as usize >= vertex_count) ||
                target_index_count == 0 || target_index_count as usize > indices.len()
            {
                return Err(afterglow_rpc::RpcError::Server(
                    "attribute-aware simplification inputs are invalid or inconsistent".into(),
                ));
            }
            let (simplified, _, _) = safe::simplify_with_attributes_locked(
                &indices,
                &positions,
                position_stride as usize,
                &attributes,
                attribute_stride as usize,
                &attribute_weights,
                &vertex_lock,
                target_index_count as usize,
                target_error,
            );
            afterglow_rpc::encode(&simplified)
        })
    }

    fn optimize_vertex_cache(
        &self,
        indices: Vec<u32>,
        vertex_count: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let optimized = safe::optimize_vertex_cache(&indices, vertex_count as usize);
            afterglow_rpc::encode(&optimized)
        })
    }

    fn optimize_overdraw(
        &self,
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        threshold: f32,
    ) -> ServeFuture {
        Box::pin(async move {
            let optimized = safe::optimize_overdraw(
                &indices,
                &positions,
                position_stride as usize,
                threshold,
            );
            afterglow_rpc::encode(&optimized)
        })
    }

    fn encode_index_buffer(
        &self,
        indices: Vec<u32>,
        vertex_count: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let encoded = safe::encode_index_buffer(&indices, vertex_count as usize);
            afterglow_rpc::encode(&encoded)
        })
    }

    fn decode_index_buffer(
        &self,
        buffer: Vec<u8>,
        index_count: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let decoded = safe::decode_index_buffer(&buffer, index_count as usize);
            afterglow_rpc::encode(&decoded)
        })
    }

    fn encode_vertex_buffer(
        &self,
        vertices: Vec<u8>,
        vertex_size: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let encoded = safe::encode_vertex_buffer(&vertices, vertex_size as usize);
            afterglow_rpc::encode(&encoded)
        })
    }

    fn decode_vertex_buffer(
        &self,
        buffer: Vec<u8>,
        vertex_count: u32,
        vertex_size: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let decoded = safe::decode_vertex_buffer(&buffer, vertex_count as usize, vertex_size as usize);
            afterglow_rpc::encode(&decoded)
        })
    }

    fn generate_vertex_remap(
        &self,
        indices: Vec<u32>,
        vertices: Vec<u8>,
        vertex_size: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let (remap, _) = safe::generate_vertex_remap(&indices, &vertices, vertex_size as usize);
            afterglow_rpc::encode(&remap)
        })
    }

    fn stripify(
        &self,
        indices: Vec<u32>,
        vertex_count: u32,
        restart_index: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let strip = safe::stripify(&indices, vertex_count as usize, restart_index);
            afterglow_rpc::encode(&strip)
        })
    }

    fn build_meshlets(
        &self,
        indices: Vec<u32>,
        positions: Vec<f32>,
        position_stride: u32,
        max_vertices: u32,
        max_triangles: u32,
        cone_weight: f32,
    ) -> ServeFuture {
        Box::pin(async move {
            let (meshlets, meshlet_vertices, meshlet_triangles) = safe::build_meshlets(
                &indices,
                &positions,
                position_stride as usize,
                max_vertices as usize,
                max_triangles as usize,
                cone_weight,
            );
            // Serialize: [meshlet_count][meshlets...][vertices...][triangles...]
            let mut out = Vec::new();
            out.extend_from_slice(&(meshlets.len() as u32).to_le_bytes());
            for m in &meshlets {
                out.extend_from_slice(&m.vertex_offset.to_le_bytes());
                out.extend_from_slice(&m.triangle_offset.to_le_bytes());
                out.extend_from_slice(&m.vertex_count.to_le_bytes());
                out.extend_from_slice(&m.triangle_count.to_le_bytes());
            }
            out.extend_from_slice(bytemuck::cast_slice::<u32, u8>(&meshlet_vertices));
            out.extend_from_slice(&meshlet_triangles);
            afterglow_rpc::encode(&out)
        })
    }

    fn analyze_vertex_cache(
        &self,
        indices: Vec<u32>,
        vertex_count: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let stats = safe::analyze_vertex_cache(
                &indices,
                vertex_count as usize,
                16, // standard GPU vertex cache size
                32, // warp size
                32, // primgroup size
            );
            let result = vec![
                stats.acmr,
                stats.atvr,
                stats.transformed_vertices as f32,
                stats.misspelled_vertices as f32,
            ];
            afterglow_rpc::encode(&result)
        })
    }

    fn quantize_half(&self, value: f32) -> ServeFuture {
        Box::pin(async move {
            let h = safe::quantize_half(value);
            afterglow_rpc::encode(&h)
        })
    }
}
