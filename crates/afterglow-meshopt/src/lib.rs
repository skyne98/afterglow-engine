//! # afterglow-meshopt
//!
//! Safe Rust wrappers for [meshoptimizer](https://github.com/zeux/meshoptimizer)
//! v1.2 — mesh optimization, LOD simplification, compression, meshlets,
//! stripification, analysis, and quantization.
//!
//! Compiles to both native and `wasm32-unknown-unknown`. The C++ source is
//! vendored in `vendor/src/` and compiled via `cc` in `build.rs`.
//!
//! ## Categories
//!
//! - **Remap**: deduplicate vertices, generate remap tables
//! - **Optimize**: vertex cache, overdraw, vertex fetch
//! - **Encode/Decode**: compress index + vertex buffers
//! - **Filters**: octahedral, quaternion, exponential, color
//! - **Simplify**: LOD generation with error control
//! - **Stripify**: triangle list ↔ strip conversion
//! - **Analyze**: vertex cache, fetch, overdraw statistics
//! - **Meshlets**: GPU-driven meshlet building + bounds
//! - **Spatial**: spatial sort remap, triangles, point clustering
//! - **Quantize**: float ↔ half, N-bit quantization

#![allow(clippy::missing_safety_doc)]

pub mod ffi;
pub mod safe;
pub mod tests;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use safe::*;
