use std::path::PathBuf;

fn main() {
    if std::env::var_os("CARGO_FEATURE_STEAM_AUDIO").is_none() {
        return;
    }
    let target = std::env::var("TARGET").expect("Cargo sets TARGET");
    if target.contains("emscripten") {
        // The pinned Emscripten build links Steam Audio and this gate shim in
        // prototype/steam-audio-wasm/build.sh.
        return;
    }

    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..");
    let sdk = std::env::var_os("STEAMAUDIO_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/steam-audio-native-build/sdk"));
    let source = root.join("prototype/steam-audio-wasm/dynamic-benchmark.cpp");
    let tracer_include = root.join("crates/afterglow-obvhs-tracer/include");
    if !sdk.join("phonon.h").is_file() || !sdk.join("libphonon.so").is_file() {
        panic!(
            "native Steam Audio SDK missing at {}; run prototype/steam-audio-wasm/build-native.sh",
            sdk.display()
        );
    }

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-env-changed=STEAMAUDIO_SDK");
    println!("cargo:rustc-link-search=native={}", sdk.display());
    println!("cargo:rustc-link-lib=dylib=phonon");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", sdk.display());
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .opt_level(3)
        .flag_if_supported("-march=x86-64-v3")
        .include(&sdk)
        .include(tracer_include)
        .file(source)
        .compile("afterglow_steam_audio_native_gate");
}
