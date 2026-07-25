# Verify Your Install

Build and run `afterglow-shell` to confirm your toolchain, the native WebGPU
stack, and the DOM/Blitz/Vello HUD all work together.

## Build

```sh
nix-shell shell.nix --run "cargo build -p afterglow-shell"
```

## Run

```sh
nix-shell shell.nix --run "cargo run -p afterglow-shell"
```

With no arguments the shell runs its bundled `native_game.ts` module on the
shared wgpu surface. A window opens and presents a WebGPU scene with the
Vello-rasterized HUD composited on top. On the console you should see the
adapter line name your real GPU (e.g. `amd/rdna-2`).

You can also point the shell at an official Three.js HTML document directly:

```sh
nix-shell shell.nix --run "cargo run -p afterglow-shell -- /path/to/three.js/examples/webgpu_clearcoat.html"
```

If the window opens and the adapter line names your real GPU, your install is
good. The shell fails closed on device loss — never accept a software/WebGL
fallback path.

> **Native host parity:** `afterglow-cef` has been removed and `afterglow-shell`
> is the sole native host. The shell does not yet compose native `afterglow-rpc`
> workers, load Afterglow asset roots, or expose a production game bootstrap API.
> Until those land, the five engine demo pages (`dungeon.html`, `vt-demo.html`,
> etc.) cannot be launched the way the removed CEF examples launched them. See
> `docs/implementation/shell-promotion-plan.md`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| No WebGPU adapter / SwiftShader | Vulkan loader wiring — re-source the devshell; check `VK_ICD_FILENAMES`; see [Debugging](../reference/debugging.md). |
| No window on Wayland | The app runs under XWayland; ensure `DISPLAY=:0` (the devshell sets this). |
| Device loss / fatal error | The shell fails closed; check the Vulkan ICD points at the real GPU, not SwiftShader. |

## Next

Read [Building — `afterglow-shell`](../building/afterglow-shell.md) for the full
presenter surface, or jump to [Defining a Service](../workers/defining-a-service.md)
to write a worker.
