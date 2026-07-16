# Native (CEF)

Build and run the native shell. All commands go through the devshell (see
[Prerequisites](../setup/prerequisites.md)).

## Build the example

```sh
nix-shell shell.nix --run "cargo build --example minimal -p afterglow-cef"
```

The first build fetches the CEF distribution (~hundreds of MB) into
`target/debug/`; subsequent builds reuse it (the devshell pins `CEF_PATH` there).

## Run

```sh
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
```

`--ozone-platform=x11` is required on CEF 149 (Wayland + Vulkan are
incompatible) — see [Graphics & DevTools](../window/graphics.md).

### Verify real WebGPU

Use the devshell unchanged. It selects a coherent Vulkan loader/ICD rather than
CEF's bundled SwiftShader; on non-NixOS hosts it defaults to Nix Mesa. This is
required on fox-laptop (Radeon 680M), where host Mesa 26.1.4 crashes CEF's GPU
process and silently makes Three.js fall back to WebGL2.

With the example open (DevTools port 9222), verify the real adapter from another
terminal:

```sh
./target/release/latency-tool eval \
  '(async()=>{const a=await navigator.gpu.requestAdapter();return JSON.stringify(a&&a.info)})()' \
  127.0.0.1:9222
```

It must report `amd` / `rdna-2` on fox-laptop. Also reject a run if its CEF log
contains `GPU process exited` or `WebGPU is not available`: visible simple
meshes alone can be the WebGL2 fallback. The complete launch and validation
procedure is in [AGENTS.md](../../AGENTS.md#fox-laptop-radeon-680m-cefwebgpu-validation).

## Build your own app

Add `afterglow-cef` as a dependency and write a `main` that calls `AppBuilder`:

```toml
[dependencies]
afterglow-cef = { path = "../afterglow-cef" }
```

```rust
use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("my game")
        .size(1920, 1080)
        .index_html(include_bytes!("index.html"))
        .fs_root("assets")
        .run();
}
```

See [The AppBuilder API](../window/app-builder.md) for the full builder, and
[Building a Game Window](../guides/game-window.md) for an end-to-end walkthrough.

## The `xtask` orchestrator

`cargo run -p xtask <cmd>` wraps the common build tasks:

```sh
cargo run -p xtask build   # build the native CEF host + examples
cargo run -p xtask wasm    # build wasm artifacts (see Web (Wasm))
cargo run -p xtask check   # cargo check the whole workspace
cargo run -p xtask test    # cargo test --workspace + node --test
cargo run -p xtask bench   # run the native ring buffer stress test
```

`xtask build` runs the same `cargo build --example minimal -p afterglow-cef`
as above.

## Release builds

The workspace `[profile.release]` uses `opt-level = 2`. For a release binary:

```sh
nix-shell shell.nix --run "cargo build --release --example minimal -p afterglow-cef"
nix-shell shell.nix --run "./target/release/examples/minimal --ozone-platform=x11"
```

## Next

- [Web (Wasm)](./web.md) — build the same worker code for the browser.
- [Building a Game Window](../guides/game-window.md) — end-to-end.
