# afterglow-cef-webgpu — cef-rs WebGPU prototype

Minimal Rust app using [`tauri-apps/cef-rs`](https://github.com/tauri-apps/cef-rs)
(the `cef` crate) that opens a windowed CEF browser and renders a **WebGPU**
triangle. Proves the stack: **Rust + CEF + Wayland/Vulkan + WebGPU** on Linux,
with Three.js-ready Chromium underneath.

## What it does

- Forces WebGPU + Vulkan on for all CEF processes via `on_before_command_line_processing`
  (`--enable-unsafe-webgpu`, `--ignore-gpu-blocklist`, `--enable-features=Vulkan`,
  `--use-angle=vulkan`).
- Loads a bundled WebGPU triangle demo (`resources/index.html`) via `file://`.
- Uses the CEF **Views framework** (windowed path — the one that works on native
  Wayland; OSR is forced to X11, see `docs/research/cef-wayland-vulkan-webgpu.md`).

## Build & run

CEF binaries are downloaded automatically by `cef-dll-sys`'s build script
(~hundreds of MB, first build only).

```sh
cd prototype/cef-webgpu

# 1. Build the binary.
cargo build --release

# 2. Bundle it with the CEF runtime/resources (locales, icudtl, etc.).
cargo run --release --bin bundle-cef-app -- afterglow-cef-webgpu -o target/bundle --release

# 3. Run it. Pass --ozone-platform=wayland to force native Wayland
#    (Chromium auto-detects on recent versions, so usually optional).
./target/bundle/afterglow-cef-webgpu --ozone-platform=wayland
```

You should see a window with a status HUD ("adapter: …", "WebGPU OK") and an
animated WebGPU triangle below it.

## NixOS note

CEF ships prebuilt Linux binaries linked against a standard glibc FHS layout
that NixOS doesn't have. If the binary fails to launch (loader errors), wrap the
run in an FHS env, e.g.:

```sh
nix-shell -p patchelf --run \
  "patchelf --set-interpreter $(cat $NIX_CC/nix-support/dynamic-linker) \
     target/bundle/afterglow-cef-webgpu"
# or run inside: nix-shell -p buildFHSUserEnv ...
```

(`libcef.so` and the helper processes may each need `patchelf` for the
interpreter + RUNPATH.)
