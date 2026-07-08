# Lightweight Rust + Chromium/CEF Electron-like shell?

**Date:** 2026-07-08
**Question:** Is there a lightweight, Rust-based, CEF/Chromium-based,
Electron-like shell? (Motivation: Tauri uses WebKitGTK on Linux which has no
WebGPU, so we want a Rust+Chromium alternative that gives WebGPU on Linux
without Electron's ~150 MB weight.)

**Verdict: No mature one exists.** "Lightweight" and "Chromium/CEF-based" are
mutually exclusive — Chromium itself is ~150 MB. The Rust CEF bindings are all
immature/stale/experimental. The only genuinely-lightweight Rust web engine
is **Servo**, which has WebGPU but is unproven for a complex Three.js app.
Realistic choices reduce to Electron vs Tauri vs a Servo research bet.

---

## 1. The fundamental tension

Any shell that embeds **Chromium/CEF** bundles the Chromium runtime →
~100–150 MB+ binary. That's not "lightweight" — it's Electron-class weight.
CEF is *architecturally* lighter than Electron (no Node.js runtime; you provide
the native host), but the on-disk size is comparable because Chromium dominates
it. The `wef` authors tried exactly this (CEF offscreen rendering for a game
engine) and **abandoned it**:

> "the app size (included a CEF Framework increase 1GB), development
> experience, etc. So, we are still use Wry [WebKitGTK] in Longbridge desktop
> app for now." — https://github.com/longbridge/wef

So "lightweight + Chromium" is a contradiction. The only lightweight Rust web
engine is **Servo** (Rust-native, no Chromium), which has WebGPU but big web-
compat risk (§3).

---

## 2. Rust CEF / Chromium bindings — all immature

| Project | ★ | Last push | Status |
|---|---|---|---|
| `bamidev/browser-window` | 48 | 2025-11 | Multi-backend (CEF / WebKitGTK / WebView2). "CEF most feature complete." Closest to "Electron-like + Rust + CEF." Small/low-adoption. |
| `not-elm/bevy_cef` | 48 | 2026-07-08 (active!) | Bevy plugin; renders web content to 3D meshes. Bevy-specific, not a general shell. |
| `hytopiagg/cef-ui` | 30 | 2024-09 (stale) | "Work in progress and not complete." Targets Chromium 121 (old). |
| `longbridge/wef` | 36 | 2026-01 | **Abandoned** (see quote above). CEF-OSR, ~1 GB app. |
| `Julusian/rust-cef` | 42 | 2019 (dead) | Stale bindings. |
| `dylanede/cef-rs` | 61 | old | Stale bindings. |

- **`browser-window`** is the only general "Electron-like" Rust+CEF option, and
  it does support CEF on Linux (CEF is cross-platform → Chromium → WebGPU works,
  with the usual `--enable-unsafe-webgpu` flag for some GPUs). But it's a
  48-star project with thin adoption and CEF's size/complexity.
- **`bevy_cef`** is actively maintained and game-focused — interesting *if* the
  engine were Bevy+wgpu, but the renderer here is Three.js, so it doesn't fit.

None of these are a mature, battle-tested, drop-in Electron replacement.

---

## 3. Servo — the only lightweight + Rust-native + WebGPU option (risky)

- **Servo** is a web rendering engine written in Rust, with **WebGL + WebGPU**
  (WebGPU via `wgpu`), and an embeddable **WebView API**.
- Genuinely lightweight and Rust-native (no Chromium).
- **Risk:** Servo's web-platform completeness is far behind Chromium. Three.js
  is a complex, modern web app relying on many APIs; there's **no evidence it
  runs reliably on Servo**, and likely gaps. Servo is great for embedding
  simple web content, unproven for a full Three.js game.
- No shipped Tauri/wry Servo backend that we could confirm.

Sources: https://servo.org/ , https://servo.org/slides/2026-02-fosdem-servo-web-platform/

---

## 4. The realistic decision matrix

| Shell | Rust-native? | Linux webview | WebGPU on Linux? | Lightweight? | Three.js proven? |
|---|---|---|---|---|---|
| **Electron** | No (Rust via napi-rs) | bundled Chromium | ✅ | ❌ (~150 MB) | ✅ (Steam precedents) |
| **Tauri** | ✅ | WebKitGTK | ❌ (no WebGPU) | ✅ (~3–10 MB) | ✅ (via WebGL2 fallback) |
| **browser-window + CEF** | ✅ | CEF (Chromium) | ✅ | ❌ (~100 MB+) | ⚠️ thin adoption |
| **Servo embed** | ✅ | Servo (Rust) | ✅ (wgpu) | ✅ | ❌ unproven |

There is **no row that is Rust-native AND lightweight AND WebGPU-on-Linux AND
proven for Three.js.** Something has to give.

---

## 5. Recommendation

Given the engine is **TS + Three.js (WebGPU) + Rust workers**:

- **If Linux-native WebGPU is non-negotiable** → **Electron**. It's the only
  proven Chromium+WebGPU+Three.js path, and you still get the Rust engine
  code-sharing via `napi-rs` (native) ↔ WASM (web). Not lightweight, not
  Rust-native shell — but mature and Steam-proven.

- **If "lightweight + Rust-native shell" is the priority** → **Tauri**, and
  accept that the **Linux native build renders via WebGL2** (WebKitGTK fallback)
  while Windows/macOS native + the web build get true WebGPU. The Rust workers
  carry the simulation either way; only rendering quality differs on Linux.
  Re-evaluate when Tauri's CEF backend ships (no ETA).

- **Research bet (only if you have appetite for risk)** → **Servo** as the
  embedded webview. Would give the ideal properties (Rust, lightweight,
  WebGPU) IF Three.js runs acceptably on it. Needs a spike to validate
  Three.js WebGPU on Servo before committing.

**CEF via Rust is not recommended** — the bindings are immature, CEF is not
lightweight, and `wef`'s abandonment is a cautionary tale. If you're paying
the Chromium size cost anyway, use Electron (mature) rather than a 48-star
CEF wrapper.

---

## Sources

- `wef` (abandoned CEF-OSR, the ~1GB cautionary quote): https://github.com/longbridge/wef
- `browser-window` (Rust, multi-backend incl. CEF): https://github.com/bamidev/browser-window
- `cef-ui` (stale WIP): https://github.com/hytopiagg/cef-ui
- `bevy_cef` (active, Bevy-specific): https://github.com/not-elm/bevy_cef
- Servo (Rust engine, WebGPU, WebView API): https://servo.org/
- Tauri "no CEF ETA" discussion #8524: https://github.com/orgs/tauri-apps/discussions/8524
- WebGPU impl status (WebKitGTK not shipping): https://github.com/gpuweb/gpuweb/wiki/Implementation-Status
