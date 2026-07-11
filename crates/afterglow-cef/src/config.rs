//! Builder + config for an afterglow-cef app.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// The custom scheme's name and host. URLs look like `afterglow://local/index.html`.
pub const SCHEME: &str = "afterglow";
pub const SCHEME_DOMAIN: &str = "local";

/// boxed `console.*` sink (stderr if unset).
pub(crate) type ConsoleCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// One-shot "CEF context ready" callback. Stored behind a [`Mutex`] so an
/// `&Config` (held in the process-global [`OnceLock`]) can take and run it
/// exactly once. Set via [`AppBuilder::on_ready`]; run from
/// `BrowserProcessHandler::on_context_initialized`.
pub(crate) type OnReadyCallback = Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>;

/// Resolved app configuration. Built by [`AppBuilder`]; read by the runtime
/// from a process-global [`OnceLock`].
#[derive(Default)]
pub(crate) struct Config {
    pub(crate) title: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) devtools_port: i32,
    pub(crate) vsync: bool,
    pub(crate) root_path: String,
    pub(crate) embedded: Vec<(String, String, &'static [u8])>,
    pub(crate) fs_root: Option<PathBuf>,
    /// Canonicalized once before CEF starts; used by every resource request.
    pub(crate) asset_root: Option<afterglow_assets::AssetRoot>,
    pub(crate) console: Option<ConsoleCallback>,
    pub(crate) on_ready: OnReadyCallback,
}

impl Config {
    /// Take and run the [`AppBuilder::on_ready`] callback exactly once, if set.
    /// Called from `BrowserProcessHandler::on_context_initialized` (after
    /// `execute_process` and CEF context init). No-op when unset; idempotent —
    /// the callback is moved out before running, so a second call is a no-op
    /// and a panicking callback never poisons the lock.
    pub(crate) fn run_ready(&self) {
        let cb = self.on_ready.lock().expect("on_ready lock poisoned").take();
        if let Some(f) = cb {
            f();
        }
    }
}

pub(crate) static CONFIG: OnceLock<Config> = OnceLock::new();

/// Builder for an afterglow-cef game window. Call [`AppBuilder::run`] to start.
///
/// ```no_run
/// use afterglow_cef::AppBuilder;
///
/// AppBuilder::new()
///     .title("my game")
///     .size(1920, 1080)
///     .devtools(9222)                 // 0 = off
///     .root("/index.html")
///     .asset("/index.html", "text/html", b"<html></html>")
///     .run();
/// ```
pub struct AppBuilder {
    cfg: Config,
}

impl AppBuilder {
    pub fn new() -> Self {
        Self {
            cfg: Config {
                title: "afterglow".into(),
                width: 1280,
                height: 800,
                devtools_port: 0,
                vsync: true,
                root_path: "/index.html".into(),
                ..Default::default()
            },
        }
    }

    pub fn title(mut self, t: impl Into<String>) -> Self {
        self.cfg.title = t.into();
        self
    }
    pub fn size(mut self, w: i32, h: i32) -> Self {
        self.cfg.width = w;
        self.cfg.height = h;
        self
    }
    /// Enable Chrome DevTools on this port (0 = off).
    pub fn devtools(mut self, port: u16) -> Self {
        self.cfg.devtools_port = port as i32;
        self
    }
    /// vsync on (default; smooth) vs off (uncapped — choppy on some setups).
    pub fn vsync(mut self, on: bool) -> Self {
        self.cfg.vsync = on;
        self
    }
    /// Scheme path loaded on startup (default `/index.html`).
    pub fn root(mut self, path: impl Into<String>) -> Self {
        self.cfg.root_path = path.into();
        self
    }

    /// Embed an asset served at `path` (e.g. `/three.webgpu.js`).
    pub fn asset(
        mut self,
        path: impl Into<String>,
        mime: impl Into<String>,
        bytes: &'static [u8],
    ) -> Self {
        self.cfg.embedded.push((path.into(), mime.into(), bytes));
        self
    }

    /// Serve files from a filesystem directory (for paths not in `embedded`).
    /// This is the direct-FS, no-HTTP load path.
    pub fn fs_root(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cfg.fs_root = Some(dir.into());
        self
    }

    /// Forward JS `console.*` to this callback (defaults to stderr).
    pub fn on_console(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.cfg.console = Some(Arc::new(f));
        self
    }

    /// Set a one-shot readiness callback invoked exactly once after CEF context
    /// initialization — from `BrowserProcessHandler::on_context_initialized`,
    /// which necessarily runs after `execute_process` and CEF init. This is the
    /// safe point to spawn native RPC worker threads (e.g.
    /// `PhysicsClient::spawn_worker` from `afterglow-rpc::native`); spawning OS
    /// threads before `execute_process` crashes the GPU process.
    ///
    /// Ordering within `on_context_initialized`: after
    /// `resources::register_factory` and
    /// before browser-view creation. It runs on CEF's browser-process UI thread,
    /// so it must not block — spawn the worker and return; the worker runs on
    /// its own thread.
    ///
    /// Default: unset (`None`) — `Config::run_ready` is a no-op. Calling
    /// `on_ready` again **replaces** any previously set callback (last wins);
    /// the replaced callback is dropped without running.
    pub fn on_ready(self, f: impl FnOnce() + Send + 'static) -> Self {
        *self.cfg.on_ready.lock().expect("on_ready lock poisoned") = Some(Box::new(f));
        self
    }

    /// Configure and run the CEF message loop (blocks until the window closes).
    pub fn run(mut self) {
        self.cfg.asset_root = self
            .cfg
            .fs_root
            .as_deref()
            .and_then(afterglow_assets::AssetRoot::new);
        crate::runtime::run(self.cfg);
    }
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn on_ready_absent_is_harmless() {
        // No callback set: run_ready is a no-op and must not panic.
        let cfg = Config::default();
        cfg.run_ready();
        // Idempotent: a second call is still a no-op.
        cfg.run_ready();
    }

    #[test]
    fn on_ready_runs_exactly_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let builder = AppBuilder::new().on_ready(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let cfg = builder.cfg;
        // First call runs the callback once.
        cfg.run_ready();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // Second call is a no-op: the callback was taken.
        cfg.run_ready();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn on_ready_replaces_prior_callback() {
        let a = Arc::new(AtomicUsize::new(0));
        let b = Arc::new(AtomicUsize::new(0));
        let a2 = a.clone();
        let b2 = b.clone();
        let builder = AppBuilder::new()
            .on_ready(move || {
                a2.fetch_add(1, Ordering::SeqCst);
            })
            .on_ready(move || {
                b2.fetch_add(1, Ordering::SeqCst);
            });
        let cfg = builder.cfg;
        cfg.run_ready();
        // Only the most recently set callback runs; the first is replaced and
        // dropped without running.
        assert_eq!(a.load(Ordering::SeqCst), 0);
        assert_eq!(b.load(Ordering::SeqCst), 1);
        cfg.run_ready();
        assert_eq!(a.load(Ordering::SeqCst), 0);
        assert_eq!(b.load(Ordering::SeqCst), 1);
    }
}
