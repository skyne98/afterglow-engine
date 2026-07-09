//! # afterglow-cef
//!
//! Game-window-focused CEF ([cef-rs] / `tauri-apps/cef-rs`) wrapper for
//! afterglow-engine. Configures CEF internally for the best game-window
//! experience and serves engine assets **directly from the FS / embedded bytes
//! via a CEF custom scheme** — no localhost HTTP server, no network stack on the
//! resource path (lower-latency native loads).
//!
//! What it sets up for you:
//! - **Windowed** rendering (lowest structural input→present latency; no OSR
//!   texture copies).
//! - **WebGPU + Vulkan forced on the real GPU** (`--enable-unsafe-webgpu`,
//!   `--enable-features=Vulkan`, `--use-angle=vulkan`, `--ignore-gpu-blocklist`).
//! - **X11/XWayland** (`--ozone-platform=x11`): Wayland+Vulkan is incompatible
//!   in CEF 149. Overridable via CLI.
//! - **`afterglow://local/` custom scheme** (standard + secure + CORS + fetch)
//!   serving embedded assets and/or files from a FS root directly through CEF —
//!   ES-module Three.js and WebGPU both work, same-origin, no TCP server.
//! - **JS↔Rust invoke bridge** over the same scheme (`POST /__invoke`).
//! - **DevTools** behind a port flag; **JS console** forwarded to a callback.
//!
//! [cef-rs]: https://github.com/tauri-apps/cef-rs

mod config;
mod flags;
mod input;
mod ipc;
mod message_router;
mod resources;
mod runtime;

pub use config::{AppBuilder, Config, SCHEME, SCHEME_DOMAIN};
pub use input::{take_input_receiver, InputEvent, InputKind};
pub use ipc::{config_js, emit, set_main_browser, INVOKE_JS};
pub use message_router::EMULATED_WORKER_JS;

/// `afterglow://local/` + a path, e.g. `root_url("/index.html")`.
pub fn root_url(path: &str) -> String {
    format!("{SCHEME}://{SCHEME_DOMAIN}{path}")
}
