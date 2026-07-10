//! afterglow-engine build orchestrator (`cargo run -p xtask <cmd>`).
//!
//! Commands:
//!   build   — build the native CEF host + examples
//!   wasm    — build afterglow-web + afterglow-rpc-demo to wasm and copy the
//!             artifacts deterministically into `crates/afterglow-web/www/`
//!   check   — cargo check the whole workspace
//!   test    — run all tests
//!   bench   — run the native ring buffer stress test

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: cargo run -p xtask <build|wasm|check|test|bench>");
        std::process::exit(2);
    });
    let r = match cmd.as_str() {
        "build" => sh(
            "cargo",
            &["build", "--example", "minimal", "-p", "afterglow-cef"],
        ),
        "wasm" => wasm(),
        "check" => sh("cargo", &["check", "--workspace"]),
        "test" => sh("cargo", &["test", "--workspace"]),
        "bench" => sh(
            "cargo",
            &[
                "run",
                "--release",
                "--example",
                "bench_rpc",
                "-p",
                "afterglow-rpc-demo",
            ],
        ),
        other => {
            eprintln!("unknown command: {other}");
            2
        }
    };
    std::process::exit(r);
}

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const WASM_PROFILE: &str = "wasm-dev";
const WASM_STD: &str = "-Zbuild-std=core,alloc,std,panic_abort";

/// Build both wasm artifacts and copy them into `crates/afterglow-web/www/`.
///
/// - `afterglow-web` cdylib  -> `www/afterglow_web.wasm`
/// - `afterglow-rpc-demo` cdylib -> `www/physics_worker.wasm`
///
/// Copies are deterministic (byte-identical to the `target/` artifacts), so the
/// checked-in `www/*.wasm` hashes match the build output.
fn wasm() -> i32 {
    for pkg in ["afterglow-web", "afterglow-rpc-demo"] {
        let r = sh(
            "cargo",
            &[
                "build",
                "-p",
                pkg,
                "--target",
                WASM_TARGET,
                WASM_STD,
                "--profile",
                WASM_PROFILE,
            ],
        );
        if r != 0 {
            eprintln!("wasm build failed for {pkg}");
            return r;
        }
    }

    let target_dir = target_dir();
    let src = target_dir.join(WASM_TARGET).join(WASM_PROFILE);
    let www = workspace_root().join("crates/afterglow-web/www");

    let copies: &[(&str, &str)] = &[
        ("afterglow_web.wasm", "afterglow_web.wasm"),
        ("afterglow_rpc_demo.wasm", "physics_worker.wasm"),
    ];
    for (from, to) in copies {
        let from_path = src.join(from);
        let to_path = www.join(to);
        if let Err(e) = std::fs::copy(&from_path, &to_path) {
            eprintln!(
                "failed to copy {} -> {}: {e}",
                from_path.display(),
                to_path.display()
            );
            return 1;
        }
        eprintln!("copied {} -> {}", from_path.display(), to_path.display());
    }
    eprintln!("wasm artifacts updated in {}", www.display());
    0
}

fn workspace_root() -> PathBuf {
    // xtask is a workspace member; CARGO_MANIFEST_DIR is the xtask crate dir.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn target_dir() -> PathBuf {
    if let Ok(t) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(t);
    }
    // Default: workspace-root/target.
    workspace_root().join("target")
}

fn sh(program: &str, args: &[&str]) -> i32 {
    let status = Command::new(program).args(args).status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("failed to run {program}: {e}");
            1
        }
    }
}
