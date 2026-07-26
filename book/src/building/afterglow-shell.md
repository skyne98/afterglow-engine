# Native (`afterglow-shell`)

`afterglow-shell` is Afterglow's lightweight native Three.js/WebGPU runtime. It
runs the application's module code directly in rusty_v8, presents JavaScript
WebGPU work through winit/wgpu, and renders HTML/CSS overlays with Blitz and
Vello. It is the **sole** native host; `afterglow-cef` has been removed.

The remaining parity gates (asset-root loading, native RPC-worker bootstrap,
production game bootstrap API, release evidence) are tracked in
`docs/implementation/shell-promotion-plan.md`.

## Build

From the workspace root:

```sh
cargo build -p afterglow-shell
```

If rusty_v8's automatic archive download is unavailable:

```sh
curl -L -o /tmp/v149_simdutf.a.gz \
  https://github.com/denoland/rusty_v8/releases/download/v149.4.0/librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz
gzip -dc /tmp/v149_simdutf.a.gz > /tmp/v149.a
RUSTY_V8_ARCHIVE=/tmp/v149.a cargo build -p afterglow-shell
```

## Run

Launch the bundled native game module:

```sh
cargo run -p afterglow-shell
```

Or pass an official Three.js HTML example:

```sh
cargo run -p afterglow-shell -- \
  /tmp/threejs/examples/webgpu_clearcoat.html
```

Run the generated Dungeon deployment in release mode:

```sh
cargo run --release -p afterglow-shell -- \
  crates/afterglow-web/www/dungeon.html
```

The shell reads the document's import map and module script and executes them
unchanged. OrbitControls, Inspector, asset loading, animation loops, HTML/CSS,
and input all run against the native environment. After renderer readiness the
host synchronizes the current physical window size once, so startup configure
events cannot leave the game at its fallback dimensions. Pointer-locked relative
motion is routed to the lock element rather than the hidden cursor's stale hit
target.

Startup does not block the native event loop while a module performs top-level
await. winit presentation turns drive a fixed-capacity `requestAnimationFrame`
queue, while deno_core is polled one bounded turn at a time. Top-level rAF
awaits therefore progress without timers, source patches, or ignored runtime
errors. The queue admits up to 1,024 callbacks per window and fails excess
requests deterministically.

The shell also exposes the explicitly admitted `scheduler.yield()` subset.
Three.js uses it to split large `compileAsync()` workloads into deno/winit task
turns instead of one presentation frame per work item. It is backed by
`deno_web::op_defer`, not a timer or microtask. `scheduler.postTask` and
`TaskController` are intentionally unsupported. The real 5,000-entity engine
demo reaches renderer readiness in about 145 ms on the RTX 3090; the pure-rAF
fallback exceeded 90 seconds. Startup defaults to a 30-second active-time
deadline, overrideable for diagnostics with `AFTERGLOW_STARTUP_TIMEOUT_MS`.

## What is native

- winit owns the OS window and input.
- Deno WebGPU and the presenter share one wgpu-core device.
- Three.js renders directly into the native surface.
- Vello rasterizes the transparent DOM HUD on the GPU.
- Computed CSS controls the native OS cursor.
- No Chromium process, duplicate presentation device, full-frame CPU readback,
  or client-source rewriting is involved.

The deterministic headless runner intentionally uses a CPU-readable paint path
to produce PNG test output. The rAF queue has a separate fast unit gate:

```sh
bun test crates/afterglow-shell/tests/raf.test.ts \
  crates/afterglow-shell/tests/scheduler.test.ts
cargo build -p afterglow-shell --example browser_test
./target/debug/examples/browser_test \
  /tmp/threejs webgpu_multiple_elements /tmp/out.png
```

## Current transition boundary

`afterglow-shell` already runs unmodified Three.js examples and reactive DOM
applications, but the engine-facing asset package, native RPC-worker bootstrap,
and final shipping configuration API still need integration. Those open gates
are tracked in `docs/implementation/shell-promotion-plan.md`; there is no longer
a CEF fallback host.
