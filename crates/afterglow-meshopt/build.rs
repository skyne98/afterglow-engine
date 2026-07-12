// build.rs — compiles the vendored meshoptimizer C++ source via cc.
//
// meshoptimizer is designed for embedded (no stdlib, custom allocators,
// bounded stack usage ≤32KB) — compiles to both native and WASM.

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
    ];

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++11")
        .include("vendor/src")
        .warnings(false);

    for src in &sources {
        build.file(&format!("vendor/src/{src}"));
    }

    build.compile("meshoptimizer");

    // Tell cargo to rerun if the source changes.
    println!("cargo:rerun-if-changed=vendor/src/meshoptimizer.h");
    for src in &sources {
        println!("cargo:rerun-if-changed=vendor/src/{src}");
    }
}
