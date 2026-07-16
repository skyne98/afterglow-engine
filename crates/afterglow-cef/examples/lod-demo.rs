//! LOD demo: loads a sphere mesh, generates 4 LOD levels via the meshopt
//! worker, and displays them in a 2×2 grid.
//!
//!   nix-shell shell.nix --run "cargo build --example lod-demo"
//!   nix-shell shell.nix --run "./target/debug/examples/lod-demo --ozone-platform=x11"

use afterglow_cef::AppBuilder;

fn main() {
    AppBuilder::new()
        .title("afterglow-engine — LOD Demo")
        .size(1440, 900)
        .devtools(9222)
        .root("/lod-demo.html")
        .fs_root("crates/afterglow-web/www")
        .run();
}
