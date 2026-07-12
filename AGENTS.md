# AGENTS.md — afterglow-engine

## Project direction

**afterglow-engine** is a **web-based game engine**. The rendering foundation
under evaluation is **Three.js with the WebGPU renderer** (with WebGL2 fallback).

See `docs/research/` for the stability/feasibility research that informs this
choice. Research notes are the canonical record of decisions — keep them
up-to-date as the stack evolves.

## Architecture

The game loop and renderer are just JS in a web page — Three.js +
`requestAnimationFrame`. There is no separate "game loop process" or "renderer
process". Workers do computation; the website orchestrates and renders.

### Communication: one mechanism

**`afterglow-rpc::RingBuffer`** — a lock-free SPSC ring buffer on raw pointers
+ `AtomicU32` (Acquire/Release). It is the only **payload** communication
mechanism for website↔worker and worker↔worker. RPC values use postcard;
thread unparks and payload-free `postMessage` calls are wake-ups only.

**Sync `#[rpc]`**: blocking request→response (one-in-flight, park/unpark).
**Async `#[rpc]`**: the poll model — `call_async` writes `[method][task_id][args]`
and returns a `Future` immediately; `poll()` per frame drains `[task_id][Response]`
completions and resolves pending futures. The worker runs an
`async-executor::LocalExecutor` (compiles on wasm). Multiple in-flight calls
are supported via `task_id` matching.

Two backends, same framing and ring layout:

| Target | Worker type | Memory backing | Crate |
|--------|-------------|----------------|-------|
| Native (CEF) | Native thread (`std::thread`) | Compact aligned heap allocation shared internally by `Arc` | `afterglow-rpc::native` |
| Web | Web Worker (wasm) | `SharedArrayBuffer` | `afterglow-web` |

- **Native**: workers are real OS threads with full `std`, no wasm overhead.
  The generated `Client::spawn_worker` creates request, response, and event
  rings, spawns the worker thread, and returns a typed client + event receiver.
  `run_worker_loop` drains requests and writes response envelopes.
- **Web**: JS creates a `SharedArrayBuffer`-backed `WebAssembly.Memory`
  (`shared: true`) and instantiates the page-side wasm with `--import-memory`.
  It sends the SAB + ring offsets to a Web Worker. The service wasm in that
  worker has separate memory; `worker.js` accesses the SAB rings with matching
  Atomics/framing and copies arguments/results to/from the service wasm.

### Crate structure

| Crate | Purpose |
|-------|---------|
| `afterglow-rpc` | Core: `RingBuffer`, `Transport` trait, postcard codec, response envelope, `ServeFuture` type, and shared wasm ABI helpers. `native` has `RingStorage`, `spawn_worker_loop`, `run_worker_loop`, `spawn_async_worker_loop`, `run_async_worker_loop`, `AsyncWorkerTransport`, `Oneshot`, and event rings. |
| `afterglow-rpc-macros` | `#[rpc]` proc macro: generates the server trait, typed Rust client, dispatch, native spawn, thin wasm exports, **and a typed TypeScript client**. Supports both sync (`fn`) and async (`async fn`) methods — async uses the poll model (task_id framing, completion queue, `async-executor`). |
| `afterglow-rpc-demo` | Demo `Physics` service + `bench_rpc` stress test. |
| `afterglow-assets-worker` | Asset loader worker: `#[rpc(worker = AssetLoaderWorker)]` with `async fn load(path) -> RpcResult<Vec<u8>>`. Uses the async `#[rpc]` poll model + `async-executor`. Native only (reads disk via `FsSource`); web asset loading goes through the serving layer (fetch + Range). |
| `afterglow-assets` | Shared asset-path/MIME helpers (`AssetRoot`, `decode_url_path`, `guess_mime`, `resolve`) for the CEF scheme handler and web dev server. Plus streaming `AssetSource` trait + `FsSource`/`BytesSource` + `Range` parser. No deps or file-content reads in the confinement module; `FsSource` owns streaming reads via `pread`. The single security boundary for FS asset confinement. |
| `afterglow-web` | Wasm target: page-side `#[no_mangle]` exports (`write_frame`, `read_response`) over two static rings in shared wasm memory. `worker.js` accesses the opposite halves directly with tested helpers from `ring-buf.js`. No wasm-bindgen. Includes a real worker benchmark + `coep_server` example. |
| `afterglow-cef` | Thin CEF shell: window + WebGPU flags + `afterglow://` scheme (embedded-first / FS-fallback assets via `afterglow-assets`) + COOP/COEP headers. No worker code, no IPC, no input. |
| `latency-tool` | CDP-based input→present latency measurement. |
| `xtask` | Build orchestrator: `build`, `wasm`, `check`, `test`, `bench`. |

### CEF shell (`afterglow-cef`)

Thin wrapper — does NOT contain worker or communication code:
- Windowed rendering (Views framework)
- WebGPU + Vulkan forced on the real GPU
- X11/XWayland (`--ozone-platform=x11`; Wayland+Vulkan incompatible in CEF 149)
- `afterglow://local/` custom scheme (standard + secure + CORS + fetch + CSP-bypass) serving **embedded-first / FS-fallback assets via `afterglow-assets`** — no localhost HTTP server
- **COOP/COEP headers** on the scheme handler — enables `SharedArrayBuffer`
- DevTools on a port; JS console forwarded to a callback

### Web target build

```sh
# .cargo/config.toml at workspace root:
# [target.wasm32-unknown-unknown]
# rustflags = ['-C', 'target-feature=+atomics,+bulk-memory,+mutable-globals',
#              '-C', 'link-arg=--import-memory',
#              '-C', 'link-arg=--max-memory=67108864',
#              '-C', 'link-arg=--shared-memory']

cargo build -p afterglow-web \
  --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort \
  --profile wasm-dev
```

The `.cargo/config.toml` at the workspace root applies `--import-memory` +
`--shared-memory` so the wasm module uses JS-provided shared memory. Without
this, the module creates its own non-shared memory and `SharedArrayBuffer`
won't work.

### NixOS / shell.nix

- `shell.nix` sets up `CEF_PATH`, `DISPLAY`, runtime libs (libvulkan, etc.)
- `nix-shell shell.nix --run "cargo ..."` for all builds
- CEF runtime libs are in `target/debug/` (copied by the cef-dll-sys build script)
- Spawning threads before `execute_process` crashes the GPU process — always
  use `on_ready` callback (or spawn from within the CEF message loop)

## Debugging

### CEF app won't start / GPU process crash

1. Check `--ozone-platform=x11` is passed (Wayland+Vulkan incompatible in CEF 149)
2. Check `CEF_PATH` env var points to the right CEF distro
3. Check no threads are spawned before `execute_process` (causes GPU crash)
4. Check `shell.nix` is sourced (provides libvulkan etc.)
5. Check `libudev` version warning is harmless (libudev-zero)

### SharedArrayBuffer not available

- Page must be served with COOP/COEP headers:
  - `Cross-Origin-Opener-Policy: same-origin`
  - `Cross-Origin-Embedder-Policy: require-corp`
- On CEF: these are set by `resources.rs` `response_headers`
- On web: use `coep_server` example or configure your web server
- Check `self.crossOriginIsolated === true` in JS console

### Wasm module doesn't share memory

- The `.cargo/config.toml` at the WORKSPACE ROOT must have `--import-memory`
  and `--shared-memory` flags. A `.cargo/config.toml` inside a subcrate is
  NOT found by cargo (cargo searches UP from cwd, not DOWN).
- JS must create `WebAssembly.Memory({ shared: true, ... })` and pass it as
  `env.memory` when instantiating the module.
- The JS memory's `maximum` must be ≤ the module's `--max-memory`.

### `v8_value_create_array_buffer` returns None

This is the **V8 sandbox** — compiled into CEF 149, not toggleable at runtime.
`CefV8Value::CreateArrayBuffer` (external backing store) always returns nullptr.
Use `CreateArrayBufferWithCopy` instead (one memcpy). This only affects the
CEF native path; the web `SharedArrayBuffer` path has no such issue.

### `latency-tool eval` times out

- The page might be running a synchronous JS task. Use `awaitPromise: true`
  and ensure the JS yields to the event loop.
- CEF Views browsers don't appear in `/json/list`. The latency-tool uses
  `Target.getTargets` + `Target.attachToTarget` instead.
- Navigate via `latency-tool nav <url>` or `latency-tool eval 'location.href=...'`

## Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Agent must always keep the user-facing mdBook (`book/`) in sync with engine changes — when a crate's public API, behavior, flags, build steps, or performance numbers change, update the relevant `book/src/` chapter in the same change. The book is the front door; `docs/api/` is the source-checked reference. When the two disagree, `docs/api/` is canonical — fix the book. Build/serve locally with `cd book && nix-shell -p mdbook mdbook-mermaid --run "mdbook serve --open"`
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
- Legacy code is bad; delete legacy code, embrace new code and systems
- From time to time, spawn a subagent to look at the code and suggest cleanups — you might have left a mess
- Always clean up temporary files
- KISS and YAGNI

## Research

- `docs/research/threejs-webgpu-stability.md` — How stable and usable is the
  Three.js WebGPU renderer? (investigated 2026-07)
- `docs/research/native-runtime-linux-steam.md` — Native runtime options to ship
  the web engine on Linux + Steam. Verdict: Electron (bundles Chromium →
  WebGPU) for desktop/Steam; `react-native-webgpu` (Dawn) for iOS/Android/macOS
  via React Three Fiber; Tauri/Neutralino NOT viable on Linux (WebKitGTK lacks
  WebGPU). Includes a multi-target strategy.
- `docs/research/lightweight-rust-chromium-shell.md` — Is there a lightweight,
  Rust-based, CEF/Chromium Electron-like? No mature one: CEF isn't lightweight
  (~100MB+, `wef` abandoned it for that), Rust CEF bindings are stale, Servo is
  the only light+Rust+WebGPU option but unproven for Three.js.
- `docs/research/servo-threejs-status.md` — Deep-dive: Servo canNOT run Three.js
  today. WebGL path has years-old unfixed bugs; WebGPU path is incomplete (missing
  methods, UB, crashes, broken CTS); a real WebGPU game (SpookyBall) doesn't run;
  0 reports of Three.js *WebGPU* on Servo. Revisit in ~2 yrs.
- `docs/research/cef-wayland-vulkan-webgpu.md` — CEF graphics on Linux: YES for
  windowed (Electron-style, Three.js full-window) on native Wayland+Vulkan+WebGPU
  via flags; NO for OSR/webview-as-texture overlay (forced to X11). Blockers are
  non-graphics (size ~100MB+, immature Rust bindings, build complexity) — offers
  no advantage over Electron for this stack.
- `docs/research/cef-rs-tauri-binding.md` — **CORRECTION:** there IS a mature
  Rust CEF binding — `tauri-apps/cef-rs` (crates `cef`+`cef-dll-sys`),
  Tauri-team-maintained, 408★, 130k dl, Chromium 149, Linux x86_64+ARM64. This is
  what `bevy_cef` uses. Foundation for a future wry/Tauri CEF backend → native
  WebGPU on Linux. Revised native-shell recommendation: use cef-rs windowed now.
- `docs/research/cef-rs-webgpu-prototype-findings.md` — **Built & ran a cef-rs
  WebGPU prototype.** ✅ WebGPU works through cef-rs on Linux (NVIDIA/Ampere via
  Dawn→Vulkan). Empirical gotchas: NixOS runtime-lib wiring (shell.nix),
  CEF-API-version must match (don't reuse stale CEF_PATH), must prefer system
  libvulkan + real ICD over CEF's bundled swiftshader. ⚠️ CORRECTION: Wayland+
  Vulkan are INCOMPATIBLE in CEF 149 — must use --ozone-platform=x11 (XWayland)
  for WebGPU; native Wayland+WebGPU not available yet. See `prototype/cef-webgpu/`.
- `docs/research/cef-games-latency-footprint-debugging.md` — CEF for games:
  real-world usage (Steam, GW2/ArenaNet ~3× faster than CoherentUI, Battle.net/
  Epic, Coherent Gameface), input→pixel latency pipeline (our windowed
  architecture sidesteps the OSR-texture-copy latency), latency flags
  (--disable-gpu-vsync/--disable-frame-rate-limit/etc., + vsync-reset caveat),
  footprint (Minimal dist + strip + en-US locale ~80-110MB floor, can't
  feature-strip), debugging (remote-debugging-port + chrome://tracing +
  crashpad), cef-rs accelerated_osr zero-copy path.
- `docs/research/performance-benchmarks.md` — Optimized communication results
  (2026-07-10): 64B service RPC is 2.4µs native vs 10.9µs web; 64KiB
  service RPC is 76.4µs / 1,636 MiB/s native vs 106.5µs / 1,174 MiB/s web.
  The web worker has separate wasm memory, so calls copy SAB→worker wasm→SAB.
  Historical input→present: 1.16ms median @ 144fps (not rerun). Run
  `cargo run --release --example bench_rpc -p afterglow-rpc-demo`.
- `docs/research/steam-overlay-cef.md` — How the Steam Overlay works (hooks
  Present/SwapBuffers/vkQueuePresentKHR in the game process), why it doesn't
  work with CEF multi-process GPU, and how to fix it (`--in-process-gpu` flag
  + `SteamAPI_Init` before CEF init).
- `docs/research/steamworks-native-worker.md` — Steamworks as a native Rust
  worker via `#[rpc(worker = SteamWorker)]`. `Client::init()` before CEF,
  `run_callbacks()` in the worker loop, overlay events via `push_event`,
  `--in-process-gpu` flag, `steam_appid.txt`.

## API docs

- `book/` — the user-facing mdBook (introductory front door). `book/src/SUMMARY.md`
  is the table of contents; build/serve with `cd book && nix-shell -p mdbook
  mdbook-mermaid --run "mdbook serve --open"`. Kept in sync with engine changes.
- `docs/api/ring-buffer.md` — `afterglow-rpc` ring buffer + native transport
  (SPSC framing, owned halves, worker transport, events, poison/timeout).
- `docs/api/rpc-macro.md` — `afterglow-rpc-macros` `#[rpc]` attribute: server/
  typed client/dispatch generation, native spawn + wasm exports, reserved names,
  TS client generation, async `#[rpc]` poll model.
- `docs/api/asset-system.md` — `afterglow-assets` streaming `AssetSource` +
  `FsSource`/`BytesSource` + range parser, serving layer (CEF scheme + web HTTP),
  `afterglow-assets-worker` async asset loader, async `#[rpc]` transport.
- `docs/api/web-shared-memory.md` — `afterglow-web` wasm exports, JS client/
  worker contract, build, and COOP/COEP headers.
- `docs/api/assets.md` — `afterglow-assets` shared `guess_mime`/`resolve`:
  path/MIME + canonical confinement for the CEF scheme and web dev server.
- `docs/api/cef-shell.md` — `afterglow-cef` game-window shell: `AppBuilder`,
  `afterglow://` scheme, WebGPU/X11 flags, COOP/COEP, console, startup caveat.
- `docs/api/latency-tool.md` — CDP diagnostic CLI commands and measurement semantics.

## Benchmarks

### fox-laptop (2026-07-12)

Hardware: AMD Ryzen 7 6800U (16 threads), AMD Radeon 680M (RADV Rembrandt,
integrated), 14 GB RAM. Panel: 2880×1800 eDP, GNOME 200% scaling → 1440×900
logical native.

Configuration: medium (5,000 instanced entities), 1440×900 window (native
logical), vsync on.

Frame timing measured via `requestAnimationFrame` intervals (CDP
`Runtime.evaluate` with `awaitPromise`). 600 consecutive frames sampled.

**Note:** CDP trace-based `SkiaRenderer::SwapBuffers` counting (the
latency-tool's default mode) undercounts swaps on RADV — the GPU process
does not emit a SwapBuffers trace event for every frame on AMD iGPUs.
Direct rAF frame timing is the reliable measurement method on this platform.

| Metric | Value |
|--------|-------|
| Average FPS | 60.0 |
| p50 frame time | 16.68 ms (60.0 FPS) |
| p90 frame time | 16.68 ms (60.0 FPS) |
| p99 frame time | 16.68 ms (60.0 FPS) |
| Max frame time | 16.68 ms (60.0 FPS) |
| Frames below 55 FPS | 0 / 600 |
| Frames above 17 ms | 0 / 600 |

Targets met: ✅ 60 FPS steady (vsync-locked), ✅ p99 = 60.0 FPS (≥ 55 FPS).
Every single frame hit the 16.68 ms vsync budget — zero drops across 600
frames (10 seconds at 60 Hz).

The demo (`engine-demo.html`) uses 5,000 entities at medium; the CEF window
is 1440×900 (native logical at the panel's 200% desktop scaling). Run:

```sh
nix-shell shell.nix --run "cargo build --example minimal -p afterglow-cef"
DISPLAY=:0 XAUTHORITY=/run/user/$(id -u)/.mutter-Xwaylandauth.* \
  nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
# In another terminal, measure frame timing (DevTools on port 9222):
./target/release/latency-tool eval '(async()=>{const f=[];let p=-1;await new Promise(r=>{function l(t){if(p>=0)f.push(t-p);p=t;if(f.length<600)requestAnimationFrame(l);else r()}requestAnimationFrame(l)});const s=[...f].sort((a,b)=>a-b);return JSON.stringify({n:f.length,fps:(1000/(s.reduce((q,v)=>q+v,0)/s.length)).toFixed(1),p99:s[s.length*0.99|0].toFixed(2),max:s[s.length-1].toFixed(2),below55:f.filter(x=>x>1000/55).length})})()' 127.0.0.1:9222
```
