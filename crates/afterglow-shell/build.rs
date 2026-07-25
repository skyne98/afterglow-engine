// Build-time snapshot for deno_core 0.408.
//
// deno_webgpu / deno_web / deno_webidl declare their JS as `lazy_loaded_esm` /
// `lazy_loaded_js`, which deno_core can only resolve from a snapshot (a bare
// snapshotless JsRuntime panics with "Specifier ... was not passed as an
// extension module and was not included in the snapshot").
//
// V8 may only run in ONE mode per process (snapshotting XOR non-snapshotting),
// so the snapshot MUST be produced here, in the build script's separate
// process, then `include_bytes!`-ed into the binary and passed as
// `RuntimeOptions::startup_snapshot`.
//
// This snapshot only embeds the extension JS module sources (the extensions
// have no eager `js` / `esm_entry_point`, so nothing GPU-related executes at
// build time). The actual `navigator.gpu` setup + `requestAdapter` happen at
// runtime via runtime bootstrap.

use std::sync::Arc;

use deno_core::PollEventLoopOptions;
use deno_core::snapshot::{CreateSnapshotOptions, create_snapshot};
use deno_web::BlobStore;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=dom_setup.ts");
    println!("cargo:rerun-if-changed=canvas_2d.ts");
    println!("cargo:rerun-if-changed=raf.ts");
    println!("cargo:rerun-if-changed=scheduler.ts");

    let output = create_snapshot(
        CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: vec![
                deno_webidl::deno_webidl::init(),
                deno_web::deno_web::init(
                    Arc::new(BlobStore::default()),
                    Some(url::Url::parse("https://example.com/").unwrap()),
                    false,
                    deno_web::InMemoryBroadcastChannel::default(),
                ),
                deno_webgpu::deno_webgpu::init(),
            ],
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(|runtime| {
                // Force-load the lazy `ext:` scripts/modules so their sources
                // get consumed and externalized into the snapshot's residual
                // table (otherwise they're `LoadedFromFsDuringSnapshot` paths
                // that only exist on the build machine and are skipped at
                // runtime -> "cannot be lazy-loaded as it was not included in
                // the binary"). We load the webgpu cascade (which pulls in
                // deno_webidl/00_webidl.js + deno_web/02_event.js) but do NOT
                // call initGPU (no GPU at build time; created per-runtime).
                runtime
                    .execute_script(
                        "<snapshot_warmup>",
                        r#"
                        (() => {
                          const core = globalThis.Deno.core;
                          try {
                            // Cascade-load the webgpu lazy chain so the sources
                            // are consumed + externalized into the snapshot:
                            //   00_init.js -> loadWebGPU -> 01_webgpu.js
                            //     -> loadExtScript(00_webidl.js, 02_event.js)
                            // createLazyLoader returns the module namespace
                            // synchronously (no TLA in 01_webgpu.js).
                            const { loadWebGPU } = core.loadExtScript("ext:deno_webgpu/00_init.js");
                            const webgpu = loadWebGPU();
                            // Browser globals used before WebGPU initialization. Loading
                            // them here embeds deno_web's lazy scripts in the snapshot.
                            core.loadExtScript("ext:deno_web/00_url.js");
                            core.loadExtScript("ext:deno_web/01_dom_exception.js");
                            core.loadExtScript("ext:deno_web/02_structured_clone.js");
                            core.loadExtScript("ext:deno_web/02_timers.js");
                            core.loadExtScript("ext:deno_web/03_abort_signal.js");
                            core.loadExtScript("ext:deno_web/08_text_encoding.js");
                            core.loadExtScript("ext:deno_web/09_file.js");
                            core.loadExtScript("ext:deno_web/12_location.js");
                            core.loadExtScript("ext:deno_web/15_performance.js");
                            core.print("[snapshot_warmup] webgpu keys: " + Object.keys(webgpu).length + "\n");
                          } catch (e) {
                            core.print("[snapshot_warmup] ERR: " + (e && e.stack || e) + "\n");
                          }
                        })();
                        "#,
                    )
                    .expect("execute snapshot warmup");
                pollster::block_on(runtime.run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                }))
                .expect("run event loop during snapshot warmup");
            })),
        },
        None,
    )
    .expect("failed to create deno_core snapshot");

    for f in &output.files_loaded_during_snapshot {
        println!("cargo:rerun-if-changed={}", f.display());
    }

    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("SNAPSHOT.bin");
    std::fs::write(&path, &output.output).expect("failed to write snapshot");
    println!(
        "cargo:warning=deno_core snapshot written to {} ({} bytes, {} lazy specifiers consumed)",
        path.display(),
        output.output.len(),
        output.consumed_lazy_specifiers.len()
    );
}
