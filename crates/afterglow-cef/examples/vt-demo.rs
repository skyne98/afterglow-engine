//! VT demo: universal virtual texturing with WGSL page table lookup + atlas.
//!
//!   nix-shell shell.nix --run "cargo build --example vt-demo"
//!   nix-shell shell.nix --run "./target/debug/examples/vt-demo --ozone-platform=x11"

use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-engine — Virtual Texturing")
        .size(1440, 900)
        .devtools(9222)
        .index_html(b"<script>location.href='/vt-demo.html'</script>")
        .fs_root("crates/afterglow-web/www")
        .run();
}
