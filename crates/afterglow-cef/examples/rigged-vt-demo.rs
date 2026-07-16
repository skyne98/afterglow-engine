//! Animated SkinnedMesh using a pipeline-packed GLB, runtime meshopt index
//! optimization, and linked glTF PBR virtual textures.
//!
//! nix-shell shell.nix --run "cargo build --example rigged-vt-demo -p afterglow-cef"
//! nix-shell shell.nix --run "./target/debug/examples/rigged-vt-demo --ozone-platform=x11"

use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-engine — Rigged Virtual Texture")
        .size(1440, 900)
        .devtools(9222)
        .index_html(b"<script>location.href='/rigged-vt-demo.html'</script>")
        .fs_root("crates/afterglow-web/www")
        .run();
}
