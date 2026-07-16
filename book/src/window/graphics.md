# Graphics & DevTools

The native shell forces WebGPU onto the real GPU through Dawn → Vulkan and
opens DevTools behind a port flag. Here's what you control and what's fixed.

## The GPU flags

These are applied internally (in `on_before_command_line_processing`, so they
propagate to all child processes) and only added if not already present — **CLI
flags win**, so you can override from the command line.

| Flag | Why | When |
|---|---|---|
| `--enable-unsafe-webgpu` | Enable WebGPU. | always |
| `--ignore-gpu-blocklist` | Use the real GPU even if "unsupported." | always |
| `--disable-webgl` | WebGPU is required; forbid a WebGL fallback. | always |
| `--enable-features=Vulkan` | Dawn → Vulkan backend. | always |
| `--use-angle=vulkan` | ANGLE over Vulkan. | always |
| `--ozone-platform=x11` | Wayland + Vulkan incompatible in CEF 149 → XWayland. | always (CLI-overridable) |
| `--disable-gpu-vsync` | Uncapped frame rate. | `vsync(false)` only |
| `--disable-frame-rate-limit` | Uncapped frame rate. | `vsync(false)` only |

The example confirms the real adapter was selected:

```text
WebGPU adapter: amd/rdna-2
```

If you see SwiftShader or a software adapter, the Vulkan loader wiring is wrong
— see [Debugging](../reference/debugging.md). Engine pages fail closed: their
WebGPU bootstrap rejects a missing adapter, initialization failure, or device
loss and shows an error rather than silently rendering through Three.js WebGL2.

## Vsync

Vsync is on by default (smooth, monitor refresh rate). For an uncapped frame
rate (e.g. benchmarking), pass `vsync(false)`:

```rust
AppBuilder::new()
    .vsync(false)
    // …
    .run();
```

This adds `--disable-gpu-vsync` and `--disable-frame-rate-limit`. Measure the
result with the [latency tool](../building/benchmarking.md) if input latency
matters.

## DevTools

`.devtools(port)` opens a remote debugging port:

```rust
AppBuilder::new()
    .devtools(9222)   // 0 = off (the default)
    // …
    .run();
```

Then open `http://127.0.0.1:9222` in a browser to inspect the page. Or attach
over CDP with the [latency tool](../building/benchmarking.md):

```sh
nix-shell shell.nix --run "cargo run -p latency-tool"                         # measure input→present
nix-shell shell.nix --run "cargo run -p latency-tool -- eval 'navigator.userAgent'"
nix-shell shell.nix --run "cargo run -p latency-tool -- nav afterglow://local/index.html"
```

> CEF Views browsers don't appear in `/json/list`. The latency tool uses
> `Target.getTargets` + `Target.attachToTarget` instead. If `eval` times out,
> the page may be running a synchronous JS task — use `awaitPromise: true` and
> ensure the JS yields.

## Console forwarding

`.on_console(f)` forwards every JS console message (severity, source, line) to
your callback. Default is stderr with a `[console]` prefix:

```rust
.on_console(|m| eprintln!("[js] {m}"))
```

This is the only JS→Rust callback the shell exposes.

## Why X11, not Wayland

In CEF 149, **Wayland + Vulkan are incompatible** — native Wayland + WebGPU
isn't available yet. So the shell defaults to `--ozone-platform=x11` and runs
under XWayland. The flag is CLI-overridable, but overriding it to `wayland` will
break WebGPU on this CEF version.

## Next

- [Defining a Service](../workers/defining-a-service.md) — write a worker.
- [Debugging](../reference/debugging.md) — the full troubleshooting playbook.
