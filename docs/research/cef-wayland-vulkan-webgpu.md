# CEF on native Wayland + Vulkan + WebGPU

**Date:** 2026-07-08
**Question:** Does CEF (Chromium Embedded Framework) work natively with
Wayland + Vulkan + WebGPU?

**Verdict:** **Yes for windowed (Electron-style) rendering** — which is the
afterglow-engine case (Three.js = full-window renderer). WebGPU runs via
Dawn → Vulkan on Wayland with explicit flags. **No for OSR** (offscreen /
webview-as-texture overlay) on native Wayland — CEF forces OSR to X11.

---

## The critical split: windowed vs OSR

CEF has two rendering modes, and they have **opposite** Wayland support:

### Windowed (the webview owns the whole window) — ✅ works on Wayland
This is the Electron-style / Three.js-full-window case. CEF's Ozone/Wayland
backend (upstreamed by Collabora in 2019) drives a native Wayland window and
the GPU composites directly. WebGPU + Vulkan work via flags (§1).

Caveats (open, polish-level — not blockers for a fullscreen game):
- `#2804` (open, 5+ yrs) "Add support for embedded Ozone/Wayland windows" —
  Ozone/Wayland only works with the **views framework** and **cannot embed the
  webview into a host window**. Fine for full-window; blocks "webview inside my
  native window" embedding.
- `#4178` / `#4061` GTK-themed/old Chrome window decorations on Wayland.
- `#4181` frameless window shadows/rounded corners.
- `#4120` keyboard focus issues on Linux/X11 (Wayland-adjacent).
- Reddit (2025): Wayland impl was stalled ~1 year, recently revived by a new
  developer — still maturing but functional.

### OSR (offscreen / webview-as-texture) — ❌ not on native Wayland
This is the "composite the webview as a texture over a native GPU surface"
pattern (e.g., HTML UI overlay on a wgpu game). **Broken on native Wayland:**
- `#3953` (closed 2026-01) "Default to use-angle=gl-egl and
  **ozone-platform=x11** with shared textures" — OSR shared-texture mode
  **forces X11**.
- `#3954` (closed) "Force use-angle=gl-egl and **ozone-platform=X11** for
  Linux OSR shared texture mode" — DMABUF/EGL OSR requires X11; works on
  Intel/AMD Mesa, NVIDIA needs `--use-angle=gl-egl`.
- `#3687` (open, 2024) "osr: linux: Add cefclient implementation for
  **OnAcceleratedPaint**" — the accelerated OSR paint path has **no Linux
  cefclient implementation** at all.

So OSR on Linux effectively **requires X11 (XWayland)**, not native Wayland.
Irrelevant for afterglow-engine (Three.js renders the full window, no overlay),
but would matter for a "native wgpu game + HTML UI overlay" design.

---

## 1. The flags that make windowed Wayland+Vulkan+WebGPU work

CEF is Chromium, so it inherits Chromium's Linux GPU flags. None are on by
default; for an embedded app **you control the launch flags**, so this is a
configuration step, not a blocker:

```
--ozone-platform=wayland          # native Wayland (not XWayland)
--enable-features=Vulkan          # Vulkan GPU backend (Linux/Android flag)
--use-angle=vulkan                 # (or --use-angle=gl-egl for some drivers)
--enable-unsafe-webgpu            # enable WebGPU (Dawn) — same as Chrome on Linux
--ignore-gpu-blocklist            # force-enable on GPUs Chromium would block
```

- **Vulkan** is gated to `--enable-features=Vulkan` (Linux/Android only; macOS
  has no Vulkan, Windows supports Vulkan but the flag is Linux/Android in
  Chromium). Confirmed via CEF forum (M131): "Vulkan: Disabled… I have to
  manually add `--enable-features=Vulkan`."
- **WebGPU**: CEF includes Dawn (Chromium's WebGPU). Disabled by default;
  enable with `--enable-unsafe-webgpu`. Confirmed: forum "Does CEF include the
  Dawn WebGPU implementation? — it must since Chromium has it."
- This is **the same WebGPU-on-Linux story as Chrome/Electron** — Intel Gen12+
  and NVIDIA 535+ auto-enable; others need the unsafe flag. On Steam Deck
  (AMD APU), it works.

End result: windowed CEF on Wayland = "Chrome on Wayland with WebGPU," which
works today.

---

## 2. What this means for afterglow-engine

- **Graphics-viable: YES.** CEF on native Wayland + Vulkan + WebGPU works for
  the full-window Three.js renderer, via flags. Steam Deck (SteamOS, Wayland,
  AMD APU) is a supported target.
- **The blockers are NOT graphics:**
  1. **Size** — CEF bundles Chromium (~100–150 MB+). Not lightweight; the
     `wef` authors measured ~1 GB with the framework. Same weight class as
     Electron.
  2. **Rust bindings are immature** (`browser-window` 48★, `cef-ui` stale,
     `wef` abandoned) — you'd host CEF from C++ or maintain a thin Rust
     wrapper, adding integration complexity (CEF app bundles, helper processes,
     macOS bundle structure).
  3. **Build complexity** — CEF build/linking is non-trivial vs. Electron's
     turnkey npm workflow.
- So CEF is graphics-capable but offers **no advantage over Electron** for this
  stack: same Chromium weight, same WebGPU, worse tooling. If you're paying the
  Chromium cost anyway, Electron is the lower-effort choice. CEF only wins if
  you need the webview as a *texture overlay* on a native GPU surface — and
  that's exactly the path that doesn't work on native Wayland (OSR → forced
  X11), so it doesn't even win there.

---

## Sources

- CEF Ozone/Wayland upstreamed (Collabora, 2019): https://www.collabora.com/news-and-blog/blog/2019/05/08/cef-on-wayland-upstreamed/
- CEF Wayland progress (Phoronix): https://www.phoronix.com/news/Chromium-CEF-Wayland-Progress
- `#2804` embedded Ozone/Wayland windows (open, 5+ yrs): https://github.com/chromiumembedded/cef/issues/2804
- `#3953` OSR defaults to ozone-platform=x11 (closed): https://github.com/chromiumembedded/cef/issues/3953
- `#3954` Force ozone-platform=X11 for OSR shared textures (closed): https://github.com/chromiumembedded/cef/issues/3954
- `#3687` OSR OnAcceleratedPaint has no Linux cefclient (open): https://github.com/chromiumembedded/cef/issues/3687
- CEF forum: Vulkan enable with M131: https://magpcss.org/ceforum/viewtopic.php?f=6&t=20090
- CEF forum: Does CEF include Dawn WebGPU?: https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19680
- WebGPU impl status (Chromium/Dawn Linux Vulkan): https://github.com/gpuweb/gpuweb/wiki/Implementation-Status
