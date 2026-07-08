//! Minimal afterglow-cef app: a WebGPU triangle + a JS<->Rust round-trip
//! (invoke web->rust, and an emitted event rust->web), assets served directly
//! via the `afterglow://` scheme.
//!
//!   nix-shell shell.nix --run "cargo build --example minimal"
//!   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"

use afterglow_cef::AppBuilder;
use serde_json::{json, Value};

const HTML: &[u8] = br#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>afterglow-cef minimal</title>
<style>html,body{margin:0;height:100%;background:#0a0c10;color:#e5e7eb;font:13px/1.4 ui-monospace,monospace}
#hud{position:fixed;inset:auto 0 0 0;padding:8px 12px;background:rgba(8,10,14,.85);border-top:1px solid #222}
button{font:inherit;color:#0a0c10;background:#7dd3fc;border:0;padding:4px 10px;border-radius:4px;cursor:pointer}
canvas{display:block;width:100vw;height:100vh}</style></head>
<body><canvas id="c"></canvas>
<div id="hud"><button id="b">ping Rust</button> <span id="r">...</span> | rust->web: <span id="e">...</span></div>
<script src="/__afterglow_bootstrap.js"></script>
<script>
// rust -> web: register a handler for the "tick" event the host emits.
window.afterglow.on('tick', (d) => { document.getElementById('e').textContent = JSON.stringify(d); });
// web -> rust: invoke "ping".
document.getElementById('b').onclick = async () => {
  const out = await window.afterglow.invoke('ping', { n: 42 });
  document.getElementById('r').textContent = JSON.stringify(out);
  console.log('pong from Rust: ' + JSON.stringify(out));
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
    // Rust -> Web: emit a "tick" event every 500ms (after the browser is up).
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        for i in 0u32.. {
            afterglow_cef::emit("tick", &format!(r#"{{"i":{i}}}"#));
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    AppBuilder::new()
        .title("afterglow-cef minimal")
        .size(1280, 800)
        .devtools(9222)
        .root("/index.html")
        .asset("/index.html", "text/html", HTML)
        .on_invoke(|method: &str, params: Value| match method {
            "ping" => json!({ "pong": params }),
            _ => json!({ "error": format!("unknown method: {method}") }),
        })
        .run();
}
