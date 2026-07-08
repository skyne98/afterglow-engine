# AGENTS.md — afterglow-engine

## Project direction

**afterglow-engine** is a **web-based game engine**. The rendering foundation
under evaluation is **Three.js with the WebGPU renderer** (with WebGL2 fallback).

See `docs/research/` for the stability/feasibility research that informs this
choice. Research notes are the canonical record of decisions — keep them
up-to-date as the stack evolves.

## Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
- Legacy code is bad; delete legacy code, embrace new code and systems
- From time to time, spawn a subagent to look at the code and suggest cleanups — you might have left a mess
- Always clean up temporary files
- KISS and YAGNI

## Research

- `docs/research/threejs-webgpu-stability.md` — How stable and usable is the
  Three.js WebGPU renderer? (investigated 2026-07)
- `docs/research/native-runtime-linux-steam.md` — Native runtime options to ship
  the web engine on Linux + Steam. Verdict: Electron (bundles Chromium →
  WebGPU) for desktop/Steam; `react-native-webgpu` (Dawn) for iOS/Android/macOS
  via React Three Fiber; Tauri/Neutralino NOT viable on Linux (WebKitGTK lacks
  WebGPU). Includes a multi-target strategy.
- `docs/research/lightweight-rust-chromium-shell.md` — Is there a lightweight,
  Rust-based, CEF/Chromium Electron-like? No mature one: CEF isn't lightweight
  (~100MB+, `wef` abandoned it for that), Rust CEF bindings are stale, Servo is
  the only light+Rust+WebGPU option but unproven for Three.js.
- `docs/research/servo-threejs-status.md` — Deep-dive: Servo canNOT run Three.js
  today. WebGL path has years-old unfixed bugs; WebGPU path is incomplete (missing
  methods, UB, crashes, broken CTS); a real WebGPU game (SpookyBall) doesn't run;
  0 reports of Three.js *WebGPU* on Servo. Revisit in ~2 yrs.
- `docs/research/cef-wayland-vulkan-webgpu.md` — CEF graphics on Linux: YES for
  windowed (Electron-style, Three.js full-window) on native Wayland+Vulkan+WebGPU
  via flags; NO for OSR/webview-as-texture overlay (forced to X11). Blockers are
  non-graphics (size ~100MB+, immature Rust bindings, build complexity) — offers
  no advantage over Electron for this stack.
- `docs/research/cef-rs-tauri-binding.md` — **CORRECTION:** there IS a mature
  Rust CEF binding — `tauri-apps/cef-rs` (crates `cef`+`cef-dll-sys`),
  Tauri-team-maintained, 408★, 130k dl, Chromium 149, Linux x86_64+ARM64. This is
  what `bevy_cef` uses. Foundation for a future wry/Tauri CEF backend → native
  WebGPU on Linux. Revised native-shell recommendation: use cef-rs windowed now.
- `docs/research/cef-rs-webgpu-prototype-findings.md` — **Built & ran a cef-rs
  WebGPU prototype.** ✅ WebGPU works through cef-rs on Linux (NVIDIA/Ampere via
  Dawn→Vulkan). Empirical gotchas: NixOS runtime-lib wiring (shell.nix),
  CEF-API-version must match (don't reuse stale CEF_PATH), must prefer system
  libvulkan + real ICD over CEF's bundled swiftshader. ⚠️ CORRECTION: Wayland+
  Vulkan are INCOMPATIBLE in CEF 149 — must use --ozone-platform=x11 (XWayland)
  for WebGPU; native Wayland+WebGPU not available yet. See `prototype/cef-webgpu/`.
- `docs/research/cef-games-latency-footprint-debugging.md` — CEF for games:
  real-world usage (Steam, GW2/ArenaNet ~3× faster than CoherentUI, Battle.net/
  Epic, Coherent Gameface), input→pixel latency pipeline (our windowed
  architecture sidesteps the OSR-texture-copy latency), latency flags
  (--disable-gpu-vsync/--disable-frame-rate-limit/etc., + vsync-reset caveat),
  footprint (Minimal dist + strip + en-US locale ~80-110MB floor, can't
  feature-strip), debugging (remote-debugging-port + chrome://tracing +
  crashpad), cef-rs accelerated_osr zero-copy path.
