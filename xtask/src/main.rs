//! afterglow-engine build orchestrator (`cargo run -p xtask <cmd>`).
//!
//! Commands:
//!   build   — build the native CEF host + examples
//!   wasm    — build the engine core to WASM (web worker)
//!   dist    — assemble the distributable (native binary + CEF + assets)
//!   check   — cargo check the whole workspace
//!
//! Everything is Rust-driven (no bun/node shell glue): this is the single
//! entry point for building the engine for both targets.

use std::process::Command;

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: cargo run -p xtask <build|wasm|dist|check>");
        std::process::exit(2);
    });
    let r = match cmd.as_str() {
        "build" => build(),
        "wasm" => wasm(),
        "dist" => dist(),
        "check" => sh("cargo", &["check", "--workspace"]),
        other => { eprintln!("unknown command: {other}"); 2 }
    };
    std::process::exit(r);
}

fn build() -> i32 {
    sh("cargo", &["build", "--example", "minimal"])
}

fn wasm() -> i32 {
    // Build the engine core to a WASM cdylib for the web worker.
    // Requires the wasm32-unknown-unknown target + wasm-bindgen-cli.
    let r = sh("cargo", &["build", "-p", "afterglow-engine-core",
        "--target", "wasm32-unknown-unknown", "--release"]);
    if r != 0 { return r; }
    eprintln!("WASM core built -> target/wasm32-unknown-unknown/release/");
    eprintln!("next: wasm-bindgen --target web --out-dir <assets> <.wasm>");
    0
}

fn dist() -> i32 {
    // Assemble the native distributable next to the CEF runtime.
    let r = sh("cargo", &["build", "--release", "--example", "minimal"]);
    if r != 0 { return r; }
    eprintln!("dist: TODO copy target/release/examples/minimal + CEF runtime + assets -> dist/");
    0
}

fn sh(program: &str, args: &[&str]) -> i32 {
    let status = Command::new(program).args(args).status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => { eprintln!("failed to run {program}: {e}"); 1 }
    }
}
