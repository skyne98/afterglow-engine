//! Minimal afterglow-cef app: WebGPU triangle + cross-process shared memory
//! data push via CefSharedMemoryRegion.
//!
//! The browser process creates a shared memory message each "frame" and
//! sends it to the renderer. The renderer copies it into a V8 ArrayBuffer
//! (one memcpy — V8 sandbox blocks external ArrayBuffers in CEF 149) and
//! exposes it as `window.__afterglow_frame_data`. JS reads it and logs the
//! size + first bytes.
//!
//!   nix-shell shell.nix --run "cargo build --example minimal"
//!   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"

use afterglow_cef::{push_frame_data, AppBuilder, MAIN_BROWSER};
use cef::{ImplBrowser, ImplFrame};
use std::time::Duration;

// Web target test page: verifies SharedArrayBuffer + ring buffer works
// via afterglow:// scheme with COOP/COEP headers.
const WEB_TEST_HTML: &[u8] = include_bytes!("../../afterglow-web/www/index.html");
const WEB_TEST_WASM: &[u8] = include_bytes!("../../afterglow-web/www/afterglow_web.wasm");
const WEB_TEST_WORKER: &[u8] = include_bytes!("../../afterglow-web/www/worker.js");

const HTML: &[u8] = br#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>afterglow-cef</title>
<style>html,body{margin:0;height:100%;background:#0a0c10;color:#e5e7eb;font:13px/1.4 ui-monospace,monospace}
canvas{display:block;width:100vw;height:100vh}</style></head>
<body><canvas id="c"></canvas>
<script>
// Per-frame data callback: called when the browser pushes new data via
// shared memory. The data is in window.__afterglow_frame_data (ArrayBuffer).
window.__afterglow_on_frame_data = function() {
  var ab = window.__afterglow_frame_data;
  if (!ab) return;
  var u8 = new Uint8Array(ab);
  // Log size + first 4 bytes (should be 0xDE 0xAD 0xBE 0xEF)
  if (!window.__frame_count) window.__frame_count = 0;
  window.__frame_count++;
  if (window.__frame_count <= 3 || window.__frame_count % 60 === 0)
    console.log('frame_data: ' + ab.byteLength + ' bytes, first=' +
      u8[0].toString(16) + u8[1].toString(16) + u8[2].toString(16) + u8[3].toString(16) +
      ' count=' + window.__frame_count);
};

// Wait for the persistent ring buffer (sent once at startup).
function waitForBuffer() {
  if (window.__afterglow_buffer) {
    var dv = new DataView(window.__afterglow_buffer);
    console.log('ring buffer: ' + window.__afterglow_buffer.byteLength + ' bytes, cap=' + dv.getUint32(0, true));
  } else {
    setTimeout(waitForBuffer, 100);
  }
}
waitForBuffer();

(async () => {
  if (!navigator.gpu) { console.log('no WebGPU'); return; }
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
  const device = await adapter.requestDevice();
  console.log('WebGPU adapter: ' + (adapter.info?.vendor || '?') + '/' + (adapter.info?.architecture || '?'));
  const canvas = document.getElementById('c');
  const ctx = canvas.getContext('webgpu');
  ctx.configure({ device, format: navigator.gpu.getPreferredCanvasFormat(), alphaMode: 'opaque' });
  const mod = device.createShaderModule({ code: `
    @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
      var p = array<vec2<f32>,3>(vec2(0.,.7),vec2(-.7,-.5),vec2(.7,-.5)); return vec4<f32>(p[i],0.,1.); }
    @fragment fn fs(@builtin(position) f: vec4<f32>) -> @location(0) vec4<f32> {
      let t = f.xyz * .5 + .5; return vec4<f32>(t,1.); }`});
  const pipe = device.createRenderPipeline({ layout:'auto', vertex:{module:mod,entryPoint:'vs'},
    fragment:{module:mod,entryPoint:'fs',targets:[{format:ctx.getConfiguration().format}]}, primitive:{topology:'triangle-list'} });
  function frame() {
    if (canvas.width !== canvas.clientWidth) canvas.width = canvas.clientWidth;
    if (canvas.height !== canvas.clientHeight) canvas.height = canvas.clientHeight;
    const enc = device.createCommandEncoder();
    const pass = enc.beginRenderPass({ colorAttachments:[{ view: ctx.getCurrentTexture().createView(),
      clearValue:{r:.04,g:.05,b:.06,a:1}, loadOp:'clear', storeOp:'store' }] });
    pass.setPipeline(pipe); pass.draw(3); pass.end(); device.queue.submit([enc.finish()]);
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  console.log('rendering WebGPU via afterglow:// scheme');
})().catch(e => console.log('ERR ' + (e?.stack||e)));
</script></body></html>"#;

fn main() {
    AppBuilder::new()
        .title("afterglow-cef minimal")
        .size(1280, 800)
        .devtools(9222)
        .root("/index.html")
        .asset("/index.html", "text/html", HTML)
        .asset("/web-test.html", "text/html", WEB_TEST_HTML)
        .asset("/afterglow_web.wasm", "application/wasm", WEB_TEST_WASM)
        .asset("/worker.js", "application/javascript", WEB_TEST_WORKER)
        .on_ready(|| {
            // Spawn a thread that pushes fake "physics data" to the renderer
            // every 16ms (60 FPS). Must be spawned after CEF init (spawning
            // threads before execute_process crashes the GPU process).
            std::thread::spawn(|| {
                // Benchmark: measure push_frame_data latency at various sizes.
                let browser = MAIN_BROWSER.lock().unwrap().clone().unwrap();
                let frame = browser.main_frame().unwrap();
                eprintln!("[afterglow] benchmarking push_frame_data latency...");
                for size in [64, 256, 1024, 4096, 16384, 65536, 262144, 1048576] {
                    let data = vec![0xAAu8; size];
                    let n = 200;
                    let t0 = std::time::Instant::now();
                    for _ in 0..n {
                        push_frame_data(&frame, &data);
                    }
                    let dt = t0.elapsed();
                    let lat_us = dt.as_micros() as f64 / n as f64;
                    eprintln!(
                        "  {:7} B: {:7.1} µs/push  {:7.1} MB/s",
                        size,
                        lat_us,
                        (size as f64 * n as f64) / dt.as_secs_f64() / 1024.0 / 1024.0
                    );
                }

                // Then push 600 frames at 60 FPS to demonstrate live data.
                let frame_count = 600u32;
                for i in 0..frame_count {
                    let browser = MAIN_BROWSER.lock().unwrap().clone();
                    if let Some(browser) = browser {
                        if let Some(frame) = browser.main_frame() {
                            let mut data = vec![0u8; 64];
                            data[0] = 0xDE;
                            data[1] = 0xAD;
                            data[2] = 0xBE;
                            data[3] = 0xEF;
                            data[4] = (i & 0xFF) as u8;
                            push_frame_data(&frame, &data);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }
                eprintln!("[afterglow] push thread finished ({frame_count} frames)");
            });
        })
        .run();
}
