# Debugging

The most common failures and how to spot them.

## CEF app won't start / GPU process crash

1. **Pass `--ozone-platform=x11`.** Wayland + Vulkan are incompatible in CEF 149.
2. **Check `CEF_PATH`** points at the right CEF distro (the devshell pins it to
   `target/debug`). A stale `CEF_PATH` mismatches the CEF API version.
3. **Don't spawn threads before `execute_process`.** It crashes the GPU
   process. Use `AppBuilder::on_ready` (or spawn from within the CEF message
   loop).
4. **Source the devshell.** `shell.nix` provides libvulkan, GTK, NSS, etc.
5. The `libudev` version warning is harmless (libudev-zero).

## `libcef.so: cannot open shared object file`

You tried to run the binary **outside the devshell**. It needs the devshell's
`LD_LIBRARY_PATH`. Always run through
`nix-shell shell.nix --run "./target/debug/examples/minimal …"`.

## SharedArrayBuffer not available

The page must be cross-origin isolated. Check
`self.crossOriginIsolated === true` in the JS console.

- **On CEF:** the `afterglow://` scheme handler sets the COOP/COEP/CORP headers
  on every response.
- **On web:** use the `coep_server` example or configure your web server with
  the same headers. See [Web (Wasm)](../building/web.md).

## WebGPU shows a software adapter (SwiftShader)

CEF bundles its own `libvulkan.so.1` + SwiftShader (a software Vulkan with **no
surface extensions** — GPU/WebGPU init fails on it). The devshell forces the real
system `libvulkan` ahead of `$CEF_PATH` in `LD_LIBRARY_PATH`. On NixOS it also
sets `VK_ICD_FILENAMES`. If you see SwiftShader or `WebGPU adapter: ?`:

- re-source the devshell,
- confirm `LD_LIBRARY_PATH` lists the real Vulkan loader before `$CEF_PATH`,
- on NixOS, confirm `VK_ICD_FILENAMES` points at the real ICDs.

The working example prints the real adapter, e.g. `WebGPU adapter: amd/rdna-2`.

## Wasm module doesn't share memory

The `.cargo/config.toml` at the **workspace root** must have `--import-memory`
and `--shared-memory`. A `.cargo/config.toml` inside a subcrate is NOT found by
cargo (cargo searches up from cwd, not down). Then:

- JS must create `WebAssembly.Memory({ shared: true, ... })` and pass it as
  `env.memory` when instantiating.
- The JS memory's `maximum` must be ≤ the module's `--max-memory` (64 MiB).

## `v8_value_create_array_buffer` returns None

This is the **V8 sandbox** — compiled into CEF 149, not toggleable at runtime.
`CefV8Value::CreateArrayBuffer` (external backing store) always returns
nullptr. Use `CreateArrayBufferWithCopy` instead (one memcpy). This only affects
the CEF native path; the web `SharedArrayBuffer` path has no such issue.

## `latency-tool eval` times out

The page may be running a synchronous JS task. Use `awaitPromise: true` and
ensure the JS yields to the event loop. Also note CEF Views browsers don't
appear in `/json/list` — `latency-tool` uses `Target.getTargets` +
`Target.attachToTarget` instead. Navigate via `latency-tool nav <url>`.

## Where to look in the source

| Symptom | Look at |
|---|---|
| Scheme/asset serving | `crates/afterglow-cef/src/resources.rs`, `crates/afterglow-assets/` |
| GPU/X11 flags | `crates/afterglow-cef/src/flags.rs` |
| CEF startup / `on_ready` | `crates/afterglow-cef/src/runtime.rs`, `config.rs` |
| Ring buffer correctness | `crates/afterglow-rpc/src/lib.rs` |
| Macro generation | `crates/afterglow-rpc-macros/` |
| Web transport | `crates/afterglow-web/src/lib.rs`, `www/worker.js`, `www/rpc.js` |
