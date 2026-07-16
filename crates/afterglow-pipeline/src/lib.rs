//! # afterglow-pipeline
//!
//! Offline asset pipeline — preprocess, compress, and package assets for
//! streaming in the `.big` format (Generals-inspired).

pub mod format;
pub mod gltf;
pub mod height;
pub mod mesh;
pub mod texture;

pub use format::*;
pub use gltf::*;
pub use height::*;
pub use mesh::*;
pub use texture::*;
