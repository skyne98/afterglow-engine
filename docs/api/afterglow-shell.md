# `afterglow-shell` API — native Three.js WebGPU runtime

> Status: the sole native host. `afterglow-cef` has been removed; the shell
> is the native deployment target. The remaining release gates are tracked in
> `docs/implementation/shell-promotion-plan.md`.

## Purpose

`afterglow-shell` runs unmodified Three.js WebGPU applications without embedding
Chromium. It combines:

- winit for the native window and input;
- Deno's rusty_v8 and `deno_webgpu` bindings;
- the same wgpu-core device for JavaScript rendering and native presentation;
- LinkeDOM for the JavaScript DOM;
- Stylo/Taffy/Blitz for CSS, layout, hit testing, and page paint;
- Vello for GPU-rasterized transparent HTML/CSS HUD composition.

The shell is an environment implementation, not an application translator.
Official Three.js HTML examples, module scripts, import maps, addons, and client
code execute verbatim. Missing browser behavior must be implemented in the host
or browser shim; client modules must never be rewritten or replaced with stubs.

## Package and binaries

| Artifact | Purpose |
|---|---|
| `afterglow-shell` | Windowed native presenter. Accepts an optional HTML or module path. |
| `browser_test` example | Deterministic headless compatibility runner that emits PNG bytes. |
| `afterglow_shell` library | Browser bridge, runtime lifecycle, native HUD bridge, and test runner modules. |

Build and run the scheduler unit gate from the workspace root:

```sh
cargo build -p afterglow-shell
cargo build -p afterglow-shell --example browser_test
bun test crates/afterglow-shell/tests/raf.test.ts \
  crates/afterglow-shell/tests/scheduler.test.ts
```

rusty_v8 149.4's SIMDUTF archive can be supplied explicitly when its automatic
download is unavailable:

```sh
curl -L -o /tmp/v149_simdutf.a.gz \
  https://github.com/denoland/rusty_v8/releases/download/v149.4.0/librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz
gzip -dc /tmp/v149_simdutf.a.gz > /tmp/v149.a
RUSTY_V8_ARCHIVE=/tmp/v149.a cargo build -p afterglow-shell
```

## Windowed presenter

Run the bundled native game module:

```sh
cargo run -p afterglow-shell
```

Run an official Three.js document directly:

```sh
cargo run -p afterglow-shell -- /path/to/three.js/examples/webgpu_clearcoat.html
```

Run the generated Dungeon deployment through the native host in release mode:

```sh
cargo run --release -p afterglow-shell -- crates/afterglow-web/www/dungeon.html
```

HTML loading extracts the document's `type="importmap"` and
`type="module"` scripts, resolves `three` and `three/addons/` from that map,
and evaluates the module unchanged. Files, data URLs, blobs, and HTTPS assets
are loaded by the runtime environment.

The presenter creates the native surface through the same wgpu-core `Global`
owned by `deno_webgpu`. JavaScript command buffers render directly to the
surface texture; no second graphics device or full-frame CPU readback is used.
The transparent DOM HUD is emitted as a Vello scene, rasterized on the shared GPU
device with MSAA16, and composited after the game pass.

## Browser environment

The shell currently provides:

- HTML import maps and ES modules;
- WebGPU canvas presentation and external-image texture uploads;
- LinkeDOM DOM mutation and event dispatch;
- computed CSS, layout, box metrics, focus, hover, active state, and scrolling;
- pointer, mouse, wheel, keyboard, pointer capture, pointer lock, and native CSS
  cursors; locked relative motion is always retargeted to the lock element;
  Wayland uses winit locked mode, while X11 falls back to a hidden confined
  pointer plus XInput2 raw-motion events because winit does not implement X11
  `CursorGrabMode::Locked`;
- `ResizeObserver`, `IntersectionObserver`, and `matchMedia`; the final physical
  window size is synchronized once after renderer readiness, even when the
  initial configure event arrived during startup;
- image decode, `createImageBitmap`, Blob, Storage, KTX2/Draco workers, and a
  software Canvas2D environment;
- GPU-rendered text, synthetic bold faces, text shadows, SVG, and page chrome;
- resize debounce, suspend/resume, adapter reporting, and fail-closed device-loss
  handling;
- browser-cadenced `requestAnimationFrame`: a fixed 1,024-callback queue with
  O(1) cancellation, deterministic overflow, shared frame timestamps, and
  deno_core external-operation tracking during top-level awaits;
- the explicitly admitted `scheduler.yield()` subset, implemented as a deferred
  deno/winit task continuation. `scheduler.postTask` and `TaskController` are
  intentionally absent.

The surface canvas remains in layout and hit testing but its placeholder raster
is suppressed from the HUD scene. This preserves unchanged HTML content across
reactive DOM updates without painting over the WebGPU surface.

## Lifecycle

The typed lifecycle is:

```text
Created → EnvironmentReady → AdapterReady → DeviceReady → CanvasReady
        → ResourcesReady → RendererReady → Running
        → Suspended | DeviceLost | Stopped
```

The windowed host is a persistent scheduler: winit remains the outer event
loop, module evaluation advances non-blockingly across redraws, and each host
turn calls `JsRuntime::poll_event_loop()` rather than running deno_core to idle.
A coalesced winit waker handles asynchronous runtime progress. Pending rAF work
references one `ExternalOpsTracker` token, preventing deno_core from falsely
classifying a top-level rAF await as deadlocked. Each bounded host turn yields
once to the current-thread Tokio scheduler so lazy deno ops can complete;
`scheduler.yield()` uses `deno_web::op_defer`, never a timer or microtask.
JavaScript/runtime errors are fatal and are never discarded. Startup defaults
to a 30-second active-time deadline; set `AFTERGLOW_STARTUP_TIMEOUT_MS` to a
positive integer to override it for diagnostics.

The deterministic runner requires a registered canvas and a real
`GPUQueue.submit()` before capture. It does not use sleeps or assume that a
returned animation callback means rendering completed.

## Library modules

| Module | Responsibility |
|---|---|
| `browser` | LinkeDOM snapshot synchronization into Blitz, browser queries, focus/input state, and paint access. |
| `native_browser` | Deno ops and Vello GPU scene extraction for the production presenter. |
| `runtime` | Monotonic clocks, lifecycle transitions, and readiness accounting. |
| `testing` | Deterministic unmodified-example runner and PNG capture path. |

These modules are public for shell integration and validation. They are not yet
a stable game-facing builder API; the command-line presenter is the supported
entry point during the CEF transition.

## Headless validation

```sh
./target/debug/examples/browser_test \
  /tmp/threejs webgpu_multiple_elements /tmp/out.png

cd crates/afterglow-shell/cdp_client
bun ../e2e/diff_pct.js \
  /tmp/out.png \
  /tmp/threejs/examples/screenshots/webgpu_multiple_elements.jpg
```

The CPU Vello path is retained only by this headless runner because PNG output
requires host-readable bytes. Production HUD presentation remains GPU-only.
Runtime-specific golden manifests live under
`crates/afterglow-shell/e2e/runtime_goldens/`.

## Vendored patches

The workspace root patches crates.io dependencies to the shell's pinned copies
of `deno_webgpu`, wgpu, Naga, Blitz, and Stylo/Taffy. The patch inventory and
upstream references are maintained in
`crates/afterglow-shell/vendor/NATIVE_WEBGPU_PATCHES.md`. Blitz is pinned by
`vendor/afterglow-shell-blitz/THREE_NATIVE_PIN` and carries tested
browser-layout/paint fixes.

## Native service composition

`ShellBuilder::with_workers` is the generic application-bootstrap hook. The
shell library provides `WorkerRegistry`, named service metadata, and the Deno op
adapter; it does not register concrete texture, mesh, physics, or game services.
The command-line application explicitly composes its reference workers after
asset-root confinement and before gameplay startup.

Authored TypeScript asks `op_afterglow_worker_ids(service)` for bootstrap-
ordered IDs and constructs generated clients over `NativeRpcTransport`. Worker
numbers therefore belong exclusively to Rust bootstrap. Payload calls still use
the generated SPSC rings.

The native asset worker serves JS-visible `size`/`read` operations in bounded
512 KiB chunks. The reference application composes `min(physical CPU cores,
16)` texture workers; on the current 16-core/32-thread host it publishes 16.
`EngineAssets` bounds consumption of that manifest by its 16-page admission
capacity. Each worker retains a confined generational source handle and performs
BIG range reads plus Basis transcode without exposing encoded page bytes to V8.
Public web remains capped at two to four WASM workers. No native service uses a
Web Worker or WASM implementation.

## Native host status

`afterglow-cef` has been removed. `afterglow-shell` is the sole native host and
native deployment target. Asset-root loading and native texture/mesh worker
composition are implemented. The remaining release work includes packaged-
resource policy, long hardware soaks, input/resize/device-loss evidence, direct
native VT atlas upload evaluation, and Steam integration; these gates remain in
`docs/implementation/shell-promotion-plan.md`.

There is no fallback to a Chromium/CEF host or to modified client code.
