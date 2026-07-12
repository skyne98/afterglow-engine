//! Fetch bridge for the wasm asset worker.
//!
//! On web, the worker can't do I/O directly — it needs JS to `fetch`. This
//! module provides the async functions that the worker's `AssetLoaderServer`
//! calls:
//!
//! - `fetch_asset(path)` — full GET, returns all bytes.
//! - `fetch_size(path)` — HEAD, returns Content-Length.
//! - `fetch_range(path, offset, len)` — ranged GET, returns the bytes.
//!
//! All use JS-imported functions (`ag_fetch_start`, `ag_fetch_head_start`,
//! `ag_fetch_range_start`, `ag_fetch_poll`) implemented in `async-worker.js`.
//! The worker's async task polls via `yield_now()` until the fetch resolves.

#![cfg(target_arch = "wasm32")]

use afterglow_rpc::{RpcError, RpcResult, encode};
use afterglow_rpc::wasm::Scratch;

const FETCH_SCRATCH_SIZE: usize = 1 << 20; // 1 MiB

static FETCH_SCRATCH: Scratch<FETCH_SCRATCH_SIZE> = Scratch::new();

/// JS-provided imports.
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Start a full GET fetch for `url`. Returns a `fetch_id` (>0) or 0 on error.
    fn ag_fetch_start(url_ptr: *const u8, url_len: usize) -> u32;
    /// Start a HEAD fetch for `url` (to get Content-Length). Returns a `fetch_id`.
    fn ag_fetch_head_start(url_ptr: *const u8, url_len: usize) -> u32;
    /// Poll a HEAD fetch. Writes the Content-Length as a `u64` to `out` (8 bytes).
    /// Returns: `-1` pending, `8` complete (bytes written), `-2` error.
    fn ag_fetch_head_poll(fetch_id: u32, out_ptr: *mut u8, out_max: usize) -> i32;
    /// Start a ranged GET fetch for `url` (`Range: bytes=offset-(offset+len-1)`).
    /// Returns a `fetch_id`.
    fn ag_fetch_range_start(url_ptr: *const u8, url_len: usize, offset: u64, len: u32) -> u32;
    /// Poll a fetch by `fetch_id`. Writes result bytes to `out` if complete.
    /// Returns: `-1` pending, `>=0` byte count (complete), `-2` out too small.
    fn ag_fetch_poll(fetch_id: u32, out_ptr: *mut u8, out_max: usize) -> i32;
}

/// Fetch an asset by URL path via JS. Returns the postcard-encoded bytes.
pub async fn fetch_asset(path: &str) -> RpcResult<Vec<u8>> {
    let url = path.as_bytes();
    let fetch_id = unsafe { ag_fetch_start(url.as_ptr(), url.len()) };
    if fetch_id == 0 {
        return Err(RpcError::Server(format!("fetch start failed: {path}")));
    }
    let bytes = poll_fetch_body(fetch_id, path).await?;
    encode(&bytes)
}

/// Fetch the size of an asset via a HEAD request. Returns the postcard-encoded `u64`.
pub async fn fetch_size(path: &str) -> RpcResult<Vec<u8>> {
    let url = path.as_bytes();
    let fetch_id = unsafe { ag_fetch_head_start(url.as_ptr(), url.len()) };
    if fetch_id == 0 {
        return Err(RpcError::Server(format!("HEAD start failed: {path}")));
    }
    let mut buf = [0u8; 8];
    loop {
        let r = unsafe { ag_fetch_head_poll(fetch_id, buf.as_mut_ptr(), 8) };
        if r == 8 {
            let size = u64::from_le_bytes(buf);
            return encode(&size);
        }
        if r == -2 {
            return Err(RpcError::Server(format!("HEAD failed: {path}")));
        }
        // r == -1: pending. Yield.
        futures_lite::future::yield_now().await;
    }
}

/// Fetch a range of an asset via a ranged GET. Returns the postcard-encoded bytes.
pub async fn fetch_range(path: &str, offset: u64, len: u32) -> RpcResult<Vec<u8>> {
    let url = path.as_bytes();
    let fetch_id = unsafe { ag_fetch_range_start(url.as_ptr(), url.len(), offset, len) };
    if fetch_id == 0 {
        return Err(RpcError::Server(format!("range fetch start failed: {path}")));
    }
    let bytes = poll_fetch_body(fetch_id, path).await?;
    encode(&bytes)
}

/// Poll a fetch's body until complete. Returns the raw bytes.
async fn poll_fetch_body(fetch_id: u32, path: &str) -> RpcResult<Vec<u8>> {
    let scratch_ptr = FETCH_SCRATCH.ptr() as *mut u8;
    let scratch_len = FETCH_SCRATCH.size();
    loop {
        let r = unsafe { ag_fetch_poll(fetch_id, scratch_ptr, scratch_len) };
        if r >= 0 {
            let n = r as usize;
            let bytes = unsafe { std::slice::from_raw_parts(scratch_ptr, n).to_vec() };
            return Ok(bytes);
        }
        if r == -2 {
            return Err(RpcError::Server(format!(
                "asset too large for fetch scratch (1 MiB): {path}"
            )));
        }
        // r == -1: pending. Yield.
        futures_lite::future::yield_now().await;
    }
}
