# Debugging

The most common failures and how to spot them.

> `afterglow-cef` has been removed. The CEF-specific GPU-process,
> `CEF_PATH`, `execute_process`, and `--ozone-platform=x11` caveats below are
> retained as historical context only — `afterglow-shell` is the sole native
> host. See `docs/implementation/shell-promotion-plan.md` for its parity gates.

## Native shell won't start / no WebGPU adapter

1. **Source the devshell.** `shell.nix` provides libvulkan, Mesa, libGL, etc.
2. **Check `VK_ICD_FILENAMES`** points at the real GPU ICD, not SwiftShader.
   On NixOS the devshell sets this via `/run/opengl-driver`.
3. **Check `DISPLAY`** is set (the app runs under X11/XWayland).
4. The shell fails closed on device loss — never accept a software/WebGL
   fallback path.

## SharedArrayBuffer not available

The page must be cross-origin isolated. Check
`self.crossOriginIsolated === true` in the JS console.

- **On web:** use the `coep_server` example or configure your web server with
  COOP/COEP/CORP headers. See [Web (Wasm)](../building/web.md).
- **On the native shell:** the host is single-process; confirm whether
  isolation is required once the G1 asset-root gate lands
  (`docs/implementation/shell-promotion-plan.md`).

## WebGPU shows a software adapter (SwiftShader)

The shell presents through Deno WebGPU → wgpu-core → the system Vulkan loader.
If you see SwiftShader or `WebGPU adapter: ?`:

- re-source the devshell,
- confirm `LD_LIBRARY_PATH` lists the real Vulkan loader,
- on NixOS, confirm `VK_ICD_FILENAMES` points at the real ICDs.

The working shell prints the real adapter, e.g. `WebGPU adapter: amd/rdna-2`.

## Wasm module doesn't share memory

The `.cargo/config.toml` at the **workspace root** must have `--import-memory`
and `--shared-memory`. A `.cargo/config.toml` inside a subcrate is NOT found by
cargo (cargo searches up from cwd, not down). Then:

- JS must create `WebAssembly.Memory({ shared: true, ... })` and pass it as
  `env.memory` when instantiating.
- The JS memory's `maximum` must be ≤ the module's `--max-memory` (64 MiB).

## `latency-tool eval` times out

The page may be running a synchronous JS task. Use `awaitPromise: true` and
ensure the JS yields to the event loop. Native-shell pages don't necessarily
appear in `/json/list` — `latency-tool` uses `Target.getTargets` +
`Target.attachToTarget` instead. Navigate via `latency-tool nav <url>`.

## Where to look in the source

| Symptom | Look at |
|---|---|
| Asset serving | `crates/afterglow-assets/`, `crates/afterglow-web/src/dev_server.rs` |
| Native presenter / WebGPU | `crates/afterglow-shell/src/main.rs`, `native_browser.rs` |
| Ring buffer correctness | `crates/afterglow-rpc/src/lib.rs` |
| Macro generation | `crates/afterglow-rpc-macros/` |
| Web transport | `crates/afterglow-web/src/lib.rs`, `www/worker.js`, `www/rpc.js` |
