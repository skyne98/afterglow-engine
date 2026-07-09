//! Minimal afterglow-cef app: WebGPU triangle + page<->host<->worker RPC
//! round trip + latency/bandwidth benchmark. Assets served via afterglow://.
//!
//!   nix-shell shell.nix --run "cargo build --example minimal"
//!   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"

use afterglow_cef::AppBuilder;
use serde_json::{json, Value};

const HTML: &[u8] = br#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>afterglow-cef</title>
<style>html,body{margin:0;height:100%;background:#0a0c10;color:#e5e7eb;font:13px/1.4 ui-monospace,monospace}
#hud{position:fixed;inset:auto 0 0 0;padding:8px 12px;background:rgba(8,10,14,.85);border-top:1px solid #222;white-space:pre-wrap}
button{font:inherit;color:#0a0c10;background:#7dd3fc;border:0;padding:4px 10px;border-radius:4px;cursor:pointer}
canvas{display:block;width:100vw;height:100vh}</style></head>
<body><canvas id="c"></canvas>
<div id="hud">
<button id="ping">ping worker</button> <span id="r">...</span>
<button id="bench">benchmark</button> <span id="b">...</span>
</div>
<script src="/__afterglow_bootstrap.js"></script>
<script>
const worker = new AfterglowWorker('Physics');
worker.onmessage = (e) => { window.__last_resp = e.data; };

function rpc(service, payload) {
  return new Promise((res) => {
    const orig = worker.onmessage;
    worker.onmessage = (e) => { worker.onmessage = orig; res(e.data); };
    worker.postMessage(service + '\u0000' + payload);
  });
}

document.getElementById('ping').onclick = async () => {
  const resp = await rpc('Physics', JSON.stringify({method:'ping',params:{n:42}}));
  document.getElementById('r').textContent = 'pong: ' + resp;
  console.log('pong: ' + resp);
};

document.getElementById('bench').onclick = async () => {
  const N = 50;
  const sizes = [0, 64, 256, 1024, 4096, 16384, 65536];
  let results = '';
  for (const sz of sizes) {
    const payload = 'x'.repeat(sz);
    const t0 = performance.now();
    for (let i = 0; i < N; i++) {
      await rpc('bench', payload);
    }
    const dt = performance.now() - t0;
    const lat = (dt / N).toFixed(2);
    const tp = sz > 0 ? ((sz * N * 2) / (dt / 1000) / 1024 / 1024).toFixed(1) + ' MB/s' : '-';
    results += sz + 'B: ' + lat + 'ms/op ' + tp + '\n';
  }
  document.getElementById('b').textContent = results;
  console.log('benchmark:\n' + results);
};

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
        .on_rpc(|request: &str| {
            let (service, payload) = request.split_once('\u{0}').unwrap_or((request, ""));
            match service {
                "Physics" => {
                    let parsed: Value = serde_json::from_str(payload).unwrap_or(json!({}));
                    let method = parsed["method"].as_str().unwrap_or("");
                    if method == "ping" {
                        serde_json::to_string(&json!({ "pong": parsed["params"] })).unwrap_or_default()
                    } else {
                        serde_json::to_string(&json!({ "error": "unknown" })).unwrap_or_default()
                    }
                }
                "bench" => payload.to_string(),
                _ => serde_json::to_string(&json!({ "error": "unknown service" })).unwrap_or_default(),
            }
        })
        .on_invoke(|method: &str, params: Value| match method {
            "ping" => json!({ "pong": params }),
            _ => json!({ "error": format!("unknown method: {method}") }),
        })
        .run();
}
