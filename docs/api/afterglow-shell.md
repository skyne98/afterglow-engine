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
cargo run -p afterglow-shell -- --compat-three \
  /path/to/three.js/examples/webgpu_clearcoat.html
```

Run the generated Dungeon deployment through the native host in release mode:

```sh
cargo run --release -p afterglow-shell -- crates/afterglow-web/www/dungeon.html
```

Diagnostic artifacts can request one final composited-surface capture. The host
preallocates a fixed staging buffer before gameplay, waits 30 complete frames
after strict `GameReady`, maps only on the diagnostic slow path, writes PNG, and exits after
ordered V8/GPU/window teardown:

```sh
AFTERGLOW_CAPTURE_PATH=/tmp/dungeon.png \
AFTERGLOW_WINDOW_WIDTH=1280 AFTERGLOW_WINDOW_HEIGHT=720 \
  cargo run -p afterglow-shell -- \
  crates/afterglow-web/www/diagnostic-dungeon.html
```

`AFTERGLOW_CAPTURE_MAX_WIDTH`/`HEIGHT` bound the staging allocation (defaults
4096×2160); `AFTERGLOW_CAPTURE_READY_FRAMES` overrides the 30-frame settling
period. Production launches omit `AFTERGLOW_CAPTURE_PATH` and allocate no
readback buffer.

HTML loading extracts the document's `type="importmap"` and
`type="module"` scripts, resolves `three` and `three/addons/` from that map,
and evaluates every module script unchanged, not only the first script.
External scripts enter one module graph in document order.
Each inline script retains a separate module scope and the document base URL.
Readiness waits for the full graph, including top-level `await`.
`--compat-three` is the explicit isolated
compatibility profile: its first successful presentation may satisfy host
readiness. Authored engine pages omit the flag and become ready only when
`EngineRuntime` emits `op_afterglow_game_ready` after a complete post-seal game
update and presentation with zero fatal diagnostics. Files, data URLs, blobs, and HTTPS assets
are loaded by the runtime environment.

The presenter creates the native surface through the same wgpu-core `Global`
owned by `deno_webgpu`. JavaScript command buffers render directly to the
surface texture; no second graphics device or full-frame CPU readback is used.
The transparent DOM HUD is emitted as a Vello scene, rasterized on the shared GPU
device with MSAA16, and composited after the game pass.

## Input timestamps

Native `UIEvent` timestamps use the same monotonic millisecond clock as `performance.now()`.
This includes pointer events and their coalesced samples.
Unix timestamps are not valid stroke timestamps.

## Random values

`crypto.getRandomValues()` fills integer typed arrays with OS-backed cryptographic bytes.
Each call accepts at most 65,536 bytes and returns the same array.
`crypto.randomUUID()` returns an RFC 4122 version-4 UUID.
An OS random failure stops the operation without a fallback.
`crypto.subtle` is not available.

## DOM-only documents

`--document <html-path>` gives a DOM-only application a shell-owned WebGPU surface.
The document must not create another WebGPU surface.
The shell clears its surface and composites the Blitz/Vello scene each frame.
Readiness needs completed module evaluation and a successful presentation.
A changed HUD or an incomplete screenshot keeps frame scheduling active even without pending RPCs or animation callbacks.
The host returns to idle after publication and screenshot completion.
Game startup and `--compat-three` do not use this bootstrap.
A missing adapter or device loss stops the application without a fallback.

Build and run the native paint editor:

```sh
cd prototype/character-editor
bun run build
cd ../..
nix-shell shell.nix --run 'CARGO_BUILD_JOBS=$(nproc) cargo run --release -p afterglow-shell -- --document prototype/character-editor/dist/paint/paint-demo.html'
```

`shell.nix` adds the GTK and desktop GSettings schema directories to `XDG_DATA_DIRS`, before existing data directories.
Native file dialogs need these compiled schemas. Library paths alone are not sufficient and can cause a GLib abort.
Check schema access with `nix-shell shell.nix --run 'bash scripts/test-shell-settings.sh'`.
This check uses memory-only settings and does not change desktop settings.

The native paint probe uses `dist/paint/paint-native-probe.html` instead.
It checks strokes, native pointer dispatch, undo/redo, brush selection, unchanged canvas pixels after panel clicks, sliders, brush labels, scrolling, and PNG export.
The probe prints `[native-paint-probe] PASS` only after all checks complete.
It also records 120 rAF intervals for unchanged frames, hover changes, and slider movement under `[native-ui-cadence]`.
These short measurements are not physical presentation timing or long-soak evidence.
Use the release build for UI performance checks, not the unoptimized debug build.
Three corrected release launches measured 143.99–144.25 FPS for slider movement and 130.04–138.31 FPS for hover changes.
Hover changes still miss frame deadlines.
See `docs/benchmarks/native-paint-shell/` for the baseline, corrected results, and evidence limits.
`AFTERGLOW_CAPTURE_PATH` can capture this page after its module evaluation completes.
This full probe keeps module evaluation open and does not accept OS input during its checks.
Its latest run stopped at the composition-undo check. That failure remains open.

The profiling capture and control pages instead complete module evaluation before measurement.
This permits native input during capture. The capture driver rejects a run before native input readiness.
Run `nix-shell shell.nix --run 'bun scripts/test-native-paint-menus.ts'` after the release and Vite builds.
This check uses XWayland/XTest mouse clicks on File, Edit, and View while the capture workload runs without a collector.
It checks menu state and Escape after native readiness, not physical input-to-pixel latency.

Software Canvas2D uses `context.commit()` to publish its full RGBA raster into Blitz.
`context.commit(x, y, width, height)` publishes a changed region in canvas pixels.
The rectangle must contain positive integer dimensions and stay inside the canvas.
The bridge borrows the existing JavaScript buffer and copies only changed rows into a retained CPU raster.
The initial publication and a resize copy all source pixels before replacement.
Each connected canvas has one pending rectangle, which combines changes before the next HUD frame.
The HUD uses a persistent Vello texture and uploads only that rectangle.
Vello still copies the full changed texture into its GPU atlas.
Resize and removal unregister old texture overrides. CPU capture makes an immutable snapshot of current pixels.
No artist configuration or fixed canvas-count limit was added.
Hardware dimension limits still apply. Aggregate memory admission and sustained GPU memory checks remain incomplete.
The region path passed browser tests and an exact hardware GPU check for pixels, capture, resize, and retirement.
The release raster results are in `docs/benchmarks/native-paint-shell/raster-regions/`.
Detached canvases remain local for PNG export.
The native paint service has the name `paint` and runs on one OS thread.
See `maipointo.md` for its capacities and memory-only history.

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
  window size is synchronized once after game readiness, even when the
  initial configure event arrived during startup;
- image decode, `createImageBitmap`, Blob, Storage, KTX2/Draco workers, and a
  software Canvas2D environment;
- GPU-rendered text, synthetic bold faces, text shadows, SVG, and page chrome;
- resize debounce, suspend/resume, adapter reporting, and fail-closed device-loss
  handling;
- browser-cadenced `requestAnimationFrame`: a fixed 1,024-callback queue with
  O(1) cancellation, deterministic overflow, shared frame timestamps, and
  deno_core external-operation tracking during top-level awaits. Each admitted
  presentation deadline is anchored to the current host instant, so startup
  wake bursts cannot accumulate future frame delays;
- the explicitly admitted `scheduler.yield()` subset, implemented as a deferred
  deno/winit task continuation. `scheduler.postTask` and `TaskController` are
  intentionally absent.

Horizontal range inputs have a native track and thumb, pointer capture, step limits, and keyboard controls.
Range changes send `input` events and send `change` after the drag.
Vertical range inputs remain unsupported.
Number inputs use the same intrinsic line height as text inputs, including inside initially closed details panels.
A raster regression checks the displayed digits, not only the stored input value.
Text controls use the existing Blitz/Parley editor for caret placement, drag selection, replacement, deletion, and keyboard selection.
The native bridge converts DOM UTF-16 selection offsets to native UTF-8 offsets.
JavaScript dispatch precedes the default edit action. Canceled `keydown` or `beforeinput` prevents the edit.
User edits send `input`. A changed value sends `change` on blur or single-line Enter.
Live values remain separate from the `value` attribute and `defaultValue`.
Readonly controls permit selection but not edits. Disabled controls reject native edits.
The local LinkeDOM patch supplies capture/target/bubble phases, listener capture identity, passive listeners, and abort removal.
Canceled `pointerdown` suppresses compatibility mouse events, not `click`. Capture release precedes click, and secondary buttons send `auxclick`.
The source comparison and remaining input-quality gates are in `docs/research/native-shell-input-chromium-comparison.md`.
The current input change adds text clipboard operations, bounded text history, and Winit IME composition through the existing Parley editor.
Native regression and device checks for this change are pending.

- `navigator.clipboard.readText()` and `writeText(text)` permit application script access without a user action, by explicit user decision.
- Text copy/cut/paste dispatch clipboard events. Transfers have a 32 MiB UTF-8 limit and one in-flight operation. Native clipboard I/O uses a blocking thread, not the UI thread.
- Failed clipboard writes do not cut text. A delayed paste does not replace text after focus, selection, or value changes.
- Ctrl/Cmd+Z, Ctrl+Y, and Ctrl/Cmd+Shift+Z operate on text history in the focused control. History has 512 edits per control and a 32 MiB document accounting limit. Capacity removes old history, not current text.
- Programmatic value changes clear obsolete text history. Undo/redo retain text and selection, and respect canceled `beforeinput` events.
- IME preedit uses native Parley composition. Commit forms one history edit. Focus loss cancels transient text and restores the original selection.
- The native IME candidate area uses the transformed text-editor rectangle. Captured text selection applies inverse CSS transforms.
- A fixed 64-sample mouse buffer retains host-arrival timestamps and sample order. A full buffer dispatches immediately. `getCoalescedEvents()` returns the retained samples.

These are UI slow paths, not `GameplaySealed` allocation evidence. Clipboard libraries can allocate external input before the shell checks its length.
Pen backends, physical input latency, cross-platform device checks, and long soaks remain open. Host-arrival timestamps are not hardware timestamps.
These checks do not establish Chromium parity.
Wheel input selects a scrollable ancestor on the requested axis and continues at its edge.
Content scrolling does not move the container client rectangle or change layout-relative offsets.
The native adapter reverses Winit wheel deltas to match DOM wheel direction and converts physical pixels to CSS pixels.
Named keyboard keys retain their DOM names, including arrows, Enter, Tab, and Escape.
Single-select controls display their current label and use one DOM popup for pointer and keyboard selection.
The native snapshot sends current option selectedness without a change to source DOM attributes.
This includes the default first option. The label stylesheet does not use `:has()`, which the current Stylo parser rejects.
Disabled options and groups cannot change the value. Escape cancels popup selection, and Tab closes the popup.
CSS transitions use a monotonic clock and keep HUD presentation active until the animation stops.
The updated Taffy layout owns grid overflow rectangles. The old cell-local content-size correction is removed.
Text hit tests stay inside their line bounds and obey ancestor overflow clips.
Unchanged stylesheet snapshots do not replace parsed stylesheets.
Empty mutation callbacks after a synchronous queue drain do not trigger another DOM synchronization.
Native mouse and pointer events calculate target-relative offsets only when a listener reads them.
Unused offsets do not force layout after pointer listeners change the DOM.
DOM interface type tags prevent reactive libraries from replacing native node identity with proxies.
Style-property mutations reach the native snapshot. Client rectangles include CSS transforms without changes to layout offsets.
Stacking positions and bounds use the completed layout, including translated popups on their initial frame.
The paint probe checks File/Edit/View opening, menu placement, item activation, Escape, keyboard opening, and outside clicks.

Blitz uses incremental layout, so unchanged text and layout retain their cached results.
Cached overflow bounds include the node position and transform before parent calculations.
Regressions compare incremental and full-layout pixels after text, range, style, visibility, viewport, and child-removal changes.

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

Winit owns the outer event loop. Bounded runtime turns advance module
evaluation, timers, and native responses independently of redraw delivery.
Each turn calls `JsRuntime::poll_event_loop()` rather than running Deno to idle.
A coalesced runtime wake marks pending work. Runtime maintenance uses the
monitor interval and does not poll immediately for every wake.
Only one shell redraw request remains outstanding at a time.
The common presentation path calls `Window::pre_present_notify()` after GPU
submission and immediately before presentation. On Wayland, Winit uses the
compositor frame callback to control redraw delivery.
Pending rAF callbacks can wait while the compositor withholds frames.
Runtime-only turns do not drain those callbacks or present a surface.
Timers, native workers, diagnostics, and input continue without a redraw.
On restore, rAF receives the current timestamp, without missed-frame replay.
Focus loss alone does not stop rendering. Other platforms retain the existing
monitor-interval limit where `pre_present_notify` has no effect.
The [pacing repair](../implementation/native-presentation-pacing-plan.md) still
needs real-driver acceptance. This mechanism cannot guarantee that a driver
call will not block. Pending rAF work
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
`vendor/afterglow-shell-blitz/THREE_NATIVE_PIN`.
The current update uses upstream `main` commit `a50cb8971a03fb4cac697b763f8f5d01ee83cefb` (223 commits after the previous pin).
It uses Blitz `0.3.0-beta.2`, Stylo `0.20`, anyrender `0.13`, and Vello `0.10`.
Native node maps keep the full versioned `NodeId`. Raster publication and initial-containing-block corrections remain local patches.
Upstream now includes the cached-overflow transform correction and no longer uses an incremental-layout feature flag.
The all-target native API check, release build, and 53 native regression tests passed.
The RTX 3090 probe passed select, input-value, wheel-zoom, hover-transition, and paint checks and produced a checked screenshot.
See `docs/benchmarks/native-paint-shell/` for the log, image, and measurement limits.

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

## Profiling port

On Unix, the command-line application composes one native `diagnostics` RPC worker.
It connects to the CLI WebSocket server at `ws://127.0.0.1:8086/`, without tokens or permission tiers.
`AFTERGLOW_DIAGNOSTICS_PORT` selects another server port, and `off` disables profiling.
Port 0 is invalid for the application. Startup logs the server URL.
An unavailable server leaves recording inactive, with at most one attempt per second.
The CLI owns capture files. No collector subprocess or JSON bootstrap is necessary.
This protocol is not Tracy-compatible and does not expose arbitrary code execution.
A shell-owned timer checks the connection and transfers bounded host and application
records through the generated native client. Host presentation, input type, and
asynchronous RPC spans use one fixed 1,024-record buffer. Diagnostics RPC calls are
excluded. The paint prototype registers a separate stroke/publication source.
The native WebSocket capture passed with no lost records and separate host/paint sources.
Controlled timing comparisons and sustained memory checks remain open. See [telemetry](telemetry.md) for capacities,
clock mapping, connection behavior, and allocation limits.

The build needs Bun to compile `diagnostics.ts` into the embedded capture entry.
The generated diagnostics client uses the standard web worker staging directory.
`op_diagnostics_capture(epoch, a, b, c, d)` and `op_diagnostics_drain()` supply only
host-local snapshots. Worker payloads still use generated RPC and RingBuffer.

## Native host status

`afterglow-cef` has been removed. `afterglow-shell` is the sole native host and
native deployment target. Asset-root loading and native texture/mesh worker
composition are implemented. The remaining release work includes packaged-
resource policy, long hardware soaks, input/resize/device-loss evidence, direct
native VT atlas upload evaluation, and Steam integration; these gates remain in
`docs/implementation/shell-promotion-plan.md`.

There is no fallback to a Chromium/CEF host or to modified client code.
