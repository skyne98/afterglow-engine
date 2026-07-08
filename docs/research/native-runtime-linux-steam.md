# Native runtime for a Three.js WebGPU engine on Linux + Steam

**Date:** 2026-07-08
**Question:** Is there a "React Native"-style native runtime that lets a
web-based Three.js engine run natively on Linux and ship on Steam?

**Verdict:** There is **no** React-Native-style native UI runtime for Three.js
(RN is for native UI components, not canvas/WebGPU). The practical answer is a
**desktop webview wrapper**, and for Linux + WebGPU + Steam that means
**Electron** (or NW.js / CEF). **Tauri is not viable on Linux for WebGPU**
because it uses the system WebKitGTK, which has not shipped WebGPU.

---

## 1. Why "React Native" is the wrong frame

React Native transpiles RN components to **native platform UI widgets**
(UIView, android.view). It is not a renderer for `<canvas>`/WebGPU apps. There
is no equivalent that takes a Three.js/WebGPU app and gives you a "native"
non-web GPU surface while keeping the Three.js API. What you actually want is a
**desktop shell that hosts a Chromium (or other) webview** so your existing
WebGPU code runs unchanged. The options differ by *which* webview engine they
embed — and that's where Linux WebGPU support diverges.

---

## 2. The deciding factor: which webview, and does it do WebGPU on Linux?

WebGPU implementation status on Linux (from the gpuweb Implementation-Status
wiki, 2026-07):

| Engine | Linux WebGPU | Notes |
|---|---|---|
| **Chromium / Dawn** | ✅ Intel Gen12+ (Chrome 144+); ✅ NVIDIA driver 535+ (Chrome 147); 👷 others behind `--enable-unsafe-webgpu` flag | The only mature Linux WebGPU path. |
| Firefox | 👷 Behind flag (Nightly/Beta + `gfx.webgpu.ignore-blocklist`) | Mozilla expects to ship on Linux in 2026. Not ready to depend on. |
| **WebKitGTK** (Linux system webview) | ❌ Not shipped | Only Apple's WebKit (Safari 26 / Tahoe) ships WebGPU. The GTK port has not. |

Source: https://github.com/gpuweb/gpuweb/wiki/Implementation-Status

This single fact rules out every wrapper that uses the **system webview on
Linux** (WebKitGTK): **Tauri, Neutralinojs, and any `wry`-based shell** cannot
give you WebGPU on Linux today.

---

## 3. The runtimes compared

### ✅ Electron — the recommended path
- **Bundles its own Chromium**, so you get the full Linux WebGPU support above
  regardless of the user's system browser.
- WebGPU is enabled in the main process:
  ```js
  // main.js (Electron)
  const { app, BrowserWindow } = require('electron');
  app.commandLine.appendSwitch('enable-features', 'Vulkan');
  app.commandLine.appendSwitch('use-vulkan');
  app.commandLine.appendSwitch('enable-unsafe-webgpu'); // for non-Intel/NVIDIA GPUs
  const win = new BrowserWindow({
    fullscreen: true, frame: false,
    webPreferences: { webgl: true, webgpu: true },
  });
  ```
- **Proven on Steam.** Real shipped examples:
  - Phaser + Electron Steam guide (gamedevjs.com, jacklehamster).
  - "The Supernatural Power Troll" / "The Test of Insanity" on Steam (Electron).
  - A BabylonJS + Divine Voxel Engine game shipped to Steam as Electron.
  - narrat (HTML5 narrative RPG engine) → Electron → Steam.
  - "Show HN: sold 30,000+ units of an HTML5/Electron game on Steam."
- **Steamworks integration** via Node bindings:
  - `steamworks.js` (modern, actively maintained Node binding for Steamworks SDK).
  - `greenworks` (older alternative).
  - Guide: https://liana.one/integrate-electron-steam-api-steamworks
- Downside: ~150 MB bundle (Chromium). For a game that's a non-issue.

### ✅ NW.js — equivalent to Electron
- Also bundles Chromium; same WebGPU story. Slightly less popular for games
  than Electron today; fewer Steam guides. No real advantage over Electron
  for this use case.

### ✅ CEF (Chromium Embedded Framework) — pro/heavier
- Embed Chromium in a C++ (or Rust) host. Full WebGPU.
- Used by AAA games for in-game UI. **Coherent Gameface** is a commercial,
  game-focused CEF product (licensed, $$).
- More integration work than Electron; only worth it if you need a custom
  native host (e.g., your engine core is Rust/C++ and you want the webview as
  a UI layer rather than the whole app).

### ❌ Tauri — NOT viable on Linux for WebGPU (today)
- Uses WRY → **WebKitGTK on Linux**. WebKitGTK has **not shipped WebGPU**.
- The Tauri community itself flags WebKitGTK as problematic: an open discussion
  "Webkit is totally unstable, so we need to use chromium or firefox instead"
  (#8524) notes WebKitGTK is "unusable," and a Chromium/Firefox backend is
  under discussion but **not officially available**.
- Tauri 2's webview-versions doc acknowledges WebKitGTK versions vary wildly
  across distros and is "a very incomplete list."
- Tauri is great for small native apps with plain DOM UI, but it is the **wrong
  choice for a WebGPU game engine on Linux** until it ships a Chromium backend.

### ❌ Neutralinojs — same problem
- Uses the system webview (WebKitGTK on Linux). No WebGPU on Linux.

---

## 4. Linux WebGPU caveats (apply to Electron/Chromium too)

Even with Chromium bundled, WebGPU on Linux is **GPU/driver-gated**:
- ✅ Auto-enabled: Intel Gen12+, NVIDIA (driver 535.183.01+), on Wayland.
- 👷 Others need `--enable-unsafe-webgpu` (+ possibly
  `--ozone-platform=x11 --use-angle=vulkan --enable-features=Vulkan`).

For a Steam release you **control the launch flags** (Electron main process or
a wrapper shell script), so you can force-enable WebGPU for all users. This is
acceptable for a game (you're not a browser). Just be aware some older/odd GPUs
may still fail `navigator.gpu.requestAdapter()` — keep the WebGL2 fallback path
from the previous research note (`WebGPURenderer` auto-falls back to WebGL2).

---

## 5. Recommendation for afterglow-engine

1. **Use Electron as the desktop runtime.** It bundles Chromium → full WebGPU
   on Linux (Intel/NVIDIA auto; others via flag), and has a proven Steam
   shipping pipeline with `steamworks.js`.
2. **Force WebGPU flags in the Electron main process** (`enable-unsafe-webgpu`,
   Vulkan features) so all users get the GPU path regardless of GPU/driver.
3. **Keep the Three.js WebGL2 fallback** for GPUs where `requestAdapter()`
   returns null.
4. **Do not use Tauri/Neutralinojs** — WebKitGTK lacks WebGPU on Linux.
5. Consider **CEF/Coherent Gameface** only if you later want the webview as a
   UI layer over a native (Rust/wgpu) core rather than as the whole engine.
6. Watch for **Firefox to ship WebGPU on Linux** (expected 2026) — not yet a
   packaging option, but relevant if you ever offer a Firefox-based runtime.

---

## Sources

- WebGPU implementation status (gpuweb wiki): https://github.com/gpuweb/gpuweb/wiki/Implementation-Status
- Electron WebGPU enable issue #26944: https://github.com/electron/electron/issues/26944
- Electron + WebGPU flags (utsubo migration guide): https://www.utsubo.com/blog/webgpu-threejs-migration-guide
- Tauri "WebKitGTK is unusable" discussion #8524: https://github.com/orgs/tauri-apps/discussions/8524
- Tauri webview-versions reference: https://v2.tauri.app/reference/webview-versions/
- Publish web games on Steam with Electron (gamedevjs): https://gamedevjs.com/tutorials/publishing-web-games-on-steam-with-electron/
- Electron + Steamworks integration guide: https://liana.one/integrate-electron-steam-api-steamworks
- BabylonJS voxel game shipped to Steam via Electron: https://dev.to/lucasdamianjohnson/i-finally-published-my-first-game-to-steam-using-electron-my-own-voxel-engine-2ikc
- "Show HN: 30k units of HTML5/Electron Steam game": https://news.ycombinator.com/item?id=12413144
- steamworks.js (Node Steamworks binding): https://github.com/ceifa/steamworks.js
