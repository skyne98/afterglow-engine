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
+ `AtomicU32` (Acquire/Release). Zero serialization, zero IPC per call. This is
the ONLY communication mechanism for website↔worker and worker↔worker.

Two backends, same `RingBuffer`:

| Target | Worker type | Memory backing | Crate |
|--------|-------------|----------------|-------|
| Native (CEF) | Native thread (`std::thread`) | Heap (`Arc<Vec<u8>>`) | `afterglow-rpc::native` |
| Web | Web Worker (wasm) | `SharedArrayBuffer` | `afterglow-web` |

- **Native**: workers are real OS threads with full `std`, no wasm overhead.
  `afterglow_rpc::native::spawn_worker` creates two ring buffers (`Arc<Vec<u8>>`),
  spawns the worker thread, returns a client. `run_worker_loop` drains the
  request ring buffer and writes responses.
- **Web**: workers are Web Workers running compiled wasm. JS creates a
  `SharedArrayBuffer`-backed `WebAssembly.Memory` (`shared: true`), instantiates
  the wasm module with `--import-memory`, shares module+memory with Web Workers
  via `postMessage`. The `RingBuffer` operates on the shared wasm memory.

### Crate structure

| Crate | Purpose |
|-------|---------|
| `afterglow-rpc` | Core: `RingBuffer`, `RingBufferTransport`, `Transport` trait, postcard codec, `#[rpc]` macro, schema. `native` module has `spawn_worker` + `run_worker_loop`. |
| `afterglow-rpc-macros` | `#[rpc]` proc macro: generates server trait, client, dispatch, schema. |
| `afterglow-rpc-demo` | Demo `Physics` service + `bench_rpc` stress test + `dump-schema` bin. |
| `afterglow-web` | Wasm target: `#[no_mangle]` exports (`write_frame`, `read_frame`, `write_response`, `read_response`, `has_data`, `has_response`) on two static ring buffers in shared wasm memory. No wasm-bindgen. Includes `bench.html` (stress test) + `coep_server` example. |
| `afterglow-cef` | Thin CEF shell: window + WebGPU flags + `afterglow://` scheme + COOP/COEP headers. No worker code, no IPC, no input. |
| `latency-tool` | CDP-based input→present latency measurement. |
| `xtask` | Build orchestrator: `build`, `wasm`, `check`, `test`, `bench`. |

### CEF shell (`afterglow-cef`)

Thin wrapper — does NOT contain worker or communication code:
- Windowed rendering (Views framework)
- WebGPU + Vulkan forced on the real GPU
- X11/XWayland (`--ozone-platform=x11`; Wayland+Vulkan incompatible in CEF 149)
- `afterglow://local/` custom scheme (standard + secure + CORS + fetch + CSP-bypass)
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
- `docs/research/performance-benchmarks.md` — Ring buffer stress test results:
  native (heap + native threads) ~3µs/64B call, 73 MB/s (postcard limited);
  web (SAB + Web Workers) ~4µs/64B, true zero-copy. Input→present 3.71ms
  median @ 144fps. Run `cargo run --example bench_rpc -p afterglow-rpc-demo`.

## API docs

- `docs/api/web-shared-memory.md` — `afterglow-web` wasm exports, JS interface,
  build instructions, two-backend architecture (native heap + web SAB).
