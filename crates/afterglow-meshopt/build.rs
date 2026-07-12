// build.rs — compiles the vendored meshoptimizer C++ source.
//
// For native targets: uses the `cc` crate (handles system compiler discovery).
// For WASM targets: invokes clang++ directly via std::process::Command,
//   bypassing NixOS's cc-wrapper (which adds x86-only flags that break WASM).
//
// meshoptimizer is freestanding (no C++ stdlib, no exceptions, no RTTI).
// We provide minimal header stubs in vendor/wasm-headers/ for assert.h,
// math.h, string.h, stdio.h.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let sources = [
        "allocator.cpp",
        "clusterizer.cpp",
        "indexanalyzer.cpp",
        "indexcodec.cpp",
        "indexgenerator.cpp",
        "meshletcodec.cpp",
        "meshletutils.cpp",
        "overdrawoptimizer.cpp",
        "partition.cpp",
        "quantization.cpp",
        "rasterizer.cpp",
        "simplifier.cpp",
        "spatialorder.cpp",
        "stripifier.cpp",
        "vcacheoptimizer.cpp",
        "vertexcodec.cpp",
        "vertexfilter.cpp",
        "vfetchoptimizer.cpp",
        "wasm_stubs.cpp",
    ];

    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    if target.contains("wasm32") {
        build_wasm(&sources, &out_dir, &manifest_dir);
    } else {
        build_native(&sources, &out_dir, &manifest_dir);
    }

    println!("cargo:rerun-if-changed=vendor/src/meshoptimizer.h");
    for src in &sources {
        println!("cargo:rerun-if-changed=vendor/src/{src}");
    }
}

fn build_native(sources: &[&str], _out_dir: &Path, _manifest_dir: &Path) {
    // NOTE: wasm_stubs.cpp is NOT included here — it conflicts with glibc.
    let native_sources: Vec<&str> = sources.iter().filter(|s| **s != "wasm_stubs.cpp").copied().collect();
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++11")
        .include("vendor/src")
        .warnings(false);

    for src in &native_sources {
        build.file(&format!("vendor/src/{src}"));
    }
    build.compile("meshoptimizer");
}

fn build_wasm(sources: &[&str], out_dir: &Path, manifest_dir: &Path) {
    // Find an unwrapped clang++ — NixOS's cc-wrapper adds x86-only flags
    // (-fzero-call-used-regs=used-gpr) that break WASM compilation.
    let clangxx = find_unwrapped_clang();

    let mut objs: Vec<PathBuf> = Vec::new();
    for src in sources {
        let obj = out_dir.join(format!("meshopt-{src}.o"));
        let status = Command::new(&clangxx)
            .args([
                "--target=wasm32-unknown-unknown",
                "-ffreestanding",
                "-fno-exceptions",
                "-fno-rtti",
                "-O2",
                "-DNDEBUG",
                "-std=c++11",
                "-I", &manifest_dir.join("vendor/src").to_string_lossy(),
                "-I", &manifest_dir.join("vendor/wasm-headers").to_string_lossy(),
                "-c", &manifest_dir.join("vendor/src").join(src).to_string_lossy(),
                "-o", &obj.to_string_lossy(),
            ])
            .status()
            .unwrap_or_else(|e| panic!("clang++ failed for {src}: {e}"));
        assert!(status.success(), "clang++ failed for {src}");
        objs.push(obj);
    }

    // Archive into a static library.
    let lib = out_dir.join("libmeshoptimizer.a");
    let ar = env::var("AR_wasm32-unknown-unknown")
        .or_else(|_| env::var("AR"))
        .unwrap_or_else(|_| "llvm-ar".to_string());
    let mut cmd = Command::new(&ar);
    cmd.arg("rcs").arg(&lib);
    for obj in &objs {
        cmd.arg(obj);
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("{ar} failed: {e}"));
    assert!(status.success(), "{ar} failed");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=meshoptimizer");
}

/// Find an unwrapped clang++ binary.
///
/// On NixOS, `clang++` on PATH is a cc-wrapper that adds x86-only flags.
/// The unwrapped binary is in a parallel store path with "clang-wrapper"
/// replaced by "clang".
fn find_unwrapped_clang() -> String {
    // 1. Check CC_wasm32-unknown-unknown env var.
    if let Ok(cc) = env::var("CC_wasm32-unknown-unknown") {
        return cc;
    }

    // 2. Find the wrapper, then read its nix-support/orig-cc (NixOS pattern).
    if let Some(path) = which("clang++") {
        if let Ok(resolved) = std::fs::canonicalize(&path) {
            // The wrapper is at .../clang-wrapper-VERSION/bin/clang++
            // nix-support/orig-cc contains the path to the unwrapped prefix.
            if let Some(wrapper_root) = resolved.parent().and_then(|p| p.parent()) {
                let orig_cc_file = wrapper_root.join("nix-support/orig-cc");
                if let Ok(prefix) = std::fs::read_to_string(&orig_cc_file) {
                    let unwrapped = format!("{}/bin/clang++", prefix.trim());
                    if Path::new(&unwrapped).exists() {
                        return unwrapped;
                    }
                }
            }
            // Fallback: replace "clang-wrapper" with "clang" (may not work due to hash diff).
            let unwrapped = resolved.to_string_lossy().replace("clang-wrapper", "clang");
            if Path::new(&unwrapped).exists() {
                return unwrapped;
            }
        }
    }

    // 3. Fallback: just use clang++ from PATH.
    "clang++".to_string()
}

fn which(bin: &str) -> Option<String> {
    let path = env::var("PATH").ok()?;
    for dir in path.split(':') {
        let full = format!("{dir}/{bin}");
        if std::path::Path::new(&full).exists() {
            return Some(full);
        }
    }
    None
}
