# Prerequisites

## Toolchain

- **Rust nightly.** The web target uses `-Zbuild-std` (it needs `panic = "abort"`
  and `--shared-memory` atomics in the standard library), so a nightly toolchain
  is required. Install the wasm target:
  ```sh
  rustup target add wasm32-unknown-unknown
  ```
- **Node.js.** The JavaScript ring-buffer and RPC helpers are tested with Node's
  built-in test runner.
- **Nix** (recommended). The `shell.nix` at the workspace root sets up
  `CEF_PATH`, `DISPLAY`, and the runtime library path (libvulkan, GTK, NSS,
  etc.). On NixOS this is effectively mandatory.

## The devshell

Always build and run through `nix-shell shell.nix`. It provides:

- `CEF_PATH` pinned to `target/debug` (the cached CEF distribution).
- `LD_LIBRARY_PATH` with the real system `libvulkan` loader placed **ahead of**
  `$CEF_PATH`, so the actual GPU driver wins over CEF's bundled SwiftShader (a
  software Vulkan with no surface extensions — GPU init fails without this).
- `VK_ICD_FILENAMES` on NixOS (the Nix Vulkan loader doesn't scan
  `/usr/share/vulkan/icd.d`, so the ICDs must be named explicitly).
- `DISPLAY` defaulting to `:0` (the app runs under X11/XWayland).

```sh
nix-shell shell.nix --run "cargo build --example minimal"
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
```

> If you see `libcef.so: cannot open shared object file`, you ran the binary
> outside the devshell — it needs the devshell's `LD_LIBRARY_PATH`.

## CEF binaries

CEF is not vendored. On the **first** build, `cef-dll-sys` downloads the
matching CEF distribution (~hundreds of MB) into `target/debug/`. After that,
the devshell pins `CEF_PATH` there so subsequent builds reuse the cache.

```sh
# First build ever (CEF_PATH unset) — fetches CEF:
cargo build --example minimal -p afterglow-cef

# Every build after — go through the devshell so CEF_PATH is pinned:
nix-shell shell.nix --run "cargo build --example minimal"
```

## WebGPU

The native shell forces WebGPU onto the real GPU through Dawn → Vulkan. You
need a Vulkan-capable GPU and driver. The example prints the adapter it picked,
for example:

```text
WebGPU adapter: amd/rdna-2
```

If you see SwiftShader or a software adapter, the Vulkan ICD wiring is wrong —
re-source the devshell and check [Debugging](../reference/debugging.md).

## Next

[Verify your install](./verify.md) by building and running the example.
