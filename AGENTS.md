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
| `afterglow-basis-encoder` | Offline-only official Basis Universal C++ UASTC encoder used by `afterglow-pipeline`; never linked into runtime or wasm crates. |
| `afterglow-pipeline` | Offline cook: confines and embeds external glTF packages, packs self-contained GLBs, extracts images into paged/UASTC VTs, emits R16 displacement, and writes seekable `.big` v5 containers. |
| `afterglow-assets-worker` | Asset loader worker: `#[rpc(worker = AssetLoaderWorker)]` with `async fn load(path) -> RpcResult<Vec<u8>>`. Uses the async `#[rpc]` poll model + `async-executor`. Native only (reads disk via `FsSource`); web asset loading goes through the serving layer (fetch + Range). |
| `afterglow-assets` | Shared asset-path/MIME helpers (`AssetRoot`, `decode_url_path`, `guess_mime`, `resolve`) for the CEF scheme handler and web dev server. Plus streaming `AssetSource` trait + `FsSource`/`BytesSource` + `Range` parser. No deps or file-content reads in the confinement module; `FsSource` owns streaming reads via `pread`. The single security boundary for FS asset confinement. |
| `afterglow-web` | Wasm target + authored TypeScript runtime: shared-ring worker bridge, fixed runtime storage, packed-GLB AssetStore loading with rig-preserving runtime meshopt, VT sampling/feedback/material adapters, and demos. No wasm-bindgen. |
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

### fox-laptop (Radeon 680M) CEF/WebGPU validation

**Never accept WebGL fallback.** `afterglow-cef` passes `--disable-webgl`, and
all authored renderer startup paths use `engine/webgpu-only.ts`: it clears
Three r185's internal fallback callback before `init()`, requires a live WebGPU
backend, and displays a fatal error after startup failure or device loss. A
visible window/cubes plus a failure panel is a failed run, not an acceptable
fallback.

- **Validated stack:** on this Fedora 44 laptop `shell.nix` must use the
  default Nix Vulkan loader + Mesa 25.3.4 RADV ICD. The host Mesa 26.1.4 RADV
  crashes CEF 149's GPU process with `SIGFPE` in
  `radv_clear_dcc_comp_to_single` during a Skia Vulkan clear. Do not set
  `AFTERGLOW_VULKAN_STACK=host` except to investigate that driver regression.
  NixOS instead uses `/run/opengl-driver/lib` and its matching ICDs.
- **Build and launch:**

  ```sh
  cd ~/dev/afterglow-engine
  nix-shell shell.nix --run "cargo build --release --example minimal -p afterglow-cef"
  XA=$(ls /run/user/1000/.mutter-Xwaylandauth.* | head -1)
  setsid env DISPLAY=:0 XAUTHORITY="$XA" nix-shell shell.nix --run \
    "./target/release/examples/minimal --ozone-platform=x11" \
    </dev/null >/tmp/cef-minimal.log 2>&1 &
  ```

- **Prove hardware WebGPU after launch** (the minimal example exposes DevTools
  on 9222); require `amd` / `rdna-2`, no fallback/crash log lines, and visible
  cubes:

  ```sh
  ./target/release/latency-tool eval \
    '(async()=>{const a=await navigator.gpu.requestAdapter();return JSON.stringify(a&&a.info)})()' \
    127.0.0.1:9222
  ! grep -E 'GPU process exited|WebGPU is not available' /tmp/cef-minimal.log
  ```

  A null/non-AMD adapter, `GPU process exited unexpectedly: exit_code=136`, or
  `WebGPU is not available, running under WebGL2 backend` is a failed run;
  stop and fix the Vulkan stack before accepting any FPS result.
- **POM evaluation result (2026-07-16, 1440×900 logical at DPR 2 =
  2880×1800 physical):** low core POM (8–32 layers; no silhouette,
  self-shadow, or relief shadow pass) held **60.0 FPS, p99 16.68 ms,
  0/300 below 60**. Full low POM was 51.6 FPS; medium was 36.1 FPS; high was
  22.2 FPS. Therefore only low core POM is viable as a strict, distance/coverage
  LOD tier on the 680M; medium/high/full-POM are evaluation modes, not an engine
  frame-budget candidate without a different rendering strategy. The corrected
  Dungeon tier uses official resident 1K, 16-bit displacement, a geometric TBN,
  8–32 layers, scale 0.05, an 8-step point-light self-shadow ray, no radial
  per-fragment fade, and VT coarser-page/tail fallback for all displaced PBR
  channels. Displacement must be converted by `afterglow-pipeline height-r16`
  and uploaded as single-channel `RedFormat + FloatType` / WebGPU `r32float`
  from exact normalized R16 samples; browser image loading silently truncates it
  and is forbidden. `r16unorm` is unfilterable and incompatible with Three r185's
  custom-function binding path, so runtime requires `float32-filterable`. VT
  feedback converts reduced-pass derivatives with the exact feedback/display
  pixel-scale vector; quality bias 0 targets 1–2 source texels per physical
  pixel. POM feedback marches to displaced UV while retaining base-UV gradients.
  The historical corrected
  fixed close-wall run measured 59.87 FPS, p99 16.68 ms, 1/600 below 55; the
  prewarmed base measured 59.77 FPS, p99 16.68 ms, 2/600 below 55. Those runs
  sampled `rgba8unorm` despite 16-bit source PNGs, so a fresh R16 GPU run is
  required before citing them for the current runtime path. Feedback cadence is
  8 frames; the prior 4-frame cadence measured 3/300 misses.

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

### Development and design philosophy

- Prefer small, generic, composable engine primitives over subsystem-specific
  infrastructure. If a capability can cache, persist, stream, queue, or store
  arbitrary bytes/items, keep that primitive policy-free and reusable; texture,
  mesh, shader, device, and asset semantics belong in thin consumers that build
  keys/namespaces and interpret values.
- Keep public primitives KISS: minimal operations, explicit bounded capacities,
  deterministic failure, stable telemetry, and one clear ownership model. Do
  not add speculative layers, parallel legacy paths, or convenience APIs that
  obscure allocation, I/O, synchronization, or invalidation behavior.
- Separate mechanism from policy. For example, persistent storage provides
  generic `open/get/put/clear/stats` byte operations; virtual texturing decides
  adapter/source cache identity, page keys, and when reads/writes are useful.
- Visual demos and examples contain pure game/presentation code and consume only
  public engine APIs. They never assemble RPC transports or generated clients,
  patch renderer/device methods, parse engine containers, or own engine worker
  lifecycles. If a demo exposes missing infrastructure, add or compose a small,
  generic engine primitive—never hide the leak behind a single-use subsystem
  wrapper or a feature-specific `*Session` abstraction.

### Runtime allocation, complexity, and frame budgets

The canonical migration plan is
`docs/implementation/no-runtime-allocation-constant-time-budget-plan.md`.
All engine work must move toward these non-negotiable requirements:

- After bootstrap/warm-up enters `GameplaySealed`, engine-authored hot paths
  must not perform general-purpose runtime allocation. Only game code may
  allocate freely. Unavoidable browser/Three.js/Fetch allocation must occur
  behind an explicit, tracked, budgeted slow-path permit.
- All intentional engine memory must come through the single logical
  `EngineMemory` ECS resource: fixed-capacity arenas, pools, rings, generational
  handles, byte/item limits, high-water metrics, and deterministic overflow.
  Each worker has a corresponding tracked memory domain because workers do not
  share an address space.
- All authored web engine, runtime, RPC, worker, and demo source must be
  TypeScript. HTML contains markup/style and external generated-script tags,
  never inline authored JavaScript. `crates/afterglow-web/web/` separates
  `src/engine/<subsystem>`, `src/workers`, `src/demos/<name>`, `public`,
  `assets`, and `contracts`. `www/` is a disposable generated deployment tree:
  never place authored source, tests, package state, manifests, or vendored
  libraries there, and never edit its generated `.js` directly.
  Authored `.ts` files must import authored modules via `.ts` specifiers, never
  via generated `.js` artifact paths; `scripts/build-web.ts` enforces this. Run `bun scripts/build-web.ts` and enforce
  drift with `bun scripts/build-web.ts --check`.
- Maintain allocation hygiene with mandatory custom linting for hot TypeScript
  paths and tracked-global-allocator/no-allocation tests for sealed Rust worker
  loops. Ban hidden frame allocations such as promises, closures, object/array
  literals, dynamic strings, growing Map/Set entries, array transforms, and new
  typed-array views in hot paths.
- Hot lookup, cache, queue, handle, and free-list operations must be O(1)
  worst-case where practical, otherwise fixed-capacity bounded O(1) amortized.
  Do not use frame-time scans, sorts, array shifts/splices, linear searches, or
  rebuilt indexes whose cost grows with world/cache age or occupancy.
- Potentially stalling work must be lazy, incremental, cancelable, stale-aware,
  and controlled by explicit per-stage time, operation-count, and byte budgets.
  Every queue has a hard capacity; no async stage may accumulate unbounded work.
- Long-running sealed-mode soak tests must prove that heap usage, queue depths,
  timers, pending tasks, cache cost, and frame/GPU timings plateau. Short rAF
  tests are not evidence of presentation stability.

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
- `docs/research/device-transcoded-texture-cache.md` — Industry comparison of
  runtime Basis/KTX2 transcoding versus cooked/derived caches, plus Afterglow's
  generic persistent-cache and device/source namespace design.
- `docs/research/surface-detail-low-end-fallbacks.md` — Surface-detail/POM
  fallback evaluation and integrated result: normal/one-tap fallback tiers,
  measured low-core 680M boundary, resident matching height, and bounded VT
  displaced-page composition.
- `docs/research/steam-overlay-cef.md` — How the Steam Overlay works (hooks
  Present/SwapBuffers/vkQueuePresentKHR in the game process), why it doesn't
  work with CEF multi-process GPU, and how to fix it (`--in-process-gpu` flag
  + `SteamAPI_Init` before CEF init).
- `docs/research/steamworks-native-worker.md` — Steamworks as a native Rust
  worker via `#[rpc(worker = SteamWorker)]`. `Client::init()` before CEF,
  `run_callbacks()` in the worker loop, overlay events via `push_event`,
  `--in-process-gpu` flag, `steam_appid.txt`.
- `docs/research/steam-audio-browser.md` — Steam Audio's experimental WASM
  target supports fully dynamic no-bake acoustics after rebuilding 4.8.1 and all
  dependencies for a fixed two-worker Emscripten pthread pool. Ray counts are
  global listener rays per
  update, not per source. On the 6800U, low 1,024×2 reflections measured 1.70 ms
  worst p99; medium 4,096×4 measured 10.95 ms for one source,
  33.43 ms for eight, and 114.83 ms for 32 at the original medium tier. The
  unlocked built-in sweep selected 128 direct-ray+HRTF sources with 64 priority
  reflection slots, 512 global rays × 2 bounces, 500 ms parametric/order 0, at
  30 Hz steady / 60 Hz burst: 15.25 ms reflection p99 and projected 1.364/2.667
  ms for 64 reflections + 128 nearest HRTFs. The selected web tracer is now
  `obvhs` 0.3.2 through all four `IPL_SCENETYPE_CUSTOM` callbacks. Its local
  four-child WASM SIMD128 kernel plus two pre-created pthreads measured 4.47 ms
  mean / 6.235 ms worst p99 over five fresh laptop launches, 0/5 over 60 Hz,
  valid dynamic IRs, 1,261 nodes, and 661,048 owned bytes. Scalar one-thread
  obvhs was 12.27 ms / 13.365 ms; SIMD+threads reduced mean 63.6%. Reflecting 96 is aggressive; reflecting all
  128 used 94% of the quantum and was rejected. The matching native OS-worker
  sweep selected two Steam simulation threads: 64-source 512×2 reflections were
  9.27 ms mean / 10.74 ms p99, with projected 1.433/2.667 ms DSP for 64
  reflections + 128 HRTFs. Four simulation threads reached 6.44 ms p99 or fit
  1,024×2 at 12.52 ms p99; per-source DSP remains serial. All official Bistro
  package scenes were tested with both the built-in tracer and Steam Audio
  4.8.1's embedded Embree 4.4. Embree is mandatory for native: on two simulation
  threads, package-worst p99 was 3.90 ms at 512×2 and 6.93 ms at 1,024×2; matching
  four-thread mean simulation improved 18.8–22.9×. Exterior BVH build fell from
  7.81 s to 0.52 s, but resident scene memory rose from ~375 MiB to ~486 MiB.
  Full render meshes are no longer a traversal blocker but remain forbidden as
  production acoustic geometry due memory and irrelevant detail; cook structural
  proxies. Web obvhs keeps static geometry and a translated door BLAS in one
  Rust-owned Emscripten memory domain; fixed-stack queries allocate nothing.
  The same medium-build, ray-batch-64, two-thread, 64-source 512×2 obvhs
  configuration works natively (five-launch laptop result: 3.39 ms mean / 4.316
  ms worst p99) and on web. Render-loaded and actual device-callback validation remain
  open.

## API docs

- `book/` — the user-facing mdBook (introductory front door). `book/src/SUMMARY.md`
  is the table of contents; build/serve with `cd book && nix-shell -p mdbook
  mdbook-mermaid --run "mdbook serve --open"`. Kept in sync with engine changes.
- `docs/api/engine-memory.md` — sealed runtime phases, fixed arenas/pools,
  resource sealing, TypeScript artifact enforcement, and allocation linting.
- `docs/api/persistent-blob-cache.md` — generic bounded OPFS byte cache:
  append-only pack/fixed SHA-256 index, crash semantics, limits, telemetry, and
  policy-free composition by VT or other systems.
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
- `docs/api/frame-budget.md` — staged frame admission, timing, and deferral counters.
- `docs/api/hierarchy.md` — fixed linked topology and incremental double-buffered rebuild.
- `docs/api/renderer-sealing.md` — descriptor pools, bounded renderer slices, warm-up, and pipeline seal.
- `docs/api/virtual-texturing.md` — bounded VT residency, scheduling, shaders,
  linked PBR materials, tuning, telemetry, and demos.
- `docs/api/pom.md` — bounded low-core POM, lossless resident R16 height fields,
  displaced VT fallback, Dungeon controls, and 680M validation.
- `docs/api/relative-pointer.md` — raw relative pointer events and unadjusted
  pointer-lock fallback.
- `docs/api/allocation-boundaries.md` — unavoidable browser/Three/codec boundaries.
- `docs/api/runtime-capacities.md` — canonical capacities and degradation behavior.
- `docs/api/steam-audio.md` — selected native Embree and web obvhs scene backends,
  custom callback ABI, ownership, dynamic instances, capacities, and validation.
- `docs/api/testing.md` — canonical unit, vertical-integration, browser/GPU, and release-evidence test lanes.

## Benchmarks

### fox-laptop (2026-07-12)

Hardware: AMD Ryzen 7 6800U (16 threads), AMD Radeon 680M (RADV Rembrandt,
integrated), 14 GB RAM. Panel: 2880×1800 eDP, GNOME 200% scaling → 1440×900
logical native.

Configuration: medium (5,000 instanced entities), 1440×900 window (native
logical), vsync on.

Frame timing measured via `requestAnimationFrame` timestamps (CDP
`Runtime.evaluate` with `awaitPromise`). 600 consecutive frames sampled.

**Method validity:** rAF timestamp intervals are the standard frame-rate
measurement used by Chrome DevTools' own FPS counter, web FPS testers, and
MDN ("the frequency of calls to the callback function will generally match
the display refresh rate"). The timestamp is a `DOMHighResTimeStamp` with
sub-millisecond precision. It measures main-thread frame *production* rate
(JS compute + Three.js render commands + matrix compose) — whether the main
thread stays within the vsync budget. If it overflows 16.67 ms, rAF fires
less often and the interval increases. It does NOT measure GPU presentation
time (when pixels reach the display) or input→present latency.

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

**Screen lock / OLED protection:** GNOME's screen lock suspends the
compositor, so rAF fires at ~1 Hz regardless of Chrome background-throttling
flags (`--disable-background-timer-throttling`, etc. were added to `flags.rs`
but cannot override compositor suspension). Setting backlight brightness to 0
triggers display/GPU power-saving and drops FPS to ~10. For OLED-safe
benchmarking: run short benchmarks (5–10 seconds) at normal brightness,
then lock the screen. The demo renders a dark background (#0a0c10) with
moving entities, so burn-in risk from a 10-second run is negligible.

The demo (`engine-demo.html`) uses 5,000 entities at medium; the CEF window
is 1440×900 (native logical at the panel's 200% desktop scaling). The demo
has a fixed-capacity **built-in frame benchmark**: load `?bench=300` to run a
300-frame rAF timing benchmark (p50/p90/p99/max + dropped-frame count).
Capture uses preallocated typed arrays; sorting/formatting occurs once after the
sample. Results appear in the JS console (`[bench]` prefix). The
`web/src/engine/diagnostics/bench.ts` module is the reusable engine API (`FrameBench` class +
`formatBenchResults`). Run:

```sh
nix-shell shell.nix --run "cargo build --example minimal -p afterglow-cef"
DISPLAY=:0 XAUTHORITY=/run/user/$(id -u)/.mutter-Xwaylandauth.* \
  nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
# Load ?bench=300. DevTools is on port 9222.
# For headless/OLED-safe measurement via CDP:
./target/release/latency-tool eval '(async()=>{const f=[];let p=-1;await new Promise(r=>{function l(t){if(p>=0)f.push(t-p);p=t;if(f.length<300)requestAnimationFrame(l);else r()}requestAnimationFrame(l)});const s=[...f].sort((a,b)=>a-b);return JSON.stringify({n:f.length,fps:(1000/(s.reduce((q,v)=>q+v,0)/s.length)).toFixed(1),p99:s[s.length*0.99|0].toFixed(2),max:s[s.length-1].toFixed(2),below55:f.filter(x=>x>1000/55).length})})()' 127.0.0.1:9222
```

### Virtual-texture GPU validation and frame timing (2026-07-12)

The 256K procedural VT demo was validated on fox-laptop (Ryzen 7 6800U,
Radeon 680M/RADV, 1440×900 logical CEF window). The session was explicitly
unlocked for measurement and locked immediately afterward; locked GNOME
sessions throttle rAF and invalidate results.

Real-GPU validation ran three independent CEF launches. Every launch exercised
east/west/rotated RG32Uint feedback, eastbound/westbound/diagonal LOD
trajectories, three byte-verified RGBA subregion uploads, and three BC7
subregion uploads. All 9,216 feedback pixels passed; no WebGPU errors occurred.

Frame timing uses 600 consecutive rAF timestamp intervals per scenario:

| Scenario | FPS | p50 | p90 | p99 | max | Frames below 55 FPS |
|---|---:|---:|---:|---:|---:|---:|
| Stable VT | 59.97 | 16.675 ms | 16.680 ms | 16.680 ms | 16.680 ms | 0/600 |
| Bidirectional pan | 59.97 | 16.675 ms | 16.680 ms | 16.680 ms | 16.680 ms | 0/600 |
| Continuous overview streaming | 59.97 | 16.675 ms | 16.680 ms | 16.680 ms | 16.680 ms | 0/600 |
| 12-way teleport every frame | 59.87 | 16.675 ms | 16.680 ms | 16.680 ms | 33.350 ms | 1/600 |
| Full-cache 20-way thrash, 4 camera updates/frame | 59.87 | 16.675 ms | 16.680 ms | 16.680 ms | 33.355 ms | 1/600 |

Normal rendering and streaming had zero dropped frames. Deliberately impossible
camera teleport/thrash workloads each missed one vsync out of 600 while retaining
p99 at 16.68 ms and producing no GPU validation errors. Run the validation with
`DISPLAY=:0 ./scripts/test-vt-gpu.sh` while the session is unlocked.

### Sealed VT runtime validation (2026-07-16)

The `.big` v5 dungeon header is 123,768 bytes (v4: 764,192). Atlas baselines at
144 Hz reached 1,896/3,600 half occupancy and 3,600/3,600 full occupancy in
~9.3 seconds each. Full-cache replacement produced 1,014 cumulative evictions
in 4.92 seconds; mean/max rAF were 6.970/20.850 ms with one interval above
17 ms, zero failed loads/queue overflow/long tasks/GPU errors. Full-state GPU
timestamps: 0.149 ms main, 0.018 ms feedback, 0.465 ms aggregate render.

Corrected 10/30/60-minute stable/traverse/eight-way-teleport soaks covered
863,264 frames and averaged 6.950 ms in every mode. They ended with zero
pending work, failed loads, queue overflow, long tasks, GPU errors, or post-seal
pipelines. Sixty-second GC-floor heap samples repeatedly returned to ~77–79 MiB.
Raw traces and methodology are in `docs/benchmarks/`.
