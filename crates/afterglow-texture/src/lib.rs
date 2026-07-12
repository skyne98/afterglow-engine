//! # afterglow-texture
//!
//! Basis Universal texture transcoder + mip generation.
//!
//! - **Transcoder**: decodes Basis Universal / KTX2 textures to GPU-native
//!   formats (BC7, ASTC, ETC2, etc.) at load time. Single .cpp, no deps.
//! - **Mip generation**: box-filter downscaling (no deps, works in WASM).
//!
//! Compiles to both native and `wasm32-unknown-unknown`.

pub mod ffi;
pub mod safe;
pub mod mips;

pub use safe::*;
pub use mips::*;

use afterglow_rpc::{RpcResult, ServeFuture};
use afterglow_rpc_macros::rpc;

/// Texture transcode worker. All methods run async on a background thread.
#[rpc(worker = TextureWorker)]
pub trait Texture {
    /// Transcode a Basis/KTX2 texture to a GPU-native format.
    /// `target_format` is a transcoder_texture_format constant (e.g. 6 = BC7).
    /// Returns the transcoded GPU-compressed texture data.
    async fn transcode(
        data: Vec<u8>,
        target_format: u32,
    ) -> RpcResult<Vec<u8>>;

    /// Generate a mip chain from raw RGBA data.
    /// Returns serialized mips: [mip_count(u32)][mip0_w(u32)][mip0_h(u32)][mip0_data...][mip1...]...
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
            // Serialize: [count][w0][h0][data0...][w1][h1][data1...]...
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
