//! Minimal afterglow-cef app: WebGPU triangle + SharedArrayBuffer ring buffer.
//!
//! The CEF shell just provides a window with WebGPU + COOP/COEP headers.
//! Workers and ring buffers use `afterglow-web` (SharedArrayBuffer) — the same
//! mechanism as the web target.
//!
//!   nix-shell shell.nix --run "cargo build --example minimal"
//!   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"

use afterglow_cef::AppBuilder;

const HTML: &[u8] = br#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>afterglow-cef</title>
<style>html,body{margin:0;height:100%;background:#0a0c10;color:#e5e7eb;font:13px/1.4 ui-monospace,monospace}
canvas{display:block;width:100vw;height:100vh}</style></head>
<body><canvas id="c"></canvas>
<script>
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

// SAB ring buffer stress test page + wasm.
const WORKER_TEST_HTML: &[u8] = include_bytes!("../../afterglow-web/www/worker-test.html");
const RPC_JS: &[u8] = include_bytes!("../../afterglow-web/www/rpc.js");
const WORKER_JS: &[u8] = include_bytes!("../../afterglow-web/www/worker.js");
const WORKER_BENCH_HTML: &[u8] = include_bytes!("../../afterglow-web/www/worker-bench.html");
const PHYSICS_WORKER_WASM: &[u8] = include_bytes!("../../afterglow-web/www/physics_worker.wasm");
const BENCH_HTML: &[u8] = include_bytes!("../../afterglow-web/www/bench.html");
const BENCH_WASM: &[u8] = include_bytes!("../../afterglow-web/www/afterglow_web.wasm");

fn main() {
    AppBuilder::new()
        .title("afterglow-cef minimal")
        .size(1280, 800)
        .devtools(9222)
        .root("/index.html")
        .asset("/index.html", "text/html", HTML)
        .asset("/worker-test.html", "text/html", WORKER_TEST_HTML)
        .asset("/rpc.js", "text/javascript", RPC_JS)
        .asset("/worker.js", "application/javascript", WORKER_JS)
        .asset("/worker-bench.html", "text/html", WORKER_BENCH_HTML)
        .asset(
            "/physics_worker.wasm",
            "application/wasm",
            PHYSICS_WORKER_WASM,
        )
        .asset("/bench.html", "text/html", BENCH_HTML)
        .asset("/afterglow_web.wasm", "application/wasm", BENCH_WASM)
        .run();
}
