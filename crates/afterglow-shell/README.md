# afterglow-shell

Afterglow's lightweight native shell for unmodified Three.js WebGPU
applications, built on rusty_v8, deno_webgpu/wgpu-core, winit, LinkeDOM, Stylo,
Taffy, Blitz, and Vello.

`afterglow-shell` is the sole native host; `afterglow-cef` has been removed.
It is not a second web engine or a client-code compatibility layer. Remaining
parity work is tracked in `docs/implementation/shell-promotion-plan.md`.

## Current architecture

- `src/main.rs` is a real native WebGPU presenter. It creates the winit surface
  through the same wgpu-core `Global` used by deno_webgpu, renders JavaScript
  WebGPU commands directly into the surface, and presents without full-frame
  CPU readback or a second presentation device.
- `src/runtime/` owns typed lifecycle, readiness, production monotonic time, and
  deterministic test time.
- `src/testing/browser_runner.rs` owns the headless unmodified-example runner.
  `examples/browser_test.rs` is deliberately only a thin executable wrapper.
- LinkeDOM is the JavaScript-facing DOM. A structured WeakMap-ID bridge keeps
  one long-lived Blitz document synchronized without HTML markers or reparsing.
- Blitz/Stylo/Taffy provide cascade, computed style, layout, box metrics,
  scrolling, focus/hover/active state, hit stacks, observers, and page paint.
- deno_webgpu uses the locally patched wgpu-core 29 stack. The primary window
  surface and JavaScript WebGPU device share that stack.

The native presenter runs either `native_game.ts` or a real unmodified official
three.js `examples/*.html` document continuously on the shared surface. For HTML
examples it applies the document's import map, evaluates its module script
verbatim, loads its assets, and drives its native animation loop. Its production
LinkeDOM/Blitz emits a Vello scene for the transparent HUD; Vello rasterizes it
entirely on the shared GPU device into a persistent texture, which is
alpha-composited after the game pass. Vello uses 16-sample GPU antialiasing,
straight-alpha compositing, synthesized bold faces when a requested weight is
absent, and GPU-drawn CSS text shadows. No HUD pixels are rasterized or staged
on the CPU. Native input is hit-tested through the real
DOM, so HUD controls and the game canvas share pointer, wheel, keyboard, and
focus routing. The native OS cursor follows computed CSS `cursor` values and
browser-semantic link/text defaults. The surface-backed WebGPU canvas remains a
layout/hit-test element but is excluded from the Vello HUD scene, preventing it
from erasing unchanged reactive UI during repaint. Resize/reconfiguration,
adapter capability reporting, suspend / resume, device-loss termination,
frame-coalesced pointer motion, and a trailing
75 ms native resize debounce are wired. Rendering pauses during the debounce so
an obsolete swapchain extent is never acquired. The headless runner remains the
full unmodified `examples/*.html` compatibility and screenshot-validation path.

## Runtime lifecycle

Startup follows explicit states rather than sleeps:

```text
Created → EnvironmentReady → AdapterReady → DeviceReady → CanvasReady
        → ResourcesReady → RendererReady → Running
        → Suspended | DeviceLost | Stopped
```

The deterministic runner does not capture merely because its rAF callback
returned. It requires both a registered canvas and a real `GPUQueue.submit()`.
This fixes the intermittent no-canvas races in `webgpu_lights_dynamic` and
`webgpu_tsl_vfx_flames`; both now pass repeated runs at 0.001% and 0.039%.

## Source layout

```text
src/main.rs                         direct native three.js/WebGPU surface presenter
native_game.ts                      production game module using unmodified three.js
src/runtime/lifecycle.rs            validated runtime state machine
src/runtime/readiness.rs            typed pending operations/canvas/submission readiness
src/runtime/clock.rs                production and deterministic clock sources
src/testing/browser_runner.rs       unmodified three.js example runner
src/browser.rs                      LinkeDOM ↔ Blitz reconciliation/composition
src/native_browser.rs               production DOM ops and Vello GPU scene source
examples/browser_test.rs            thin test-runner entry point
dom_setup.ts                        browser APIs and native bridge
canvas_2d.ts                        software Canvas2D environment
vendor/deno_webgpu/                 local native WebGPU patches
vendor/naga/                        local shader/backend patches
../../vendor/afterglow-shell-blitz/ pinned layout/paint engine and regression tests
vendor/linkedom/                    JavaScript DOM implementation
```

Obsolete CDP, clear-color, duplicate-device, and standalone canvas probe
executables were removed from the product build. Cargo uses `autoexamples =
false`; the only retained example executable is the real browser runner.

## Build

```bash
cd ~/Project/afterglow-engine
curl -L -o /tmp/v149_simdutf.a.gz \
  https://github.com/denoland/rusty_v8/releases/download/v149.4.0/librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz
gzip -dc /tmp/v149_simdutf.a.gz > /tmp/v149.a
export RUSTY_V8_ARCHIVE=/tmp/v149.a

# Native window runtime
cargo build --locked -p afterglow-shell

# Unmodified three.js example runner
cargo build --locked -p afterglow-shell --example browser_test
```

Run the native presenter in a graphical session, optionally with an official
three.js HTML example:

```bash
cargo run --locked -p afterglow-shell
cargo run --locked -p afterglow-shell -- /tmp/threejs/examples/webgpu_clearcoat.html
```

Run an unmodified example headlessly:

```bash
./target/debug/examples/browser_test /tmp/threejs webgpu_multiple_elements /tmp/out.png
cd crates/afterglow-shell/cdp_client
bun ../e2e/diff_pct.ts /tmp/out.png /tmp/threejs/examples/screenshots/webgpu_multiple_elements.jpg
```

## Browser/game environment

The compatibility runner provides:

- unmodified HTML import maps and three.js/addon modules;
- real Inspector and OrbitControls;
- WebGPU capture canvases and full Blitz page composition;
- file/data/blob/HTTPS resources and deterministic readiness accounting;
- image decode, `createImageBitmap`, KTX2/Draco workers, Blob, Storage, and
  software Canvas2D;
- computed style, DOMRect and box metrics;
- `elementFromPoint()` and front-to-back `elementsFromPoint()`;
- pointer/mouse/wheel/keyboard/focus events and pointer capture;
- cancelable form-control defaults;
- scrolling, ResizeObserver, IntersectionObserver, and Stylo `matchMedia()`;
- deterministic rAF/timers/randomness for screenshot tests.

Canvas is a first-class inline replaced element in the pinned Blitz fork. The
fork also contains initial-containing-block percentage fixes, viewport-correct
fixed positioning, live checked state, public Stylo computed values, multi-hit
traversal, and exact external canvas raster paint support.

## Validation status

The game-engine profile contains 182 screenshot-backed examples after three.js's
own XR/forced-WebGL exclusions.

- Both former renderer-readiness errors pass 10/10 byte-identical runs each.
- The only intentional runtime exclusions are the two video/WebCodecs examples.
- 39 examples remain above the strict bundled-Chrome `<0.1%` pixel threshold in
  the latest complete NVIDIA run (141 pass, 39 rendered differences, 2 optional
  video errors). These are
  predominantly exact text/DOM paint, temporal first-frame state, texture/MSAA
  sampling, or compute diagnostics; they are not runtime crashes.
- Representative results: `multiple_elements` 0.039%, `clipping` 0.023%,
  `water` 0.003%, `materials_basic` 0.000%, `lights_dynamic` 0.001%, and
  `tsl_vfx_flames` 0.039%.

Same-adapter Chrome 150/NVIDIA/Vulkan diagnostics identify the bundled golden's
adapter/backend as the dominant cause for the core differences. Ten of the
remaining eleven core examples are below the strict threshold on the same
adapter: seven match exactly, lines differ by 0.001%, refraction by 0.003%, and
rect-area lights by 0.013%. Anisotropy falls from 9.108% to an exact match;
BPCEM falls from 2.543% to 0.109%. Per-example evidence is recorded in
`e2e/gpu_fidelity_diagnosis.json`; captures are reproducible with
`scripts/capture_same_adapter_chrome.ts`. Driver-specific deterministic visual output
is pinned in `e2e/runtime_goldens/nvidia-rtx3090` and can be checked with:

```bash
scripts/verify_runtime_goldens.sh /tmp/runs
```

For a game engine, core texture, depth, MSAA, compute state/order, initialization,
and device-loss correctness remain strict gates. Exact Chrome/Skia glyph pixels,
full browser navigation/forms, and video are not core release blockers.

## Implementation contract

- [`AGENTS.md`](AGENTS.md) — mandatory no-stub/no-client-modification rules.
- [`BLITZ_LINKEDOM_COMPLETE_FIX_PLAN.md`](BLITZ_LINKEDOM_COMPLETE_FIX_PLAN.md) —
  DOM/layout atomic bridge contract.
- [`GAME_ENGINE_RUNTIME_ATOMIC_IMPLEMENTATION_PLAN.md`](GAME_ENGINE_RUNTIME_ATOMIC_IMPLEMENTATION_PLAN.md)
  — production native-runtime cutover and validation plan.
- [`vendor/NATIVE_WEBGPU_PATCHES.md`](vendor/NATIVE_WEBGPU_PATCHES.md) — native
  WebGPU patch inventory and upstream references.

No example source, addon, or extracted module is rewritten. Missing behavior is
implemented in the runtime environment or native backend and tested at that
layer.
