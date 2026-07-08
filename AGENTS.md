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
