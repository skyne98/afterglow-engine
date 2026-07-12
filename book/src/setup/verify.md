# Verify Your Install

Build and run the `minimal` example to confirm your toolchain, CEF, and WebGPU
all work together.

## Build

```sh
nix-shell shell.nix --run "cargo build --example minimal -p afterglow-cef"
```

The first build fetches the CEF distribution (see
[Prerequisites](./prerequisites.md)); subsequent builds reuse the cache and are
fast (~10 s incremental).

## Run

```sh
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
```

A 1280×800 window opens showing a WebGPU triangle. On the console you should
see:

```text
[console] afterglow://local/index.html:11 WebGPU adapter: amd/rdna-2
[console] afterglow://local/index.html:32 rendering WebGPU via afterglow:// scheme
```

If the window opens and the adapter line names your real GPU, your install is
good. The `--ozone-platform=x11` flag is required on CEF 149 (Wayland + Vulkan
are incompatible) — see [Graphics & DevTools](../window/graphics.md).

## What the example is

It's a complete app in ~60 lines — the smallest thing that proves the shell
works:

```rust
use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-cef minimal")
        .size(1280, 800)
        .devtools(9222)
        .index_html(HTML)
        .fs_root("crates/afterglow-web/www")
        .run();
}
```

`HTML` is an embedded `index.html` that renders a WebGPU triangle.
`.index_html(...)` embeds it; `.fs_root(...)` streams everything else from
disk. That's the whole `AppBuilder` API — covered in full in [The AppBuilder
API](../window/app-builder.md).

## Troubleshooting

| Symptom | Fix |
|---|---|
| `libcef.so: cannot open shared object file` | Run through `nix-shell shell.nix`, not the bare binary. |
| GPU process crash / won't start | Pass `--ozone-platform=x11`; don't spawn threads before `.run()`. |
| `WebGPU adapter: ?` or SwiftShader | Vulkan loader wiring — re-source the devshell; see [Debugging](../reference/debugging.md). |
| No window on Wayland | The app runs under XWayland; ensure `DISPLAY=:0` (the devshell sets this). |

## Next

With the install verified, read [The AppBuilder API](../window/app-builder.md)
to learn how to configure your own window, or jump to
[Defining a Service](../workers/defining-a-service.md) to write a worker.
