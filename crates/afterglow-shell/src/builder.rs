//! `ShellBuilder` — the production bootstrap API for `afterglow-shell`.
//!
//! Configures the native host (page root, window size/title, DevTools port) and
//! the worker-composition hook: a closure that registers the engine's native
//! `afterglow-rpc` workers (assets, texture, audio, …) into the
//! [`WorkerRegistry`](crate::rpc_bridge::WorkerRegistry) at startup. The
//! op-bridge (`op_afterglow_rpc_call_async`) then exposes them to JS.
//!
//! This is the native equivalent of CEF's `AppBuilder::on_ready`: the host
//! composes native workers once the winit window + wgpu device are ready, before
//! gameplay sealing. There is no `execute_process` ordering constraint on the
//! shell (a normal native process), so workers spawn directly at startup.

use std::path::PathBuf;

use deno_core::OpState;

/// A worker-composition hook: registers native workers at startup. Receives
/// `OpState` so it can borrow the [`WorkerRegistry`] it needs.
///
/// [`WorkerRegistry`]: crate::rpc_bridge::WorkerRegistry
pub type WorkerCompositionHook = Box<dyn FnOnce(&mut OpState)>;

/// Bootstrap configuration for the native shell.
pub struct ShellBuilder {
    /// HTML or module path to load (defaults to the bundled `native_game.ts`).
    pub root: Option<PathBuf>,
    /// Native filesystem asset root. Defaults to the game entry's directory.
    pub asset_root: Option<PathBuf>,
    /// Window size in physical pixels.
    pub size: (u32, u32),
    /// Window title.
    pub title: String,
    /// DevTools port (0 = disabled).
    pub devtools: u16,
    workers: Option<WorkerCompositionHook>,
}

impl Default for ShellBuilder {
    fn default() -> Self {
        Self {
            root: None,
            asset_root: None,
            size: (1280, 720),
            title: "afterglow-shell".into(),
            devtools: 0,
            workers: None,
        }
    }
}

impl ShellBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    /// HTML or module path to load. When omitted, the bundled `native_game.ts`
    /// runs.
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }
    /// Override the confined native asset root. Local game asset reads are
    /// served only by the native asset worker beneath this directory.
    pub fn asset_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.asset_root = Some(root.into());
        self
    }
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = (width, height);
        self
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }
    pub fn devtools(mut self, port: u16) -> Self {
        self.devtools = port;
        self
    }
    /// Register the worker-composition hook. The closure runs at startup, after
    /// the winit window + wgpu device are ready, with `OpState` to populate the
    /// [`WorkerRegistry`]. This is where the engine's native workers (assets,
    /// texture, audio, …) are composed.
    ///
    /// [`WorkerRegistry`]: crate::rpc_bridge::WorkerRegistry
    pub fn with_workers<F>(mut self, compose: F) -> Self
    where
        F: FnOnce(&mut OpState) + 'static,
    {
        self.workers = Some(Box::new(compose));
        self
    }

    /// Take the composition hook (called once at startup by the host).
    pub fn take_workers(&mut self) -> Option<WorkerCompositionHook> {
        self.workers.take()
    }
}

/// Compose a real engine async worker (the texture transcoder, asset loader,
/// audio worker): spawn it natively (a real OS thread via
/// `afterglow_rpc::native`), register it under a stable id. The op-bridge then
/// exposes it to JS through `op_afterglow_rpc_call`. Workers are
/// `Arc<AsyncWorkerTransport>` (`Send + Sync`) so they share across the JS
/// thread + the worker thread.
pub fn register_async_worker(
    state: &mut OpState,
    service: impl Into<String>,
    id: u32,
    transport: std::sync::Arc<afterglow_rpc::native::AsyncWorkerTransport>,
) {
    state
        .borrow_mut::<crate::rpc_bridge::WorkerRegistry>()
        .register_named_async(service, id, transport);
}
