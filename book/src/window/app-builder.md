# The AppBuilder API

`AppBuilder` is the only configuration API for the native shell. You build one,
call `.run()`, and it blocks until the window closes.

```rust
use afterglow_cef::AppBuilder;

AppBuilder::new()
    .title("my game")
    .size(1920, 1080)
    .devtools(9222)                 // 0 = off
    .index_html(include_bytes!("index.html"))
    .fs_root("assets")              // everything else, streamed from disk
    .on_console(|message| eprintln!("[js] {message}"))
    .on_ready(|| { /* spawn native workers here */ })
    .run();
```

## Methods

| Method | Effect | Default |
|---|---|---|
| `new()` | Create a builder with defaults. | — |
| `title(t)` | Window title. | `"afterglow"` |
| `size(w, h)` | Window size in px. | `1280 × 800` |
| `devtools(port)` | Remote debugging port; `0` = off. | off |
| `vsync(on)` | Vsync on (smooth) or off (uncapped). | on |
| `root(path)` | Scheme path loaded on startup. | `"/index.html"` |
| `index_html(bytes)` | Embed `index.html` (the one embedded asset). | none |
| `fs_root(dir)` | Filesystem root for streaming all non-embedded assets. | none |
| `on_console(f)` | Forward each JS console message. | stderr `[console]` |
| `on_ready(f)` | One-shot readiness callback, fires once after CEF init. | unset |
| `run()` | Load CEF, create the window, block until it closes. | — |

## Constants and `root_url`

```rust
pub const SCHEME: &str = "afterglow";
pub const SCHEME_DOMAIN: &str = "local";

/// `afterglow://local/` + path; a missing leading slash is added.
pub fn root_url(path: &str) -> String;
```

`root_url("/index.html")` and `root_url("index.html")` both yield
`afterglow://local/index.html`. The runtime loads `root_url(cfg.root_path)` on
startup. Useful when you need to construct a same-origin URL from JS-side
configuration.

## `on_ready`: the safe spawn point

A native worker is an OS thread, and **spawning threads before CEF's
`execute_process` crashes the GPU process**. So spawn native workers only from
`on_ready`, which fires exactly once, after CEF init, on the browser-process UI
thread:

```rust
AppBuilder::new()
    .on_ready(|| {
        // Safe: this runs after execute_process, on the UI thread.
        // Spawn the worker, move the client somewhere reachable, return.
        let (client, _events) = PhysicsClient::spawn_worker(PhysicsWorker).unwrap();
        // store `client` …
    })
    .run();
```

The callback **must not block** — spawn the worker, hand it off, and return;
the worker runs on its own thread. Calling `on_ready` again replaces any
previously set callback (last wins; the replaced callback is dropped without
running).

If you don't need a native worker, leave `on_ready` unset and drive the worker
from the page over `SharedArrayBuffer` instead (the `minimal` example does
this — no Rust spawn needed). See [Native Workers](../workers/native-workers.md).

## `on_console`

`on_console(f)` forwards each JS console message formatted with severity,
source, and line. If unset, messages go to stderr with a `[console]` prefix.
This is the only JS→Rust callback the shell exposes; richer two-way calls
should go through a worker ring buffer, not through the shell.

## What the shell sets up for you

You don't configure these — they're internal defaults:

- **Windowed rendering** (CEF Views; no off-screen texture copies).
- **WebGPU + Vulkan on the real GPU** (`--enable-unsafe-webgpu`,
  `--ignore-gpu-blocklist`, `--enable-features=Vulkan`, `--use-angle=vulkan`).
- **X11/XWayland** (`--ozone-platform=x11`); CLI-overridable.
- **`afterglow://local/` scheme** (standard + secure + CORS + fetch +
  CSP-bypass) serving the embedded `index.html` + FS assets — see
  [Serving Assets](./assets.md).
- **COOP/COEP headers** so `SharedArrayBuffer` works.
- **Streaming + ranges** — assets stream via `AssetSource::read_at` (no
  whole-file buffering); `Range`/`skip` supported. See
  [The Asset System](./asset-system.md).
- **DevTools** behind a port flag.

## Next

- [Serving Assets](./assets.md) — how the `afterglow://` scheme resolves your
  embedded and filesystem assets.
- [Graphics & DevTools](./graphics.md) — the GPU flags, vsync, and debugging.
