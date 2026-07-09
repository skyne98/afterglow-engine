//! afterglow-engine build orchestrator (`cargo run -p xtask <cmd>`).
//!
//! Commands:
//!   build   — build the native CEF host + examples
//!   wasm    — build the web target (afterglow-web) to wasm
//!   check   — cargo check the whole workspace
//!   test    — run all tests
//!   bench   — run the native ring buffer stress test

use std::process::Command;

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: cargo run -p xtask <build|wasm|check|test|bench>");
        std::process::exit(2);
    });
    let r = match cmd.as_str() {
        "build" => sh("cargo", &["build", "--example", "minimal", "-p", "afterglow-cef"]),
        "wasm" => wasm(),
        "check" => sh("cargo", &["check", "--workspace"]),
        "test" => sh("cargo", &["test", "--workspace"]),
        "bench" => sh("cargo", &["run", "--example", "bench_rpc", "-p", "afterglow-rpc-demo"]),
        other => { eprintln!("unknown command: {other}"); 2 }
    };
    std::process::exit(r);
}

fn wasm() -> i32 {
    // Build afterglow-web to wasm with shared memory + atomics.
    // .cargo/config.toml at workspace root applies --import-memory etc.
    let r = sh("cargo", &[
        "build", "-p", "afterglow-web",
        "--target", "wasm32-unknown-unknown",
        "-Zbuild-std=core,alloc,std,panic_abort",
        "--profile", "wasm-dev",
    ]);
    if r != 0 { return r; }
    eprintln!("wasm built -> target/wasm32-unknown-unknown/wasm-dev/afterglow_web.wasm");
    eprintln!("copy to: crates/afterglow-web/www/afterglow_web.wasm");
    0
}

fn sh(program: &str, args: &[&str]) -> i32 {
    let status = Command::new(program).args(args).status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => { eprintln!("failed to run {program}: {e}"); 1 }
    }
}
