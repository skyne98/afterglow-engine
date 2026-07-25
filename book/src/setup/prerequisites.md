# Prerequisites

## Toolchain

- **Rust nightly.** The web target uses `-Zbuild-std` (it needs `panic = "abort"`
  and `--shared-memory` atomics in the standard library), so a nightly toolchain
  is required. Install the wasm target:
  ```sh
  rustup target add wasm32-unknown-unknown
  ```
- **Node.js / Bun.** The JavaScript ring-buffer and RPC helpers are tested with
  Node's built-in test runner; the web build and conformance gates use Bun.
- **Nix** (recommended). The `shell.nix` at the workspace root sets up `DISPLAY`
  and the runtime graphics library path (libvulkan, Mesa, libGL, GTK, etc.).
  On NixOS this is effectively mandatory.

## The devshell

Always build and run through `nix-shell shell.nix`. It provides:

- `LD_LIBRARY_PATH` with the real system `libvulkan` loader so the GPU driver
  wins over any software fallback.
- `VK_ICD_FILENAMES` on NixOS (the Nix Vulkan loader doesn't scan
  `/usr/share/vulkan/icd.d`, so the ICDs must be named explicitly).
- `DISPLAY` defaulting to `:0` (the app runs under X11/XWayland).

```sh
nix-shell shell.nix --run "cargo build -p afterglow-shell"
nix-shell shell.nix --run "cargo run -p afterglow-shell"
```

> The former CEF-specific `CEF_PATH`, `cef-dll-sys` download, and
> `--ozone-platform=x11` caveats no longer apply — `afterglow-cef` has been
> removed. See `docs/implementation/shell-promotion-plan.md` for the native
> host's remaining parity gates.

## WebGPU

The native shell presents through Deno WebGPU → wgpu-core → Vulkan. You need a
Vulkan-capable GPU and driver. The shell reports the adapter it picked, for
example:

```text
WebGPU adapter: amd/rdna-2
```

If you see SwiftShader or a software adapter, the Vulkan ICD wiring is wrong —
re-source the devshell and check [Debugging](../reference/debugging.md).

## Next

[Verify your install](./verify.md) by building and running the shell.
