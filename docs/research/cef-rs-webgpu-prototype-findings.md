# cef-rs WebGPU prototype — empirical findings

**Date:** 2026-07-08
**Prototype:** `prototype/cef-webgpu/` — a Rust app using `tauri-apps/cef-rs`
(crate `cef` v149.3 / Chromium 149) that opens a windowed CEF browser and
renders a WebGPU triangle.

**Result: ✅ WebGPU works through cef-rs on Linux.** Confirmed via JS console
forwarding to stderr:

```
[console] navigator.gpu present ✓
[console] adapter: nvidia / ampere / ?        # real NVIDIA Ampere GPU
[console] device acquired ✓ features: core-features-and-limits
[console] rendering ✓ — WebGPU is working through cef-rs on this platform.
```

Dawn → Vulkan → real NVIDIA GPU. The triangle renders. The stack
(Rust + cef-rs + CEF/Chromium + WebGPU) is viable on Linux.

---

## How to run (NixOS)

```sh
cd prototype/cef-webgpu
# Build WITHOUT CEF_PATH set so cef-dll-sys downloads the matching v149 CEF
# (a stale ~/.local/share/cef at a different version causes an API mismatch
# abort — see gotcha #2).
unset CEF_PATH && cargo build

# Run inside the nix devshell (provides CEF's runtime libs + Vulkan ICD wiring):
env CEF_PATH="$PWD/target/debug" nix-shell shell.nix --run \
  "./target/debug/afterglow-cef-webgpu --ozone-platform=x11"
```

---

## Gotchas hit (and fixed)

### 1. NixOS: CEF's prebuilt libcef.so needs standard FHS libs
`libcef.so` links against glib, gtk3, nss, xorg, cups, libgbm, libudev, … which
NixOS doesn't put in `/usr/lib`. Fix: `shell.nix` pulls them as nix packages
and builds `LD_LIBRARY_PATH` via `lib.makeLibraryPath`. **`makeLibraryPath`
does not follow propagated deps** — so gtk3's transitive deps (cairo, pango,
gdk-pixbuf, atk) must be listed explicitly.

Full list that worked: glib, gtk3, at-spi2-atk, atk, cairo, pango, gdk-pixbuf,
nss, nspr, alsa-lib, dbus, expat, libudev-zero, fontconfig, freetype, harfbuzz,
libxkbcommon, libsm, libice, libx11, libxcomposite, libxcursor, libxdamage,
libxext, libxfixes, libxi, libxrandr, libxrender, libxtst, libxscrnsaver,
libxcb, libdrm, mesa, libGL, libgbm, vulkan-loader, vulkan-validation-layers,
cups, libva, pipewire, libgcrypt, stdenv.cc.cc.lib.

### 2. CEF API version must match exactly
`cef` Rust binding v149 (API 14900) ↔ `libcef.so` v149. If `CEF_PATH` points at
a stale cache from a different CEF version (here: v142 from an old download),
`cef-dll-sys`'s build.rs **reuses the stale binaries** and you get at runtime:
`Request for unsupported CEF API version 14900` → `Trace/breakpoint trap`.
Fix: build with `CEF_PATH` unset (forces a fresh v149 download to OUT_DIR /
`target/debug`), then run with `CEF_PATH=$PWD/target/debug`.

### 3. CEF bundles its own libvulkan + swiftshader → software fallback
CEF ships `libvulkan.so.1` + `vk_swiftshader_icd.json` in its resource dir.
Its bundled loader doesn't know NixOS's ICD path
(`/run/opengl-driver/share/vulkan/icd.d/`), so it falls back to swiftshader
(software), producing a flood of `Unable to initialize SkSurface` /
`Attempt to read from an uninitialized SharedImage` errors and a blank window.
Fix: in `shell.nix`, **prepend `/run/opengl-driver/lib`** to `LD_LIBRARY_PATH`
(so the NixOS system libvulkan loader wins) and set
`VK_ICD_FILENAMES` to the real GPU ICD (nvidia/radeon/intel).

### 4. ⚠️ Wayland + Vulkan are incompatible in CEF 149
**This corrects `docs/research/cef-wayland-vulkan-webgpu.md`.** Empirically,
forcing `--ozone-platform=wayland` with Vulkan enabled logs:

```
'--ozone-platform=wayland' is not compatible with Vulkan.
Consider switching to '--ozone-platform=x11' or disabling Vulkan
```

and WebGPU does not initialize. Since WebGPU on Linux (Dawn) needs the Vulkan
backend, **you must run on X11** (`--ozone-platform=x11`, i.e. XWayland on a
Wayland session) to get WebGPU. Native Wayland + WebGPU is **not** available in
this CEF/Chromium build. (Revisit when Chromium's Ozone-Wayland gains Vulkan
support.)

### 5. JS console forwarding
To see WebGPU init status in the terminal, the prototype adds a
`DisplayHandler::on_console_message` that prints JS `console.log` to stderr,
and the demo HTML `console.log`s its milestones. (CEF does not forward
`console.log` to the host stderr by default.)

---

## What this proves for afterglow-engine

- **cef-rs + WebGPU on Linux works today** (Rust-native host, no Node runtime,
  real GPU via Dawn→Vulkan). Stronger position than Electron for the stated
  "Rust-based + CEF + WebGPU on Linux" goal.
- The native shell would be `tauri-apps/cef-rs` in windowed mode, with the
  flags from `simple_app.rs::on_before_command_line_processing`, on **X11**
  (XWayland) — not native Wayland, until Chromium supports Wayland+Vulkan.
- NixOS needs the `shell.nix` runtime-lib wiring + system-libvulkan preference
  + real Vulkan ICD. This belongs in the engine's dev flake.
- The WebGPU page itself (here a hand-written triangle) is trivially swappable
  for a Three.js WebGPU bundle — the shell is renderer-agnostic.

## Files

- `prototype/cef-webgpu/Cargo.toml`, `build.rs`, `shell.nix`, `run.sh`
- `prototype/cef-webgpu/resources/index.html` (WebGPU triangle + status HUD)
- `prototype/cef-webgpu/src/main.rs`, `src/shared/{mod,simple_app,simple_handler}.rs`
