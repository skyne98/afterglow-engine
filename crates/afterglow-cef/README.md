# afterglow-cef

Game-window-focused CEF ([cef-rs] / `tauri-apps/cef-rs`) wrapper for
afterglow-engine. It internalizes all CEF configuration and serves engine assets
**directly from the FS / embedded bytes via a CEF custom scheme** — no localhost
HTTP server, no network stack on the resource path (lower-latency native loads
than a web build).

## Why a custom scheme instead of an HTTP server

A `file://` page can't do cross-file ES-module imports (CORS treats `file:`
origins as unique), so loading Three.js (an ES module) needed a localhost HTTP
server in the prototype. That works but funnels every asset through a TCP
socket. Instead we register `afterglow://local/` as a **standard + secure +
CORS + fetch** scheme with a CEF `SchemeHandlerFactory`/`ResourceHandler`:
Chromium routes requests for it internally to our handler, which reads the bytes
straight from embedded assets or the filesystem — no socket, no port, proper
same-origin secure context (modules + WebGPU both work).

> Note: this is about **asset load** latency, not per-frame latency. The game
> render loop (Three.js → WebGPU → present) never touches resource loading; our
> measured input→present is ~3 ms regardless. The scheme handler removes the
> TCP/HTTP overhead from asset streaming and large/concurrent fetches.

## What it configures for you

- **Windowed** rendering (lowest structural input→present latency; no OSR copies).
- **WebGPU + Vulkan on the real GPU** (`--enable-unsafe-webgpu`,
  `--enable-features=Vulkan`, `--use-angle=vulkan`, `--ignore-gpu-blocklist`).
- **X11/XWayland** (`--ozone-platform=x11`): Wayland+Vulkan is incompatible in
  CEF 149. CLI-overridable.
- **`afterglow://local/`** scheme serving embedded + FS assets directly.
- **JS↔Rust invoke** bridge over the same scheme (`POST /__invoke`).
- **DevTools** behind a port flag; **JS console** forwarded to a callback.
- vsync on by default (smooth, monitor refresh rate); opt-in uncapped.

## Usage

```rust
use afterglow_cef::AppBuilder;
use serde_json::{json, Value};

AppBuilder::new()
    .title("my game")
    .size(1920, 1080)
    .devtools(9222)                 // 0 = off
    .root("/index.html")
    .asset("/index.html", "text/html", include_bytes!("../assets/index.html"))
    .asset("/three.webgpu.js", "text/javascript", include_bytes!("../assets/three.webgpu.js"))
    .fs_root("assets")              // serve other files straight from disk
    .on_invoke(|method: &str, params: Value| match method {
        "ping" => json!({ "pong": params }),
        _ => json!({ "error": "unknown" }),
    })
    .run();
```

In the page, include the invoke helper (`afterglow_cef::INVOKE_JS`):

```html
<script>/* paste INVOKE_JS here */</script>
<script>
  const out = await window.afterglow.invoke("ping", { n: 1 });
</script>
```

## Build & run (NixOS)

CEF binaries are downloaded by `cef-dll-sys` on first build (~hundreds of MB).
Build **without** `CEF_PATH` set so it fetches the matching CEF version, then
run inside the devshell (provides CEF's runtime libs + the real Vulkan ICD):

```sh
nix-shell shell.nix --run "cargo build --example minimal"
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
```

Always build & run through `nix-shell shell.nix` (it pins `CEF_PATH` to the
workspace target so `cef-dll-sys` reuses the cached CEF instead of
re-downloading it every build).

[cef-rs]: https://github.com/tauri-apps/cef-rs
