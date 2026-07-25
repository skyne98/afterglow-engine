//! # afterglow-web
//!
//! Web-facing runtime crate for afterglow-engine. It owns the native COOP/COEP
//! development asset server and (historically) hosted the SharedArrayBuffer
//! transport wasm. The transport now lives in [`afterglow_rpc::wasm`]; this
//! crate no longer produces a wasm module.
//!
//! The authored TypeScript runtime, demos, and generated worker clients live
//! under `web/` (not in this Rust crate). The disposable generated deployment
//! tree is `www/`.
//!
//! Build the dev server: `cargo build -p afterglow-web --example coep_server`

// The HTTP dev server is native-only (filesystem + TCP). Gating it off the
// wasm target keeps `afterglow-assets` and unrelated server code out of any
// wasm build.
#[cfg(not(target_arch = "wasm32"))]
pub mod dev_server;
