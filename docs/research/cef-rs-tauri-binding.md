# `tauri-apps/cef-rs` — the mature Rust CEF binding (correction)

**Date:** 2026-07-08
**Question:** What does `not-elm/bevy_cef` use, and is there actually a mature
Rust CEF binding? (Corrects the earlier "no mature binding" claim in
`lightweight-rust-chromium-shell.md`.)

**Verdict:** Yes — **`tauri-apps/cef-rs`** (crates `cef` + `cef-dll-sys`) is a
mature, Tauri-team-maintained Rust CEF binding. `bevy_cef` uses it. It is the
foundation for a future wry/Tauri CEF backend that would give Tauri native
WebGPU on Linux.

---

## 1. What `bevy_cef` uses

From `bevy_cef`'s `Cargo.toml`:
```toml
cef        = { version = "149.3.0+149.0.6" }              # tauri-apps/cef-rs
cef-dll-sys = { version = "149.3.0+149.0.6", features = ["sandbox"] }
bevy       = { features = ["bevy_winit", "bevy_ui_render"] }
raw-window-handle = "0.6"
```

So `bevy_cef` = a Bevy 0.19 plugin layered on **`tauri-apps/cef-rs`**. bevy_cef
uses **OSR** (offscreen rendering → web content onto Bevy 3D meshes / 2D
sprites), which is why it hits the Wayland-OSR limitation. That's bevy_cef's
*rendering choice*, not a limit of the `cef` crate, which also supports
windowed mode (its `cefsimple` example).

## 2. `tauri-apps/cef-rs` — the real mature binding

- Repo: https://github.com/tauri-apps/cef-rs — **408★**, 61 forks.
- Created 2025-01-10, **pushed 2026-07-07** (very active).
- crates.io: `cef` = **130,695 total / 74,661 recent downloads** — most-adopted
  Rust CEF binding by far. Owned by `tauri-bot` + `wusyong` (Tauri team).
- **Tracks current Chromium**: v149.3.0+149.0.6 (Chromium 149), released
  2026-06-28. Frequent releases tracking Chromium's cadence (148.x → 149.x
  across May–June 2026).
- **Supported targets: Linux + macOS + Windows, x86_64 + ARM64.** Steam Deck
  (x86_64) ✓; ARM64 Linux too.
- Ships `cefsimple` example, `bundle-cef-app` bundling tool, macOS
  helper-bundle config. Downloads CEF binaries via `build.rs` (or shared
  `CEF_PATH`).

> Why I missed it initially: I surveyed GitHub "cef rust" by star count, which
> surfaces stale high-star repos (`dylanede/cef-rs` 61★ from 2017) but buries
> the actively-published `cef` crate. The real adoption signal is crates.io
> download counts + the Tauri org ownership.

## 3. The bigger picture: Tauri is building the CEF backend

`wry` (Tauri's webview layer) README states a feature *"was added in
preparation of other ports like **cef and servo**."* So `cef-rs` is the
**foundation for a future wry/Tauri CEF backend** — the thing Tauri discussion
#8524 said had "no ETA." It is actively being built (Chromium 149, pushed
yesterday). When it lands, **Tauri gets native WebGPU on Linux** via
CEF/Chromium — solving the WebKitGTK-WebGPU problem entirely.

## 4. Impact on afterglow-engine's native shell

This reopens CEF as viable — better than the earlier "all bindings immature"
claim:

- **Now:** `tauri-apps/cef-rs` can power a **Rust-native, WebGPU-on-Linux
  shell** directly. Windowed CEF on Wayland + Vulkan + WebGPU works via flags
  (see `cef-wayland-vulkan-webgpu.md`): `--ozone-platform=wayland
  --enable-features=Vulkan --enable-unsafe-webgpu --use-angle=vulkan
  --ignore-gpu-blocklist`. Linux x86_64 + ARM64. No Node runtime (lighter than
  Electron architecturally).
- **Near future:** when the wry CEF backend lands, you get it through Tauri
  itself — turnkey windowing/packaging + Steamworks via Rust (`steamworks-rs`).
- **Caveats (unchanged):** CEF still bundles Chromium (~100 MB+; lighter than
  Electron since no Node, but not tiny); it's a *binding*, not a turnkey
  Electron replacement (you build the host); OSR-on-Wayland is broken — but the
  Three.js full-window case uses **windowed**, which works.
- `bevy_cef` is **not applicable** to afterglow-engine: it's a Bevy plugin
  using OSR-to-texture, and the engine's renderer is Three.js, not Bevy. Use
  `cef-rs` directly in a windowed host instead.

## 5. Revised native-shell recommendation

| Shell | Rust-native | WebGPU on Linux | Lightweight | Proven for Three.js |
|---|---|---|---|---|
| **`tauri-apps/cef-rs` (windowed CEF)** | ✅ | ✅ (flags) | ⚠️ ~100 MB+ (no Node) | ✅ (Chromium) |
| Electron + napi-rs | ⚠️ (Rust via napi) | ✅ | ❌ ~150 MB | ✅ |
| Tauri (wry/WebKitGTK) | ✅ | ❌ (Linux) | ✅ | ✅ (WebGL2 fallback) |
| Tauri + future wry-CEF backend | ✅ | ✅ | ⚠️ | ✅ (when shipped) |

**New recommendation:** for a Rust-native, WebGPU-on-Linux shell, use
**`tauri-apps/cef-rs` in windowed mode** now, with a planned migration to
native Tauri once the wry CEF backend ships. This best matches the stated goal
(Rust-based, CEF-based, WebGPU on Linux) — better than Electron (Rust-native,
no Node) and better than current Tauri (has Linux WebGPU).

## Sources

- `tauri-apps/cef-rs`: https://github.com/tauri-apps/cef-rs
- crates.io `cef`: https://crates.io/crates/cef (130k dl, Chromium 149)
- crates.io `cef-dll-sys`: https://crates.io/crates/cef-dll-sys
- `bevy_cef` Cargo.toml (uses `cef` 149.3.0+149.0.6): https://github.com/not-elm/bevy_cef
- wry README "preparation of other ports like cef and servo": https://github.com/tauri-apps/wry
- Tauri CEF discussion #8524 (now being addressed via cef-rs): https://github.com/orgs/tauri-apps/discussions/8524
- CEF Wayland+Vulkan+WebGPU flags: `docs/research/cef-wayland-vulkan-webgpu.md`
