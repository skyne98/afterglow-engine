//! First-person dungeon combining three scanned 8K virtual PBR materials,
//! distance-faded low-core POM, persistent cache, and raw pointer input.
//!
//! nix-shell shell.nix --run "cargo build --example dungeon -p afterglow-cef"
//! nix-shell shell.nix --run "./target/debug/examples/dungeon --ozone-platform=x11"

use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-engine — Dungeon")
        .size(1440, 900)
        .devtools(9222)
        .index_html(b"<script>location.href='/dungeon.html'</script>")
        .fs_root("crates/afterglow-web/www")
        .run();
}
