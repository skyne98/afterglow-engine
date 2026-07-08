# latency-tool — automated input→present latency measurement

Measures **input→present** latency for the cef-rs prototype via the Chrome
DevTools Protocol (CDP), fully automated and reproducible (no hardware).

## What it measures

`input event (blink dispatch) → next SkiaRenderer::SwapBuffers (frame present)`,
all in Chromium's trace clock (no wall-clock alignment). It:

1. Connects to CEF's browser-level CDP endpoint (`/json/version` → `webSocketDebuggerUrl`).
2. Finds the page target via `Target.getTargets` + `Target.attachToTarget` (CEF Views
   browsers aren't listed in `/json/list`).
3. `Tracing.start` (blink/cc/gpu/… categories).
4. For N iterations: `Input.dispatchMouseEvent` (synthetic click) — the input events
   themselves (`EventDispatch`, `handleMousePressEvent`, …) become the trace-clock markers.
5. `Tracing.end`, drain `dataCollected` + `tracingComplete`.
6. For each input marker ts, find the next `SkiaRenderer::SwapBuffers` ts → latency.
   Reports min / median / mean / p90 / max.

## Scope / caveat

CDP-dispatched input **bypasses the OS input stack** (kernel→evdev→Wayland/X→CEF UI
thread), so this is a **lower bound** on true input→present. The OS→renderer portion
needs real input (uinput/xdotool) or hardware (photodiode / OpenLDAT — see
`docs/research/cef-games-latency-footprint-debugging.md`). What this *does* capture
reliably and reproducibly: renderer input handling → compositor → present.

## Run

```sh
cd prototype/cef-webgpu
# 1. Launch the app with CDP enabled (AFTERGLOW_DEVTOOLS=9222):
env CEF_PATH="$PWD/target/debug" AFTERGLOW_DEVTOOLS=9222 nix-shell shell.nix --run \
  "./target/debug/afterglow-cef-webgpu --ozone-platform=x11" &
# 2. Wait for CDP, then run the tool:
./latency-tool/target/release/latency-tool
```

## Empirical result (this machine: NVIDIA Ampere, X11/XWayland, vsync-on)

```
markers=205 swaps=267 samples=27
min=0.07  median=3.59  mean=2.53  p90=5.49  max=5.57  (ms)
```

~2.5–3.6 ms median input→present, vsync-on. (Reproducible across runs.)

### Vsync-off was unstable here
Forcing `--disable-gpu-vsync` + `--disable-frame-rate-limit` +
`--run-all-compositor-stages-before-draw` made rendering **very choppy** and yielded
0 captured swaps on this CEF/NVIDIA/Linux setup — matching the "vsync-off can
destabilize" caveat in the research. Left off by default; pass on the CLI to
experiment.
