# CEF for games: latency, footprint, debugging

**Date:** 2026-07-09
**Question:** How do people use CEF for games, and how do we guarantee the
lowest input→pixel latency, smallest footprint, and best debugging?

**TL;DR**
- **Real-world:** CEF powers Steam's UI, the GW2 in-game UI (ArenaNet replaced
  CoherentUI with CEF — ~3× faster), and the Battle.net/Epic launchers.
  Coherent Gameface is the commercial CEF-derived game-UI product.
- **Latency:** The worst CEF-for-games latency comes from **OSR-to-texture**
  (offscreen render → cross-process IPC → copy to game VRAM). **Our windowed
  architecture sidesteps this entirely** — Three.js renders the game *to the
  webview canvas*, so there's no texture-copy IPC. Remaining latency is the
  standard browser input→pixel pipeline, minimized by vsync-off + uncapped FPS
  + rAF + WebGPU + offloading logic to Rust workers.
- **Footprint:** CEF can't be feature-stripped via flags (official). Use the
  **Minimal** binary distribution, **strip symbols**, drop unused **locales**
  (~30 MB), drop **swiftshader** if Vulkan-only. Realistic floor ~80–110 MB.
- **Debugging:** `--remote-debugging-port` + Chrome DevTools (`chrome://inspect`),
  `chrome://tracing` for frame/input latency, `--enable-chrome-runtime`.

---

## 1. Who uses CEF for games (and what they learned)

| Project | Use | Notes |
|---|---|---|
| **Steam** (Valve) | Entire client UI | Customized CEF; replaced VGUI in June 2023. Steam ships a Chromium ~20 majors behind stable (stability over currency). Source: Valve Developer Community / Steam forums. |
| **Guild Wars 2** (ArenaNet) | In-game books, launcher, Trading Post | Replaced CoherentUI with CEF (2023 blog). CEF is **~3× faster** (Trading Post load: 6.99ms vs 19.24ms). Used by Steam/Battle.net/Epic launchers per ArenaNet. |
| **Battle.net / Epic Games launcher** | Launcher UI | CEF-based. |
| **Coherent Gameface** | Commercial game-UI middleware | CEF-derived (not raw CEF); claims **<1ms/frame** UI by optimizing hard for games (zero-copy texture path, game-focused input). The reference for "CEF, but for games." |

**Key lessons from ArenaNet's GW2 writeup:**
- They use CEF for **overlay/UI surfaces** (OSR-to-texture pattern), not as the
  main renderer.
- **Crash resilience is critical:** a launcher crash = "player can't play."
  They rolled back the CEF release **twice** due to obscure, unreproducible
  crashes on a tiny fraction of hardware configs. → Ship telemetry, fallback
  flags, and test on many configs.
- Keep CEF on a reasonable update cadence (don't fall 20 majors behind like
  Steam, or you lose perf/security fixes).

**Coherent Labs' critique of raw CEF for games** (the definitive "what to
consider" article):
- CEF wasn't designed for real-time apps. The multi-process architecture means
  **rendering is in a separate OS process** from the game.
- OSR-to-texture requires **≥2 GPU VRAM↔RAM copies per frame** across processes
  → expensive, hurts smoothness (animations/video).
- **IPC lag:** CEF's inter-process IPC was built for desktop browsers →
  "several millisecond lags" undesirable for games.
- **Async by design:** "rendering is never in sync with the 3D engine and
  usually lags several frames." OK for menus/browsers; **not OK for
  3D-anchored UI** (nameplates that must track the player).
- Software-rendered OSR avoids the GPU copies but loses modern HTML features,
  and Chromium is removing the software OSR path.

> **Why this doesn't doom us:** the Coherent critique is about the
> **OSR-overlay** pattern (web UI composited into a native game frame). Our
> architecture is **windowed** — Three.js renders the game directly to the
> webview's canvas via WebGPU. There is **no game↔web texture copy, no
> game-process-vs-renderer-process desync** — the webview *is* the frame. This
> is the lowest-latency way to use CEF for a game.

Sources:
- https://www.guildwars2.com/en/news/inside-arenanet-chromium-embedded-framework-in-guild-wars-2/
- https://coherent-labs.com/posts/what-developers-should-consider-when-using-chromium-embedded-framework-cef-in-their-games/
- https://coherent-labs.com/products/coherent-gameface/ (Gameface, <1ms/frame claim)
- https://developer.valvesoftware.com/wiki/Chromium_Embedded_Framework (Steam)

---

## 2. Input→pixel latency — the pipeline & how to minimize it

### The Chromium frame pipeline (what you're optimizing)
Chromium's input→pixel path (see "Life of a frame" in Chromium docs):
```
OS input event
  → CEF UI thread (Browser process)
  → IPC to Renderer process
  → JS event handler (main thread)  ← keep this free!
  → requestAnimationFrame callback  ← Three.js render here
  → WebGPU/WebGL render
  → compositor (GPU process)
  → present (scanout, vsync-gated by default)
```
Each arrow is potential latency. The big, controllable ones: **vsync** (waits up
to a full refresh), **frame-rate cap**, and **main-thread blocking**.

### Our windowed architecture already wins the structural latency fight
- **No OSR texture copy, no game/renderer desync.** Three.js → WebGPU canvas →
  compositor → screen, all in-process-of-the-webview. The Coherent critique's
  worst offenders (cross-process texture IPC, frame lag) don't apply.
- **Input goes straight to the webview** — no game→CEF coordinate conversion
  or manual `SendInputEvent` routing needed (that's an OSR concern).
- **Rust WASM workers carry the sim** off the JS main thread, so the rAF render
  callback isn't blocked by physics/netcode — directly cuts input→rAF latency.

### Flags to minimize latency (force in `on_before_command_line_processing`)
```sh
--disable-gpu-vsync                 # biggest win: don't wait for vsync
--disable-frame-rate-limit          # uncap FPS (render as fast as possible)
--run-all-compositor-stages-before-draw   # finish compositor work before draw (trades throughput for latency)
--enable-begin-frame-scheduling     # BeginFrame-driven scheduling
--ignore-gpu-blocklist              # ensure GPU accel (already set)
```
- **`--disable-gpu-vsync`** is the single biggest latency reducer (uncapped →
  300+ FPS in tests). ⚠️ Known CEF/Chromium bug: after a **touch/resize** the
  rate can reset to 60 FPS — re-assert the flag and avoid resizing mid-game.
  (https://stackoverflow.com/questions/27753708)
- **`--disable-frame-rate-limit`** pairs with vsync-off to actually hit high
  FPS. On its own (vsync on) it just uncaps rAF but still presents at refresh.
- **`--run-all-compositor-stages-before-draw`** can shave a frame of latency at
  the cost of throughput — useful for latency-critical UI, test both ways.

### JS-side latency techniques
- Render in `requestAnimationFrame` (aligns to the compositor; lower latency
  than `setInterval`/`setTimeout`). Three.js `setAnimationLoop` uses rAF.
- **Keep the main thread idle between input and rAF** — that's exactly why the
  heavy compute lives in Rust workers (postMessage / SharedArrayBuffer), not on
  the JS main thread.
- Minimize compositor layers / avoid animated CSS box-shadows/filters on hot
  elements.
- Use **WebGPU** (lower per-frame CPU overhead than WebGL; the prototype
  confirms it works on the real GPU).

### If OSR is ever needed (webview-as-texture overlay): use zero-copy
cef-rs ships an **`accelerated_osr`** feature (wgpu + ash + DMABUF/native
pixmap) — the bindings expose `AcceleratedPaintInfo`, `shared_texture_enabled`,
and native-pixmap planes. This is the **zero-copy shared-texture** OSR path
that avoids the VRAM→RAM→VRAM copies Coherent warned about. Use this (not the
legacy software `OnPaint` byte-buffer path) if you ever overlay HTML UI on a
native surface. (On Linux it's still forced to X11 for shared textures — see
`cef-wayland-vulkan-webgpu.md`.)

Sources:
- Chromium "Life of a frame": https://chromium.googlesource.com/chromium/src/+/lkgr/docs/life_of_a_frame.md
- FrameViewer / tracing: https://www.chromium.org/developers/how-tos/trace-event-profiling-tool/using-frameviewer/
- vsync-off + rAF latency: https://github.com/w3c/html/issues/375 , #785

---

## 3. Smallest possible CEF footprint

**Official answer (Czarek, CEF maintainer):** "CEF doesn't support disabling
features to reduce its size." You can't flag-off functionality. Reduction is
about distribution choice + stripping, not feature flags.

### Use the right binary distribution
cef-builds offers variants; cef-rs downloads **`cef_binary_*_minimal`** by
default — that already excludes the `cefclient` example app and debug symbols.
Don't ship the `Client` or `Debug+Symbols` distributions.

### Strip symbols (Linux/Mac)
`strip libcef.so` / `strip libcef.dylib`. Debug+Symbols distributions are
~1.4 GB; stripped release is ~150 MB. cef-rs's `bundle-cef-app --release` does
a smaller Linux bundle.

### Drop unused files
- **Locales:** `locales/` ships ~100 language `.pak` files (~30 MB). Keep only
  `en-US.pak` (set `CefSettings.locale="en-US"` + `locales_dir_path`). Biggest
  easy win.
- **swiftshader** (`libvk_swiftshader.so`, `vk_swiftshader_icd.json`,
  `libEGL.so`/`libGLESv2.so` software fallbacks): if you're Vulkan/WebGPU-only
  and force the real GPU ICD (as the prototype does), you *may* drop swiftshader.
  Risky — some compositing fallbacks use it. Test thoroughly.
- **`chrome-sandbox`:** keep unless `no_sandbox` (security). The prototype runs
  `no_sandbox` for dev convenience only.

### Realistic floor
~80–110 MB stripped minimal + en-US locale only. You cannot get CEF to
"Electron-lite" size — it bundles Chromium. (If size is the priority over
Rust-native, that's the Tauri tradeoff, at the cost of Linux WebGPU — see
`lightweight-rust-chromium-shell.md`.)

### Compress the bundle
SquashFS/UPX on the shipped tree can cut on-disk size, but UPX triggers AV
false-positives and complicates the loader — avoid for a game.

Sources:
- https://stackoverflow.com/questions/8222571 (min size, "no #define")
- https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=15729 (Czarek: can't disable features; strip)
- https://stackoverflow.com/questions/38354107 (70–90 MB normal)

---

## 4. Best debugging

### DevTools (JS/DOM/network/performance)
- Enable via **`CefSettings.remote_debugging_port = 9222`** (or
  `--remote-debugging-port=9222`). Then attach Chrome DevTools at
  `chrome://inspect` → "Configure..." → `localhost:9222`, or open
  `http://localhost:9222` in a real Chrome.
- `--enable-chrome-runtime` gives the full Chrome DevTools UI (better than the
  default CEF devtools window).
- Programmatic: `browser_host.show_dev_tools()` / `close_dev_tools()`.
- The prototype already forwards JS `console.log` to stderr via
  `DisplayHandler::on_console_message` — keep that for headless/CI logs.

### Frame & input latency profiling
- **`chrome://tracing`** ("Performance" panel / "Record trace") — captures the
  full input→compositor→present pipeline with per-stage latency (the
  `EventLatency` and `BeginMainFrame`/`Compositor` rows are what you read).
- **FrameViewer** (in the trace, enable "Frame viewer") to inspect layer
  compositing/invalidation — finds accidental repaints/extra layers that add
  latency.
- Capture a trace while interacting; the gap between the input event and the
  corresponding `DrawAndSwap`/present is your true input→pixel latency.

### Native/host debugging
- Set `CefSettings.log_severity = LOGSEVERITY_VERBOSE` + `--enable-logging=stderr`
  for CEF/Chromium native logs (GPU process, IPC, sandbox).
- `--v=1`/`--v=2` for verbose Chromium module logging (noisy; use targeted
  `--vmodule=`).
- The NixOS `shell.nix` already surfaces GPU/Vulkan init lines (Dawn→Vulkan
  adapter selection, ICD resolution) — where the swiftshader-fallback issue
  was diagnosed.

### Crash/telemetry (the GW2 lesson)
- CEF crashes in helper/GPU processes are easy to miss. Wire **crashpad**
  (`CefSettings.crash_reporter_enabled`) or a Rust panic hook + minidump
  upload, because ArenaNet's experience shows obscure config-specific crashes
  only surface at scale.

Sources:
- https://www.chromium.org/developers/how-tos/trace-event-profiling-tool/using-frameviewer/
- Remote debugging: `--remote-debugging-port` (CefSettings.remote_debugging_port)
- https://stackoverflow.com/questions/29117882 (JS debugging in CEF, --enable-chrome-runtime)

---

## 5. Recommendations for afterglow-engine

1. **Stay windowed** (the prototype's choice). It is structurally the
   lowest-latency CEF mode — no OSR texture copies, no renderer/game desync.
2. **Force latency flags** in `on_before_command_line_processing`:
   `--disable-gpu-vsync`, `--disable-frame-rate-limit`,
   `--run-all-compositor-stages-before-draw`, `--enable-begin-frame-scheduling`
   (on top of the WebGPU/Vulkan flags already there). Document the vsync-reset
   caveat.
3. **Keep JS main thread idle** — the Rust WASM workers carry the simulation;
   the rAF callback only renders. This is the architecture's latency moat.
4. **Footprint:** ship the **Minimal** CEF dist, **strip** libs, keep **en-US**
   locale only (set `locale` + `locales_dir_path`). Floor ~80–110 MB.
5. **Debugging:** expose `remote_debugging_port` behind a dev flag; keep the
   `console.log`→stderr forwarding; use `chrome://tracing` to measure
   input→present and assert a latency budget in CI.
6. **Telemetry/crashpad** before launch — the GW2 rollbacks show config-specific
   CEF crashes only appear at scale.
7. **Keep CEF current** (cef-rs tracks Chromium monthly) — don't drift like
   Steam's 20-major-behind build.

## Sources (consolidated)
- ArenaNet GW2 + CEF: https://www.guildwars2.com/en/news/inside-arenanet-chromium-embedded-framework-in-guild-wars-2/
- Coherent "using CEF in games": https://coherent-labs.com/posts/what-developers-should-consider-when-using-chromium-embedded-framework-cef-in-their-games/
- Coherent Gameface: https://coherent-labs.com/products/coherent-gameface/
- Valve/Steam CEF: https://developer.valvesoftware.com/wiki/Chromium_Embedded_Framework
- Chromium Life of a frame: https://chromium.googlesource.com/chromium/src/+/lkgr/docs/life_of_a_frame.md
- FrameViewer tracing: https://www.chromium.org/developers/how-tos/trace-event-profiling-tool/using-frameviewer/
- vsync-off + rAF latency: https://github.com/w3c/html/issues/375
- disable-gpu-vsync reset bug: https://stackoverflow.com/questions/27753708
- CEF min size / can't disable features: https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=15729 , https://stackoverflow.com/questions/8222571
- cef-rs `accelerated_osr` (zero-copy): `cef/src/osr_texture_import.rs`, bindings `AcceleratedPaintInfo`/`shared_texture_enabled`
- Remote debugging: CefSettings.remote_debugging_port / https://stackoverflow.com/questions/29117882
