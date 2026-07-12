//! # afterglow-texture
//!
//! Basis Universal texture transcoder + mip generation — pure Rust, no C++.
//!
//! Uses `basisu_rs` (pure Rust, `#![no_std]`, `#![forbid(unsafe_code)]`) for
//! transcoding Basis/KTX2 textures to GPU-native formats:
//! - BC7 (desktop)
//! - ASTC (mobile)
//! - ETC1/ETC2 (mobile)
//! - RGBA (uncompressed fallback)
//!
//! Box-filter mip generation (pure Rust, no deps).
//!
//! Compiles to both native and `wasm32-unknown-unknown` — no C++, no
//! Emscripten, no pre-built WASM. Pure Rust.

pub mod safe;
pub mod mips;

pub use safe::*;
pub use mips::*;

use afterglow_rpc::{RpcResult, ServeFuture};
use afterglow_rpc_macros::rpc;

/// Target GPU format for transcoding.
pub const FORMAT_BC7: u32 = 0;
pub const FORMAT_ASTC: u32 = 1;
pub const FORMAT_ETC1: u32 = 2;
pub const FORMAT_ETC2: u32 = 3;
pub const FORMAT_RGBA: u32 = 4;

/// Texture transcode worker. All methods run async on a background thread.
#[rpc(worker = TextureWorker)]
pub trait Texture {
    /// Transcode a Basis texture to a GPU-native format.
    /// `target_format` is one of the FORMAT_* constants.
    /// Returns the transcoded GPU-compressed texture data.
    async fn transcode(
        data: Vec<u8>,
        target_format: u32,
    ) -> RpcResult<Vec<u8>>;

    /// Generate a mip chain from raw RGBA data.
    /// Returns serialized mips: [count][w0][h0][len0][data0...][w1]...
    async fn generate_mips(
        data: Vec<u8>,
        width: u32,
        height: u32,
    ) -> RpcResult<Vec<u8>>;

    /// Downscale raw RGBA to target dimensions (box filter).
    async fn downscale(
        data: Vec<u8>,
        width: u32,
        height: u32,
        target_width: u32,
        target_height: u32,
    ) -> RpcResult<Vec<u8>>;
}

/// Concrete worker implementation.
#[derive(Default)]
pub struct TextureWorker;

impl TextureServer for TextureWorker {
    fn transcode(&self, data: Vec<u8>, target_format: u32) -> ServeFuture {
        Box::pin(async move {
            let result = safe::transcode(&data, target_format)
                .map_err(|e| afterglow_rpc::RpcError::Server(e))?;
            afterglow_rpc::encode(&result)
        })
    }

    fn generate_mips(&self, data: Vec<u8>, width: u32, height: u32) -> ServeFuture {
        Box::pin(async move {
            let mips = mips::generate_mip_chain(&data, width, height);
            let mut out = Vec::new();
            out.extend_from_slice(&(mips.len() as u32).to_le_bytes());
            for (w, h, mip_data) in &mips {
                out.extend_from_slice(&w.to_le_bytes());
                out.extend_from_slice(&h.to_le_bytes());
                out.extend_from_slice(&(mip_data.len() as u32).to_le_bytes());
                out.extend_from_slice(mip_data);
            }
            afterglow_rpc::encode(&out)
        })
    }

    fn downscale(&self, data: Vec<u8>, width: u32, height: u32, target_width: u32, target_height: u32) -> ServeFuture {
        Box::pin(async move {
            let result = mips::downscale_box(&data, width, height, target_width, target_height);
            afterglow_rpc::encode(&result)
        })
    }
}
