//! Builder + config for an afterglow-cef app.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// The custom scheme's name and host. URLs look like `afterglow://local/index.html`.
pub const SCHEME: &str = "afterglow";
pub const SCHEME_DOMAIN: &str = "local";

/// Resolved app configuration. Built by [`AppBuilder`]; read by the runtime
/// from a process-global [`OnceLock`].
#[derive(Default)]
pub struct Config {
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub devtools_port: i32,
    pub vsync: bool,
    /// Path under the scheme to load first, e.g. "/index.html".
    pub root_path: String,
    /// Embedded assets: (scheme-path, mime, bytes).
    pub embedded: Vec<(String, String, &'static [u8])>,
    /// If set, files under this dir are served for paths not in `embedded`.
    pub fs_root: Option<PathBuf>,
    /// JS console.* forwarded here (stderr if unset).
    pub console: Option<Arc<dyn Fn(&str) + Send + Sync>>,
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

    pub fn title(mut self, t: impl Into<String>) -> Self { self.cfg.title = t.into(); self }
    pub fn size(mut self, w: i32, h: i32) -> Self { self.cfg.width = w; self.cfg.height = h; self }
    /// Enable Chrome DevTools on this port (0 = off).
    pub fn devtools(mut self, port: u16) -> Self { self.cfg.devtools_port = port as i32; self }
    /// vsync on (default; smooth) vs off (uncapped — choppy on some setups).
    pub fn vsync(mut self, on: bool) -> Self { self.cfg.vsync = on; self }
    /// Scheme path loaded on startup (default `/index.html`).
    pub fn root(mut self, path: impl Into<String>) -> Self { self.cfg.root_path = path.into(); self }

    /// Embed an asset served at `path` (e.g. `/three.webgpu.js`).
    pub fn asset(mut self, path: impl Into<String>, mime: impl Into<String>, bytes: &'static [u8]) -> Self {
        self.cfg.embedded.push((path.into(), mime.into(), bytes));
        self
    }

    /// Serve files from a filesystem directory (for paths not in `embedded`).
    /// This is the direct-FS, no-HTTP load path.
    pub fn fs_root(mut self, dir: impl Into<PathBuf>) -> Self { self.cfg.fs_root = Some(dir.into()); self }

    /// Forward JS `console.*` to this callback (defaults to stderr).
    pub fn on_console(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.cfg.console = Some(Arc::new(f));
        self
    }

    /// Configure and run the CEF message loop (blocks until the window closes).
    pub fn run(self) {
        crate::runtime::run(self.cfg);
    }
}

impl Default for AppBuilder {
    fn default() -> Self { Self::new() }
}
