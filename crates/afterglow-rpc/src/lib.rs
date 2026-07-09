//! # afterglow-rpc
//!
//! Ultra-light RPC runtime for main<->worker and worker<->worker calls.
//!
//! - **Wire format**: [`postcard`] (serde-based, compact, no schema bytes on the
//!   wire, `no_std`-friendly). Generated clients/servers frame calls as
//!   `(service, method_id, postcard(args))` -> `postcard(ret)`.
//! - **Transport**: abstracted by [`Transport`]; one impl per link (in-process
//!   channel for native CEF workers, postMessage for web workers, the CEF IPC
//!   bridge for host<->page). The generated clients are transport-agnostic.
//! - **Schema**: each `#[rpc]` trait emits a `SCHEMA: RpcSchema` const so the
//!   build system can generate TypeScript + Rust clients.
//!
//! Interfaces are defined once in Rust; the `afterglow-rpc-macros` `#[rpc]`
//! macro generates the server dispatch, the Rust client, and the schema.

use serde::{de::DeserializeOwned, Serialize};

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

/// Web Worker transport (wasm32). The WASM worker receives postMessage'd
/// framed requests, runs the generated `serve`, and posts responses. The TS
/// client (xtask gen-ts) posts requests + listens for responses.
#[cfg(target_arch = "wasm32")]
pub mod web {
    use crate::{RpcError, RpcResult};
    /// Frame layout: `[method_id: u32 LE][args bytes...]`. Returns (method_id, args).
    pub fn decode_frame(msg: &[u8]) -> RpcResult<(u32, &[u8])> {
        if msg.len() < 4 {
            return Err(RpcError::Transport("short frame".into()));
        }
        let method = u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);
        Ok((method, &msg[4..]))
    }
    /// Frame a response (serve already encoded it): raw bytes.
    pub fn frame_response(resp: &[u8]) -> Vec<u8> {
        resp.to_vec()
    }
}

pub type RpcResult<T> = Result<T, RpcError>;

#[derive(Debug)]
pub enum RpcError {
    /// postcard (de)serialization failure.
    Codec(String),
    /// Unknown service/method id.
    UnknownMethod,
    /// Transport-level failure (channel closed, fetch error, etc.).
    Transport(String),
    /// Server-side handler returned an error.
    Remote(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(s) => write!(f, "rpc codec: {s}"),
            Self::UnknownMethod => write!(f, "rpc unknown method"),
            Self::Transport(s) => write!(f, "rpc transport: {s}"),
            Self::Remote(s) => write!(f, "rpc remote: {s}"),
        }
    }
}
impl std::error::Error for RpcError {}

// --- codec helpers (used by generated code) -------------------------------

pub fn encode<T: Serialize>(v: &T) -> RpcResult<Vec<u8>> {
    postcard::to_allocvec(v).map_err(|e| RpcError::Codec(e.to_string()))
}
pub fn decode<T: DeserializeOwned>(b: &[u8]) -> RpcResult<T> {
    postcard::from_bytes(b).map_err(|e| RpcError::Codec(e.to_string()))
}

// --- transport ------------------------------------------------------------

/// The byte-pipe a generated client talks over. One implementation per link
/// type (channel, postMessage, CEF IPC). `call` is synchronous — native
/// in-process workers block on a channel; cross-process/web links wrap it.
pub trait Transport {
    fn call(&self, service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>>;
}

/// In-memory loopback: handy for tests and for worker<->worker calls within the
/// same process. The handler dispatches `(service, method, args)` -> response.
pub struct Loopback<F>(pub F);
impl<F> Transport for Loopback<F>
where
    F: Fn(&str, u32, &[u8]) -> RpcResult<Vec<u8>>,
{
    fn call(&self, service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        (self.0)(service, method, args)
    }
}

// --- schema (for build-system codegen) ------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct RpcSchema {
    pub name: &'static str,
    pub methods: &'static [RpcMethod],
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct RpcMethod {
    pub id: u32,
    pub name: &'static str,
    /// (param name, rust type as a string) — the build system maps these to TS.
    pub params: &'static [(&'static str, &'static str)],
    pub returns: &'static str,
}

/// Map a Rust type string to a TypeScript type string. Unknown/custom types
/// pass through (they must be generated separately, e.g. via a `#[derive]` that
/// emits their `.ts`).
pub fn rust_type_to_ts(ty: &str) -> String {
    let ty = ty.trim();
    match ty {
        "bool" => "boolean".into(),
        "f32" | "f64" | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize" => "number".into(),
        "String" | "&str" | "str" => "string".into(),
        _ if ty.starts_with("Vec<u8>") || ty == "&[u8]" || ty == "[u8]" => "Uint8Array".into(),
        _ if ty.starts_with("Vec<") => {
            let inner = ty.trim_start_matches("Vec<").trim_end_matches('>');
            format!("{}[]", rust_type_to_ts(inner))
        }
        _ if ty.starts_with("Option<") => {
            let inner = ty.trim_start_matches("Option<").trim_end_matches('>');
            format!("{} | null", rust_type_to_ts(inner))
        }
        _ => ty.into(), // custom type — generated elsewhere
    }
}
