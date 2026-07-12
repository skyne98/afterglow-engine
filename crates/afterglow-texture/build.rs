// build.rs — compiles the Basis Universal transcoder (single .cpp, no deps).
//
// The transcoder is a single file (basisu_transcoder.cpp) with no third-party
// dependencies. It transcodes Basis Universal compressed textures to GPU-native
// formats (BC1-7, ASTC, ETC2, PVRTC) at load time.
//
// For WASM: uses clang++ with freestanding stubs (same approach as meshopt).
// For native: uses cc crate with the system compiler.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    if target.contains("wasm32") {
        build_wasm(&out_dir, &manifest_dir);
    } else {
        build_native(&out_dir, &manifest_dir);
    }

    println!("cargo:rerun-if-changed=vendor/transcoder/basisu_transcoder.cpp");
}

fn build_native(_out_dir: &Path, manifest_dir: &Path) {
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(manifest_dir.join("vendor/transcoder"))
        .include(manifest_dir.join("vendor")) // for ../zstd/zstd.h
        .warnings(false)
        .file(manifest_dir.join("vendor/transcoder/basisu_transcoder.cpp"))
        .file(manifest_dir.join("vendor/transcoder/wrapper.cpp"))
        .file(manifest_dir.join("vendor/zstd/zstddeclib.c"))
        .compile("basisu_transcoder");
}

fn build_wasm(out_dir: &Path, manifest_dir: &Path) {
    // Find unwrapped clang++ (same as meshopt — NixOS wrapper adds x86 flags).
    let clangxx = find_unwrapped_clang();

    let src = manifest_dir.join("vendor/transcoder/basisu_transcoder.cpp");
    let obj = out_dir.join("basisu_transcoder.o");

    let status = Command::new(&clangxx)
        .args([
            "--target=wasm32-unknown-unknown",
            "-ffreestanding",
            "-fno-exceptions",
            "-fno-rtti",
            "-O2",
            "-DNDEBUG",
            "-std=c++17",
            "-I", &manifest_dir.join("vendor/transcoder").to_string_lossy(),
            "-I", &manifest_dir.join("vendor").to_string_lossy(),
            "-isystem", &manifest_dir.join("vendor/wasm-headers").to_string_lossy(),
            "-I", "/nix/store/0axpqqmj1rcanmm2i8vzpd8xxwkmq2jf-gcc-15.2.0/include/c++/15.2.0",
            "-I", "/nix/store/0axpqqmj1rcanmm2i8vzpd8xxwkmq2jf-gcc-15.2.0/include/c++/15.2.0/x86_64-unknown-linux-gnu",
            "-c", &src.to_string_lossy(),
            "-o", &obj.to_string_lossy(),
        ])
        .status()
        .unwrap_or_else(|e| panic!("clang++ failed: {e}"));
    assert!(status.success(), "clang++ failed for basisu_transcoder.cpp");

    // Compile wrapper.cpp
    let wrapper_src = manifest_dir.join("vendor/transcoder/wrapper.cpp");
    let wrapper_obj = out_dir.join("wrapper.o");
    let status = Command::new(&clangxx)
        .args([
            "--target=wasm32-unknown-unknown",
            "-ffreestanding",
            "-fno-exceptions",
            "-fno-rtti",
            "-O2",
            "-DNDEBUG",
            "-std=c++17",
            "-I", &manifest_dir.join("vendor/transcoder").to_string_lossy(),
            "-I", &manifest_dir.join("vendor").to_string_lossy(),
            "-isystem", &manifest_dir.join("vendor/wasm-headers").to_string_lossy(),
            "-I", "/nix/store/0axpqqmj1rcanmm2i8vzpd8xxwkmq2jf-gcc-15.2.0/include/c++/15.2.0",
            "-I", "/nix/store/0axpqqmj1rcanmm2i8vzpd8xxwkmq2jf-gcc-15.2.0/include/c++/15.2.0/x86_64-unknown-linux-gnu",
            "-c", &wrapper_src.to_string_lossy(),
            "-o", &wrapper_obj.to_string_lossy(),
        ])
        .status()
        .unwrap_or_else(|e| panic!("clang++ failed: {e}"));
    assert!(status.success(), "clang++ failed for wrapper.cpp");

    // Archive.
    let lib = out_dir.join("libbasisu_transcoder.a");
    let ar = env::var("AR_wasm32-unknown-unknown")
        .or_else(|_| env::var("AR"))
        .unwrap_or_else(|_| "llvm-ar".to_string());
    let status = Command::new(&ar)
        .args(["rcs", &lib.to_string_lossy(), &obj.to_string_lossy(), &wrapper_obj.to_string_lossy()])
        .status()
        .unwrap_or_else(|e| panic!("{ar} failed: {e}"));
    assert!(status.success());

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=basisu_transcoder");
}

fn find_unwrapped_clang() -> String {
    if let Ok(cc) = env::var("CC_wasm32-unknown-unknown") {
        return cc;
    }
    if let Some(path) = which("clang++") {
        if let Ok(resolved) = std::fs::canonicalize(&path) {
            if let Some(wrapper_root) = resolved.parent().and_then(|p| p.parent()) {
                let orig_cc_file = wrapper_root.join("nix-support/orig-cc");
                if let Ok(prefix) = std::fs::read_to_string(&orig_cc_file) {
                    let unwrapped = format!("{}/bin/clang++", prefix.trim());
                    if Path::new(&unwrapped).exists() {
                        return unwrapped;
                    }
                }
            }
        }
    }
    "clang++".to_string()
}

fn which(bin: &str) -> Option<String> {
    let path = env::var("PATH").ok()?;
    for dir in path.split(':') {
        let full = format!("{dir}/{bin}");
        if Path::new(&full).exists() {
            return Some(full);
        }
    }
    None
}
