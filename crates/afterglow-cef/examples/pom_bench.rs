//! POM (parallax occlusion mapping) prototype launcher.
//!
//! Serves the prototype scene from the repo root via the `afterglow://` scheme
//! so the page can load the shared `engine-bundle.js` from `www/` and the
//! authored `prototype/pom/` scene at once. WebGPU-only, X11/XWayland.
//!
//!   cd ~/dev/afterglow-engine
//!   nix-shell shell.nix --run "cargo build --example pom_bench -p afterglow-cef"
//!   nix-shell shell.nix --run "./target/debug/examples/pom_bench --ozone-platform=x11"
//!
//! See prototype/pom/README.md for benchmark + validation commands.

use afterglow_cef::AppBuilder;

fn main() {
    // fs_root = repo root (two levels up from this crate). Covers both
    // `crates/afterglow-web/www/` (engine-bundle.js) and `prototype/pom/`.
    let repo_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    AppBuilder::new()
        .title("afterglow-engine — POM prototype")
        .size(1440, 900)
        .devtools(9222)
        .root("/prototype/pom/pom.html")
        .fs_root(repo_root)
        .run();
}
