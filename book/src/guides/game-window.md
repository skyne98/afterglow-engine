# Building a Game Window

> **Removed.** `afterglow-cef` and its `AppBuilder` have been removed; this
> walkthrough targets the deleted CEF host. `afterglow-shell` is the sole native
> host but does not yet expose a production game bootstrap API or asset-root
> loading. A replacement walkthrough is tracked in
> `docs/implementation/shell-promotion-plan.md` (gates G1 + G3). The code
> below no longer compiles.

An end-to-end walkthrough: a native CEF window with WebGPU, an embedded page,
and an asset filesystem root.

## The shell

```rust
use afterglow_cef::AppBuilder;

// Embed the entry page; it imports Three.js and your game module.
const INDEX: &[u8] = include_bytes!("../assets/index.html");

fn main() {
    AppBuilder::new()
        .title("my game")
        .size(1920, 1080)
        .devtools(9222)                       // 0 = off for release
        .index_html(INDEX)
        // Three.js is large; serve it from disk during dev, embed for ship:
        .fs_root("crates/afterglow-web/www")  // worker wasm + JS
        .on_console(|m| eprintln!("[js] {m}"))
        .on_ready(|| {
            // Spawn native workers here — safe (after execute_process).
        })
        .run();
}
```

Build and run through the devshell:

```sh
nix-shell shell.nix --run "cargo build --example minimal"
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
```

## The page

The page is a normal Three.js/WebGPU page. Because `afterglow://` is registered
as standard + secure + CORS + fetch + CSP-bypass, ES-module imports work:

```html
<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>my game</title></head>
<body><canvas id="c"></canvas>
<script type="module" src="./game.js"></script>
</body></html>
```

Author `game.ts` against package modules and public engine barrels, then bundle
it into the deployment:

```ts
import * as THREE from 'three/webgpu';
import { EngineRuntime, RendererHost } from './engine/index.ts';
```

`afterglow://local/` is same-origin and secure-context, so WebGPU,
`SharedArrayBuffer` (COOP/COEP is set by the scheme handler), and ES modules all
work without a localhost server. See [Serving Assets](../window/assets.md).

## Starting a native worker

Spawn from `on_ready` (never before `execute_process` — it crashes the GPU
process):

```rust
use afterglow_rpc_demo::{PhysicsClient, PhysicsWorker};
use std::sync::OnceLock;

static PHYSICS: OnceLock<PhysicsClient<afterglow_rpc::native::WorkerTransport>> = OnceLock::new();

// in main():
.on_ready(|| {
    let (client, _events) = PhysicsClient::spawn_worker(PhysicsWorker).unwrap();
    // The callback must not block — store the client and return.
    let _ = PHYSICS.set(client);
})
```

The callback must not block — spawn the worker, hand the client off, and return.
CEF services that have native implementations must be spawned here; their
WASM/Web Worker build is not a CEF fallback. `SharedArrayBuffer` workers are for
the public-web target and explicitly page-only diagnostics.

> **CLI flags win.** Flags like `--ozone-platform=x11` and `--disable-gpu-vsync`
> are only added by the shell if not already present, so you can override the
> defaults from the command line.

## Asset strategy

- **Embed `index.html`** with `.index_html(include_bytes!("index.html"))` —
  it ships in the binary and loads with zero filesystem access.
- **Use `.fs_root(dir)` for everything else** — Three.js, textures, wasm,
  models. Assets stream via `pread` (no whole-file buffering); the FS path is
  canonically confined (traversal and symlink escapes rejected).
- For shipping, keep a read-only asset directory next to the binary.

## Debugging

- `.devtools(9222)` opens a remote debugging port — open
  `http://127.0.0.1:9222` in a browser, or attach with
  [`latency-tool`](../building/benchmarking.md).
- `.on_console(f)` forwards every JS console message to your callback.
- If WebGPU shows a software adapter, the Vulkan loader wiring is wrong — see
  [Debugging](../reference/debugging.md).

## Next

- [The AppBuilder API](../window/app-builder.md) — the full builder reference.
- [Your First Worker](./first-worker.md) — define + call a service.
