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
(~hundreds of MB, first build only). Three.js is fetched by a one-off script
(gitignored — it's a build input for `include_bytes!`).

```sh
cd prototype/cef-webgpu

# 0. Fetch the Three.js WebGPU build (pinned version, one-off).
bash resources/download-three.sh

# 1. Build the binary. Build WITHOUT CEF_PATH set so cef-dll-sys downloads the
#    matching CEF version (a stale ~/.local/share/cef at a different version
#    causes an API-mismatch abort — see findings doc).
unset CEF_PATH && cargo build

# 2. Run inside the nix devshell (provides CEF's runtime libs + Vulkan ICD
#    wiring). Must use X11 (XWayland) — Wayland+Vulkan is incompatible in
#    CEF 149 (see findings doc).
env CEF_PATH="$PWD/target/debug" nix-shell shell.nix --run \
  "./target/debug/afterglow-cef-webgpu --ozone-platform=x11"
```

You should see a window with a status HUD (`THREE.REVISION = 185`,
`backend = WebGPUBackend`, `adapter: nvidia/ampere`) and a rotating green box
rendered via Three.js WebGPU on the real GPU.

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
