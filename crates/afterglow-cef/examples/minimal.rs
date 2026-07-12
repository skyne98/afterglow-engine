//! Minimal afterglow-cef app: bitECS + Three.js + WebGPU.
//!
//! The CEF shell provides a window with WebGPU + COOP/COEP headers.
//! The page (`engine-demo.html`) is served from the filesystem via the
//! `afterglow://` scheme — it loads bitECS + Three.js + the render adapter
//! and renders 10,000 instanced cubes.
//!
//!   nix-shell shell.nix --run "cargo build --example minimal"
//!   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"

use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-engine — bitECS + Three.js")
        .size(1440, 900)
        .devtools(9222)
        .index_html(b"<script>location.href='/engine-demo.html'</script>")
        .fs_root("crates/afterglow-web/www")
        .run();
}
