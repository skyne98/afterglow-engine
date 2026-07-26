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

pub mod mips;
pub mod safe;
pub mod worker_tests;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use mips::*;
pub use safe::*;

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
    async fn transcode(data: Vec<u8>, target_format: u32) -> RpcResult<Vec<u8>>;

    /// Generate a mip chain from raw RGBA data.
    /// Returns serialized mips: [count][w0][h0][len0][data0...][w1]...
    async fn generate_mips(data: Vec<u8>, width: u32, height: u32) -> RpcResult<Vec<u8>>;

    /// Downscale raw RGBA to target dimensions (box filter).
    async fn downscale(
        data: Vec<u8>,
        width: u32,
        height: u32,
        target_width: u32,
        target_height: u32,
    ) -> RpcResult<Vec<u8>>;

    /// Retain one confined native source and return its generational handle.
    /// Bootstrap calls this once per worker; web workers use `transcode(data)`.
    async fn open_source(path: String) -> RpcResult<u32>;

    /// Read one encoded texture range and transcode it without exposing source
    /// bytes to JavaScript. Native callers use a handle returned by
    /// `open_source`; the wasm service intentionally rejects this method.
    async fn transcode_range(
        source: u32,
        offset: u64,
        len: u32,
        target_format: u32,
    ) -> RpcResult<Vec<u8>>;
}

#[cfg(not(target_arch = "wasm32"))]
const TEXTURE_SOURCE_CAPACITY: usize = 16;
#[cfg(not(target_arch = "wasm32"))]
const TEXTURE_INPUT_BYTES: usize = 4 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
static TEXTURE_SOURCE_PROVIDER: std::sync::OnceLock<
    std::sync::Arc<afterglow_assets::AssetSourceCache>,
> = std::sync::OnceLock::new();

/// Concrete worker implementation. Native instances retain a fixed table of
/// confined sources; encoded page bytes stay inside this OS worker.
pub struct TextureWorker {
    #[cfg(not(target_arch = "wasm32"))]
    source_provider: Option<std::sync::Arc<afterglow_assets::AssetSourceCache>>,
    #[cfg(not(target_arch = "wasm32"))]
    sources: std::sync::Arc<std::sync::Mutex<afterglow_assets::AssetSourceTable>>,
    #[cfg(not(target_arch = "wasm32"))]
    input: std::sync::Arc<std::sync::Mutex<Box<[u8]>>>,
}

impl TextureWorker {
    /// Configure the process-wide confined source provider before spawning
    /// native texture workers. Repeated calls retain the first root.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_asset_root(root: afterglow_assets::AssetRoot) {
        let _ = TEXTURE_SOURCE_PROVIDER.set(std::sync::Arc::new(
            afterglow_assets::AssetSourceCache::new(root),
        ));
    }
}

impl Default for TextureWorker {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            source_provider: TEXTURE_SOURCE_PROVIDER.get().cloned(),
            #[cfg(not(target_arch = "wasm32"))]
            sources: std::sync::Arc::new(std::sync::Mutex::new(
                afterglow_assets::AssetSourceTable::new(TEXTURE_SOURCE_CAPACITY),
            )),
            #[cfg(not(target_arch = "wasm32"))]
            input: std::sync::Arc::new(std::sync::Mutex::new(
                vec![0; TEXTURE_INPUT_BYTES].into_boxed_slice(),
            )),
        }
    }
}

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

    fn downscale(
        &self,
        data: Vec<u8>,
        width: u32,
        height: u32,
        target_width: u32,
        target_height: u32,
    ) -> ServeFuture {
        Box::pin(async move {
            let result = mips::downscale_box(&data, width, height, target_width, target_height);
            afterglow_rpc::encode(&result)
        })
    }

    fn open_source(&self, path: String) -> ServeFuture {
        #[cfg(not(target_arch = "wasm32"))]
        let provider = self.source_provider.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let sources = self.sources.clone();
        Box::pin(async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let provider = provider.ok_or_else(|| {
                    afterglow_rpc::RpcError::Server("texture worker has no asset root".into())
                })?;
                let handle = sources
                    .lock()
                    .map_err(|_| {
                        afterglow_rpc::RpcError::Server("texture source table poisoned".into())
                    })?
                    .open(provider.as_ref(), &path)
                    .ok_or_else(|| {
                        afterglow_rpc::RpcError::Server(format!(
                            "texture source missing or source capacity exceeded: {path}",
                        ))
                    })?;
                afterglow_rpc::encode(&handle.into_raw())
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = path;
                Err(afterglow_rpc::RpcError::Server(
                    "source-backed texture transcoding is native-only".into(),
                ))
            }
        })
    }

    fn transcode_range(
        &self,
        source: u32,
        offset: u64,
        len: u32,
        target_format: u32,
    ) -> ServeFuture {
        #[cfg(not(target_arch = "wasm32"))]
        let sources = self.sources.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let input = self.input.clone();
        Box::pin(async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut data = input.lock().map_err(|_| {
                    afterglow_rpc::RpcError::Server("texture input scratch poisoned".into())
                })?;
                let len = len as usize;
                if len > data.len() {
                    return Err(afterglow_rpc::RpcError::Server(format!(
                        "texture source range {len} exceeds input capacity {}",
                        data.len(),
                    )));
                }
                let mut read = 0usize;
                while read < len {
                    let count = sources
                        .lock()
                        .map_err(|_| {
                            afterglow_rpc::RpcError::Server("texture source table poisoned".into())
                        })?
                        .read_at(
                            afterglow_assets::AssetSourceHandle::from_raw(source),
                            offset + read as u64,
                            &mut data[read..len],
                        )
                        .ok_or_else(|| {
                            afterglow_rpc::RpcError::Server(
                                "stale or invalid texture source handle".into(),
                            )
                        })?
                        .map_err(|error| afterglow_rpc::RpcError::Server(error.to_string()))?;
                    if count == 0 {
                        return Err(afterglow_rpc::RpcError::Server(format!(
                            "texture source range truncated: requested {len}, read {read}",
                        )));
                    }
                    read += count;
                }
                let result = safe::transcode(&data[..len], target_format)
                    .map_err(afterglow_rpc::RpcError::Server)?;
                afterglow_rpc::encode(&result)
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (source, offset, len, target_format);
                Err(afterglow_rpc::RpcError::Server(
                    "source-backed texture transcoding is native-only".into(),
                ))
            }
        })
    }
}
