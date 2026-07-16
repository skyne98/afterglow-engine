//! First-person dungeon using the engine VirtualTextureStore for twelve unique
//! 128K procedural wall textures sharing one physical atlas.
//!
//! nix-shell shell.nix --run "cargo build --example vt-dungeon -p afterglow-cef"
//! nix-shell shell.nix --run "./target/debug/examples/vt-dungeon --ozone-platform=x11"

use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-engine — VT Dungeon")
        .size(1440, 900)
        .devtools(9222)
        .index_html(b"<script>location.href='/vt-dungeon.html'</script>")
        .fs_root("crates/afterglow-web/www")
        .run();
}
