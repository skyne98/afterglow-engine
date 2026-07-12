//! # afterglow-pipeline
//!
//! Offline asset pipeline — preprocess, compress, and package assets for
//! streaming in the `.big` format (Generals-inspired).

pub mod format;
pub mod texture;
pub mod mesh;

pub use format::*;
pub use texture::*;
pub use mesh::*;
