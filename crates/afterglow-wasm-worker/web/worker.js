import init, { serve } from "./pkg/afterglow_wasm_worker.js";
await init();
self.onmessage = (e) => {
  let msg;
  if (e.data instanceof ArrayBuffer) msg = new Uint8Array(e.data);
  else if (e.data instanceof Uint8Array) msg = e.data;
  else { self.postMessage("__err__:type " + typeof e.data); return; }
  try {
    const resp = serve(msg);
    self.postMessage(resp);
  } catch(err) {
    self.postMessage("__err__:" + err.message + " input=" + JSON.stringify(Array.from(msg)));
  }
};
self.postMessage("__ready__");
