//! Minimal afterglow-cef app: WebGPU triangle + ring-buffer RPC benchmark.
//!
//!   nix-shell shell.nix --run "cargo build --example minimal"
//!   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"

use afterglow_cef::AppBuilder;
use afterglow_rpc_demo::{spawn_worker, PhysicsWorker};
use afterglow_rpc::Transport;

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

fn main() {
    // Spawn a Physics worker (native thread + shared-memory ring buffer).
    let (client, _events) = spawn_worker(PhysicsWorker);

    // Benchmark: RPC over the ring buffer (no IPC, no postMessage).
    let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
    assert_eq!(next, vec![0.5, 1.5, 2.5]);

    let accepted = client.apply_force(3, 0.0, 9.8, 0.0).unwrap();
    assert!(accepted);

    // Latency + bandwidth benchmark
    let n = 1000;
    for size in [0usize, 64, 256, 1024, 4096, 16384, 65536] {
        let state: Vec<f32> = (0..size).map(|i| i as f32).collect();
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let _ = client.step(state.clone(), 0.0).unwrap();
        }
        let dt = t0.elapsed();
        let lat_us = dt.as_micros() as f64 / n as f64;
        let lat_ms = lat_us / 1000.0;
        let bytes = size * 4; // Vec<f32> → bytes
        let throughput = if bytes > 0 {
            (bytes as f64 * n as f64 * 2.0) / dt.as_secs_f64() / 1024.0 / 1024.0
        } else { 0.0 };
        eprintln!(
            "{:6} B: {:7.3} ms/op  {:7.1} MB/s",
            bytes, lat_ms, throughput
        );
    }

    AppBuilder::new()
        .title("afterglow-cef minimal")
        .size(1280, 800)
        .devtools(9222)
        .root("/index.html")
        .asset("/index.html", "text/html", HTML)
        .run();
}
