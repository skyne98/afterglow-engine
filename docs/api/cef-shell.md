# `afterglow-cef` API — game-window CEF shell

> Status: working; API checked against the 2026-07-10 source.

## Purpose

`afterglow-cef` is a game-window-focused CEF ([cef-rs] / `tauri-apps/cef-rs`)
wrapper. It configures CEF internally for the best game-window experience and
serves engine assets **directly from embedded bytes and/or a filesystem root
through a CEF custom scheme** — no localhost HTTP server, no network stack on
the resource path (lower-latency native loads).

[cef-rs]: https://github.com/tauri-apps/cef-rs

What it sets up:

- **Windowed** rendering (Views framework; lowest structural input→present
  latency — no OSR texture copies).
- **WebGPU + Vulkan on the real GPU, fail-closed**:
  `--enable-unsafe-webgpu`, `--ignore-gpu-blocklist`,
  `--enable-features=Vulkan`, `--use-angle=vulkan`, and `--disable-webgl`.
  Engine pages never silently render under WebGL2.
- **X11/XWayland** (`--ozone-platform=x11`): Wayland+Vulkan is incompatible in
  CEF 149 (native Wayland + WebGPU isn't available yet). Overridable via CLI.
- **`afterglow://local/` custom scheme** (standard + secure + CORS + fetch +
  CSP-bypass) serving embedded assets and/or files from a FS root directly
  through CEF — ES-module Three.js and WebGPU both work, same-origin, no TCP
  server.
- **COOP/COEP headers** on the scheme handler so `SharedArrayBuffer` works.
- **DevTools** behind a port flag; **JS console** forwarded to a callback.

The minimal example (`crates/afterglow-cef/examples/minimal.rs`) serves the
`afterglow-web` SharedArrayBuffer worker transport (`afterglow_web.wasm` +
`physics_worker.wasm` + `worker.js` + `rpc.js`) over `afterglow://` — the same
ring-buffer mechanism as the web target (see
[`web-shared-memory.md`](web-shared-memory.md)). Native `#[rpc]` workers
(`spawn_worker`) are an OS-thread option started safely from
`AppBuilder::on_ready` (the readiness callback that fires once after CEF
context init); see the startup caveat below and
[`ring-buffer.md`](ring-buffer.md).

## Public API

The crate re-exports its public surface from `lib.rs`:

```rust
pub use config::{AppBuilder, SCHEME, SCHEME_DOMAIN};
pub fn root_url(path: &str) -> String;
```

Everything else (`runtime`, `flags`, `resources`, and the `GameApp` /
`GameClient` / `AfterglowResource` / `AfterglowSchemeFactory` wrappers) is
internal — driven by the process-global `Config` — and is documented as
*behavior* below, not as API you call.

### `AppBuilder`

Builder for an afterglow-cef game window. `run` blocks until the window closes.

```rust
pub struct AppBuilder { /* private config */ }

impl AppBuilder {
    pub fn new() -> Self;
    pub fn title(self, t: impl Into<String>) -> Self;
    pub fn size(self, w: i32, h: i32) -> Self;
    pub fn devtools(self, port: u16) -> Self;   // 0 = off
    pub fn vsync(self, on: bool) -> Self;
    pub fn root(self, path: impl Into<String>) -> Self;       // scheme path loaded on startup
    pub fn asset(self, path: impl Into<String>, mime: impl Into<String>, bytes: &'static [u8]) -> Self;
    pub fn fs_root(self, dir: impl Into<PathBuf>) -> Self;     // FS fallback for non-embedded paths
    pub fn on_console(self, f: impl Fn(&str) + Send + Sync + 'static) -> Self;
    pub fn on_ready(self, f: impl FnOnce() + Send + 'static) -> Self;  // one-shot readiness cb after CEF init
    pub fn run(self);                                          // blocks until window closes
}
```

Defaults:

| Field | Default |
|---|---|
| `title` | `"afterglow"` |
| `size` | `1280 × 800` |
| `devtools` | `0` (off) |
| `vsync` | `true` |
| `root` | `"/index.html"` |
| `asset` | none (empty embedded list) |
| `fs_root` | `None` (embedded-only) |
| `on_console` | `None` (console → stderr) |
| `on_ready` | `None` (unset; no callback) |

Minimal app (from `crates/afterglow-cef/examples/minimal.rs`):

```rust
use afterglow_cef::AppBuilder;

AppBuilder::new()
    .title("my game")
    .size(1920, 1080)
    .devtools(9222)                 // 0 = off
    .root("/index.html")
    .asset("/index.html", "text/html", include_bytes!("index.html"))
    .run();
```

The resolved `Config` is intentionally crate-private; `AppBuilder` is the only
configuration API, preventing callers from constructing partially initialized
runtime state.

### Constants and `root_url`

```rust
pub const SCHEME: &str = "afterglow";
pub const SCHEME_DOMAIN: &str = "local";

/// `afterglow://local/` + path; a missing leading slash is added.
pub fn root_url(path: &str) -> String;
```

So `root_url("/index.html")` and `root_url("index.html")` both yield
`afterglow://local/index.html`. The runtime loads `root_url(cfg.root_path)` on
startup.

## Resource serving (internal behavior)

Resources are served through CEF's internal request routing on the
`afterglow://local/` scheme — no localhost HTTP server. The `resources` module
owns only the CEF-specific policy; path/MIME/status resolution is shared with
the web dev server via [`afterglow-assets`](assets.md).

**Scheme registration** (`on_register_custom_schemes`): `afterglow` is
registered with flags `121` = `STANDARD(1) | SECURE(8) | CORS_ENABLED(16) |
CSP_BYPASSING(32) | FETCH_ENABLED(64)`, so ES-module imports, WebGPU, fetch, and
inline scripts work on `afterglow://` URLs. The factory is registered for
`SCHEME` + `SCHEME_DOMAIN` on the browser-process UI thread
(`on_context_initialized`).

**Resolution policy** (`AfterglowResource::open`), per request:

1. Strip `afterglow://local` + `?query` and percent-decode the UTF-8 path.
2. **Embedded-first**: if the path matches an `asset(..)` entry, serve its
   bytes with its configured MIME (200).
3. **FS fallback**: else if `fs_root` is set, delegate to
   [`afterglow_assets::resolve`](assets.md) (lexically + canonically confined —
   traversal/symlink escapes rejected) and stream it through `FsSource`; MIME
   comes from [`afterglow_assets::guess_mime`](assets.md).
4. Anything else (missing, escaped, unreadable, or no FS root) → `404 not found`
   as `text/plain`. A malformed request (no `Request`) → `400 bad request`.

Each response resets a single mutex-protected `ResponseState`
(body/mime/status/offset) on `open`, so a reused handler can never leak a prior
response. `read` uses `AssetSource::read_at` with offset tracking; `cancel` is
a no-op. Single byte-range requests return `206` with `Content-Range`. CEF then
calls `skip(start)`, so the handler starts range responses at source offset zero
and applies that skip exactly once. This keeps `.big` page fetches bounded
instead of buffering the whole multi-gigabyte container.

## COOP/COEP (SharedArrayBuffer)

`response_headers` sets, on every response:

```http
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
Cross-Origin-Resource-Policy: same-origin
```

These headers match the web dev server and enable `SharedArrayBuffer` on
`afterglow://` pages.

## Console forwarding

`on_console(f)` forwards each JS console message formatted with severity,
source, and line. If unset, messages go to stderr with a `[console]` prefix.
This is the only JS→Rust callback the shell exposes today.

## WebGPU / X11 constraints

Flags are applied in `on_before_command_line_processing` (so they propagate to
all child processes) and only added if not already present — **CLI flags win**,
so `--ozone-platform=wayland` or `--disable-gpu-vsync` override the defaults.

| Flag | Why |
|---|---|
| `--enable-unsafe-webgpu` | enable WebGPU |
| `--ignore-gpu-blocklist` | use the real GPU even if "unsupported" |
| `--disable-webgl` | forbid Chromium WebGL; WebGPU is required, never a fallback |
| `--enable-features=Vulkan` | Dawn → Vulkan backend |
| `--use-angle=vulkan` | ANGLE over Vulkan |
| `--ozone-platform=x11` | Wayland+Vulkan incompatible in CEF 149 → XWayland |
| `--disable-gpu-vsync` | only when `vsync(false)` — smooth by default, opt-in uncapped |
| `--disable-frame-rate-limit` | only when `vsync(false)` |

See
[`docs/research/cef-rs-webgpu-prototype-findings.md`](../research/cef-rs-webgpu-prototype-findings.md)
and
[`docs/research/cef-games-latency-footprint-debugging.md`](../research/cef-games-latency-footprint-debugging.md)
for the empirical basis, and the debugging notes in
[`AGENTS.md`](../../AGENTS.md) for `CEF_PATH` / `shell.nix` wiring.

### Linux Vulkan-stack requirement

`shell.nix` deliberately selects one coherent real Vulkan stack before CEF's
resource directory, which contains CEF's unusable SwiftShader loader. NixOS uses
`/run/opengl-driver/lib` and its matching ICDs. Other Linux distributions use
the Nix `vulkan-loader` + Mesa ICD by default; set
`AFTERGLOW_VULKAN_STACK=host` only to diagnose a host driver.

This distinction is operationally important on fox-laptop (Radeon 680M): host
Fedora 44 Mesa 26.1.4 RADV crashes CEF 149's GPU process with `SIGFPE` in
`radv_clear_dcc_comp_to_single`. Historically CEF then silently fell back to
WebGL2. This is now prevented twice: CEF disables WebGL and authored pages use
`engine/webgpu-only.ts`, which clears Three r185's fallback callback before
`init()`, validates `isWebGPUBackend`, and replaces the page with a fatal error
on startup failure or device loss. The default Nix Mesa 25.3.4 stack is
validated on that machine. A successful test must
confirm an `amd` / `rdna-2` `navigator.gpu.requestAdapter()` result and contain
neither `GPU process exited` nor `WebGPU is not available` in the CEF log. The
reproducible command sequence and performance boundary are canonical in
[`AGENTS.md`](../../AGENTS.md#fox-laptop-radeon-680m-cefwebgpu-validation).

## Process / thread startup caveat

`AppBuilder::run` sets the config, loads the CEF library, parses args, then
calls `execute_process` (which forks CEF child processes) followed by
`initialize` + `run_message_loop`. **Do not spawn OS threads before
`execute_process` runs** — spawning threads before the GPU process is forked
crashes it. The shell itself spawns no threads before that point.

Consequence for native workers: a `#[rpc]` native worker
(`PhysicsClient::spawn_worker`) creates an OS thread and must be started only
after CEF context initialization (from the browser-process UI thread), never
before `.run()`. Use `AppBuilder::on_ready` for this — it stores a one-shot
`FnOnce() + Send + 'static` callback that the runtime invokes **exactly once**
from `BrowserProcessHandler::on_context_initialized`, which necessarily runs
after `execute_process` and CEF init.

Startup ordering inside `on_context_initialized`:

1. `resources::register_factory()` — the `afterglow://` scheme handler.
2. `Config::run_ready()` — the `on_ready` callback, if set.
3. Browser-view + top-level window creation.

The callback runs on CEF's browser-process UI thread, so it must not block —
spawn the worker and return; the worker runs on its own thread. Default is unset
(`None`), in which case `run_ready` is a no-op. Calling `on_ready` again
**replaces** any previously set callback (last wins; the replaced callback is
dropped without running). The practical alternative — for page-driven workers
without a native thread — is the `SharedArrayBuffer` web transport used by the
minimal example; see [`web-shared-memory.md`](web-shared-memory.md).

## Cross-links

- [`assets.md`](assets.md) — `guess_mime` / `resolve`, the shared path/MIME
  boundary the FS-fallback path delegates to.
- [`web-shared-memory.md`](web-shared-memory.md) — the `SharedArrayBuffer`
  worker transport served over `afterglow://`, and COOP/COEP.
- [`ring-buffer.md`](ring-buffer.md) / [`rpc-macro.md`](rpc-macro.md) — native
  worker transport and the `#[rpc]` macro (for the native-worker option).
