// @ts-nocheck
// The shell owns this surface for DOM-only documents.
// Game and Three.js startup do not execute this script.
let documentDevice = null;
let documentContext = null;
let documentReady = false;
globalThis.__documentLoaded = false;
globalThis.__presentDocument = () => {
  if (!documentContext) return;
  const texture = documentContext.getCurrentTexture();
  const encoder = documentDevice.createCommandEncoder();
  const pass = encoder.beginRenderPass({ colorAttachments: [{
    view: texture.createView(),
    clearValue: { r: 0, g: 0, b: 0, a: 1 },
    loadOp: 'clear', storeOp: 'store',
  }] });
  pass.end();
  documentDevice.queue.submit([encoder.finish()]);
  if (Deno.core.ops.op_try_present_surface() && globalThis.__documentLoaded && !documentReady) {
    documentReady = true;
    Deno.core.ops.op_engine_ready();
  }
};
void (async () => {
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error('No native document GPU adapter.');
  documentDevice = await adapter.requestDevice();
  documentContext = engineCanvas.getContext('webgpu');
  if (!documentContext) throw new Error('No native document GPU surface.');
  documentContext.configure({ device: documentDevice, format: navigator.gpu.getPreferredCanvasFormat() });
})().catch(error => Deno.core.reportUnhandledException(error));
