// crates/afterglow-web/www/engine/procedural-vt.ts
var VT_PAGE_SIZE = 128;
var VT_PAGE_BORDER = 4;
var VT_SLOT_SIZE = VT_PAGE_SIZE + VT_PAGE_BORDER * 2;
function noiseHash(x, y, seed = 0) {
  let h = Math.imul(x | 0, 521288629) ^ Math.imul(y | 0, 1597334677) ^ Math.imul(seed | 0, 1831565813);
  h = Math.imul(h ^ h >>> 15, h | 1);
  h ^= h + Math.imul(h ^ h >>> 7, h | 61);
  return ((h ^ h >>> 14) >>> 0) / 4294967295;
}
function smoothNoise(x, y, scale, seed = 0) {
  const gx = x / scale, gy = y / scale, ix = Math.floor(gx), iy = Math.floor(gy), fx = gx - ix, fy = gy - iy;
  const sx = fx * fx * (3 - 2 * fx), sy = fy * fy * (3 - 2 * fy);
  const a = noiseHash(ix, iy, seed), b = noiseHash(ix + 1, iy, seed);
  const c = noiseHash(ix, iy + 1, seed), d = noiseHash(ix + 1, iy + 1, seed);
  return a + (b - a) * sx + (c + (d - c) * sx - (a + (b - a) * sx)) * sy;
}
function pagePixels(mip, pageX, pageY, virtualSize, pixel) {
  const out = new Uint8Array(VT_SLOT_SIZE * VT_SLOT_SIZE * 4), scale = 2 ** mip;
  for (let sy = 0;sy < VT_SLOT_SIZE; sy++)
    for (let sx = 0;sx < VT_SLOT_SIZE; sx++) {
      const mx = pageX * VT_PAGE_SIZE + sx - VT_PAGE_BORDER, my = pageY * VT_PAGE_SIZE + sy - VT_PAGE_BORDER;
      const x = Math.max(0, Math.min(virtualSize - 1, mx * scale));
      const y = Math.max(0, Math.min(virtualSize - 1, my * scale));
      const [r, g, b] = pixel(x, y, scale, mip);
      const i = (sy * VT_SLOT_SIZE + sx) * 4;
      out[i] = r;
      out[i + 1] = g;
      out[i + 2] = b;
      out[i + 3] = 255;
    }
  return out;
}
function generateTerrainPage(mip, pageX, pageY, virtualSize) {
  return pagePixels(mip, pageX, pageY, virtualSize, (x, y, mipScale) => {
    let value = 0, amplitude = 0.55, total = 0;
    for (const scale of [16384, 4096, 1024, 256, 64]) {
      if (scale >= mipScale * 2) {
        value += smoothNoise(x, y, scale) * amplitude;
        total += amplitude;
      }
      amplitude *= 0.5;
    }
    if (!total) {
      value = smoothNoise(x, y, mipScale * 2);
      total = 1;
    }
    value /= total;
    const ridge = 1 - Math.abs(value * 2 - 1), elevation = Math.max(0, Math.min(1, value * 0.72 + ridge * 0.28));
    let r, g, b;
    if (elevation < 0.38) {
      const t = elevation / 0.38;
      r = 8 + 18 * t;
      g = 24 + 55 * t;
      b = 55 + 95 * t;
    } else if (elevation < 0.58) {
      const t = (elevation - 0.38) / 0.2;
      r = 24 + 42 * t;
      g = 72 + 55 * t;
      b = 45 + 22 * t;
    } else if (elevation < 0.78) {
      const t = (elevation - 0.58) / 0.2;
      r = 66 + 78 * t;
      g = 127 + 48 * t;
      b = 67 + 54 * t;
    } else {
      const t = (elevation - 0.78) / 0.22;
      r = 144 + 105 * t;
      g = 175 + 74 * t;
      b = 121 + 128 * t;
    }
    const edge = Math.min(x, y, virtualSize - 1 - x, virtualSize - 1 - y);
    if (edge < 1024) {
      const checker = (Math.floor(x / 256) + Math.floor(y / 256) & 1) === 0;
      r = 255;
      g = checker ? 245 : 92;
      b = checker ? 235 : 18;
    }
    return [r, g, b];
  });
}

// crates/afterglow-web/www/engine/vt-gpu-test.ts
async function testRawFeedback(device, direction) {
  const transform = direction === "west" ? "vec2f(1.0-in.uv.x,in.uv.y)" : direction === "rotated" ? "vec2f(in.uv.y,1.0-in.uv.x)" : "in.uv";
  const shader = device.createShaderModule({ code: `struct Out{@builtin(position) position:vec4f,@location(0) uv:vec2f};@vertex fn vs(@builtin(vertex_index)i:u32)->Out{var p=array<vec2f,3>(vec2f(-1,-1),vec2f(3,-1),vec2f(-1,3));var o:Out;o.position=vec4f(p[i],0,1);o.uv=p[i]*.5+.5;return o;}@fragment fn fs(in:Out)->@location(0) vec2u{let q=${transform};let x=u32(clamp(floor(q.x*2048.),0.,2047.));let y=u32(clamp(floor(q.y*2048.),0.,2047.));return vec2u(0x80000000u|(x<<6)|(y<<17),0x12345678u);}` });
  const info = await shader.getCompilationInfo(), errors = info.messages.filter((x) => x.type === "error");
  if (errors.length)
    throw new Error(errors.map((x) => x.message).join(`
`));
  const pipeline = device.createRenderPipeline({ layout: "auto", vertex: { module: shader, entryPoint: "vs" }, fragment: { module: shader, entryPoint: "fs", targets: [{ format: "rg32uint" }] }, primitive: { topology: "triangle-list" } });
  const tex = device.createTexture({ size: [32, 32], format: "rg32uint", usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC }), buffer = device.createBuffer({ size: 8192, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ }), encoder = device.createCommandEncoder(), pass = encoder.beginRenderPass({ colorAttachments: [{ view: tex.createView(), loadOp: "clear", storeOp: "store", clearValue: { r: 0, g: 0, b: 0, a: 0 } }] });
  pass.setPipeline(pipeline);
  pass.draw(3);
  pass.end();
  encoder.copyTextureToBuffer({ texture: tex }, { buffer, bytesPerRow: 256 }, [32, 32]);
  device.queue.submit([encoder.finish()]);
  await buffer.mapAsync(GPUMapMode.READ);
  const words = new Uint32Array(buffer.getMappedRange());
  let valid = 0, minX = 2048, maxX = -1, minY = 2048, maxY = -1;
  const xs = new Set, ys = new Set;
  for (let i = 0;i < words.length; i += 2)
    if (words[i] & 2147483648 && words[i + 1] === 305419896) {
      valid++;
      const x = words[i] >>> 6 & 2047, y = words[i] >>> 17 & 2047;
      xs.add(x);
      ys.add(y);
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
  buffer.unmap();
  buffer.destroy();
  tex.destroy();
  if (valid !== 1024 || xs.size !== 32 || ys.size !== 32)
    throw new Error(`feedback ${direction} mismatch`);
  return { direction, valid, range: [minX, maxX, minY, maxY] };
}
async function testUploadLocations(device, atlasWidth, atlasHeight, slotSize) {
  const result = { rgba: 0, compressed: 0, compressedFormat: "unsupported" }, tex = device.createTexture({ size: [64, 64], format: "rgba8unorm", usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.COPY_SRC }), origins = [[0, 0], [60, 60], [24, 36]];
  for (let n = 0;n < 3; n++) {
    const color = [17 + n, 83, 201, 255], pixels = new Uint8Array(64);
    for (let i = 0;i < 64; i += 4)
      pixels.set(color, i);
    device.queue.writeTexture({ texture: tex, origin: origins[n] }, pixels, { bytesPerRow: 16, rowsPerImage: 4 }, [4, 4]);
    const b = device.createBuffer({ size: 1024, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ }), e = device.createCommandEncoder();
    e.copyTextureToBuffer({ texture: tex, origin: origins[n] }, { buffer: b, bytesPerRow: 256 }, [4, 4]);
    device.queue.submit([e.finish()]);
    await b.mapAsync(GPUMapMode.READ);
    const bytes = new Uint8Array(b.getMappedRange());
    for (let y = 0;y < 4; y++)
      for (let x = 0;x < 4; x++)
        for (let c = 0;c < 4; c++)
          if (bytes[y * 256 + x * 4 + c] !== color[c])
            throw new Error(`RGBA mismatch ${n}/${x}/${y}/${c}`);
    b.unmap();
    b.destroy();
    result.rgba++;
  }
  tex.destroy();
  const compressed = device.features.has("texture-compression-bc") ? ["bc7", "bc7-rgba-unorm"] : device.features.has("texture-compression-astc") ? ["astc", "astc-4x4-unorm"] : null;
  if (compressed) {
    const t = device.createTexture({ size: [atlasWidth, atlasHeight], format: compressed[1], usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.TEXTURE_BINDING }), o = [[0, 0], [atlasWidth - slotSize, atlasHeight - slotSize], [Math.floor(atlasWidth / slotSize / 2) * slotSize, Math.floor(atlasHeight / slotSize / 2) * slotSize]];
    for (let n = 0;n < 3; n++) {
      const blocks = new Uint8Array(34 * 34 * 16);
      blocks.fill(31 + n);
      device.queue.writeTexture({ texture: t, origin: o[n] }, blocks, { bytesPerRow: 544, rowsPerImage: 34 }, [slotSize, slotSize]);
      result.compressed++;
    }
    t.destroy();
    result.compressedFormat = compressed[0];
  }
  await device.queue.onSubmittedWorkDone();
  return result;
}

// crates/afterglow-web/www/engine/webgpu-only.ts
function disableWebGLFallback(renderer) {
  renderer._getFallback = null;
}
function assertWebGPUBackend(renderer) {
  if (renderer.backend?.isWebGPUBackend !== true || renderer.backend.device == null) {
    throw new Error("Afterglow requires a live WebGPU backend; WebGL fallback is forbidden.");
  }
}
function showWebGPUFailure(error) {
  const message = error instanceof Error ? error.message : String(error);
  const panel = document.createElement("pre");
  panel.id = "afterglow-webgpu-failure";
  panel.textContent = `Afterglow requires hardware WebGPU.

${message}`;
  panel.style.cssText = "box-sizing:border-box;margin:0;min-height:100vh;padding:24px;background:#11151c;color:#ff9a9a;font:16px/1.5 ui-monospace,monospace;white-space:pre-wrap";
  document.body.replaceChildren(panel);
  console.error("Afterglow WebGPU startup failed:", error);
}
var legacyWindowRendererFactory = (parameters) => {
  const legacyWindow = window;
  return new legacyWindow.THREE.WebGPURenderer(parameters);
};
async function createWebGPUOnlyRenderer(parameters = {}, factory) {
  const gpu = navigator.gpu;
  if (!gpu)
    throw new Error("navigator.gpu is unavailable. WebGL fallback is disabled.");
  const adapter = await gpu.requestAdapter();
  if (!adapter)
    throw new Error("Unable to acquire a hardware WebGPU adapter. WebGL fallback is disabled.");
  const renderer = factory(parameters);
  renderer.afterglowAdapterInfo = adapter.info;
  disableWebGLFallback(renderer);
  try {
    await renderer.init();
    assertWebGPUBackend(renderer);
  } catch (error) {
    renderer.dispose();
    throw error;
  }
  const onDeviceLost = renderer.onDeviceLost.bind(renderer);
  renderer.onDeviceLost = (info) => {
    onDeviceLost(info);
    showWebGPUFailure(new Error(`WebGPU device lost (${info.reason ?? "unknown"}): ${info.message ?? "no detail"}`));
  };
  return renderer;
}

// crates/afterglow-web/www/vt-demo.ts
var THREE = window.THREE;
var VT = window.AfterglowVT;
var { wgslFn, Fn, texture, sampler, uv, uniform, float, uint } = THREE;
var VIRTUAL_SIZE = 262144;
var PAGE_GRID = VIRTUAL_SIZE / VT.PAGE_SIZE;
var renderer = await createWebGPUOnlyRenderer({ antialias: true }, legacyWindowRendererFactory).catch((error) => {
  showWebGPUFailure(error);
  throw error;
});
renderer.setSize(innerWidth, innerHeight);
document.body.append(renderer.domElement);
var errors = [];
renderer.backend.device.addEventListener("uncapturederror", (e) => errors.push(String(e.error?.message ?? e.error)));
addEventListener("error", (e) => errors.push(String(e.error?.stack ?? e.message)));
addEventListener("unhandledrejection", (e) => errors.push(String(e.reason?.stack ?? e.reason)));
var loader = { read: async () => new Uint8Array, poll() {} };
var path = "procedural://terrain";
var store = new VT.VirtualTextureStore(loader, async (_, req) => generateTerrainPage(req.mip, req.x, req.y, VIRTUAL_SIZE), VT.FORMAT_RGBA, renderer.backend.device);
store.loadTexture(path, { width: VIRTUAL_SIZE, height: VIRTUAL_SIZE });
var entry = store.getEntry(path);
var atlas = texture(store.atlasTexture);
var pageTable = texture(entry.pageTableTexture);
var sample = wgslFn(VT.VT_SAMPLE_WGSL);
var material = new THREE.MeshStandardNodeMaterial({ roughness: 0.9, metalness: 0 });
material.colorNode = Fn(() => sample({ pageTable, atlas, atlasSampler: sampler(atlas), uv: uv(), virtualSize: uniform(new THREE.Vector2(VIRTUAL_SIZE, VIRTUAL_SIZE)), pageGrid: uniform(new THREE.Vector2(PAGE_GRID, PAGE_GRID)), pageSize: float(VT.PAGE_SIZE), pageBorder: float(VT.PAGE_BORDER), atlasSize: uniform(new THREE.Vector2(store.atlasWidth, store.atlasHeight)), maxMip: float(entry.maxMip), textureMaxMip: float(entry.textureMaxMip), addressMode: uint(0) }))();
var scene = new THREE.Scene;
scene.background = new THREE.Color(658448);
scene.add(new THREE.AmbientLight(8425632, 1.5));
var camera = new THREE.OrthographicCamera(-7, 7, 5, -5, 0.1, 30);
camera.position.z = 14;
var quad = new THREE.Mesh(new THREE.PlaneGeometry(12, 10), material);
scene.add(quad);
var camX = 0.5;
var camY = 0.5;
var zoom = 4;
var frame = 0;
var last = performance.now();
var programmatic = false;
var keys = new Set;
var waiters = [];
function feedback() {
  const out = new Map, mip = Math.max(0, Math.min(entry.maxMip - 1, Math.ceil(Math.log2(VIRTUAL_SIZE / zoom / Math.max(1, innerWidth))) + 1)), pages = PAGE_GRID >> mip, w = 1 / zoom, x0 = Math.max(0, Math.floor((camX - w / 2) * pages)), x1 = Math.min(pages - 1, Math.floor((camX + w / 2) * pages)), y0 = Math.max(0, Math.floor((camY - w / 2) * pages)), y1 = Math.min(pages - 1, Math.floor((camY + w / 2) * pages));
  for (let y = y0;y <= y1; y++)
    for (let x = x0;x <= x1; x++) {
      const req = { path, mip, x, y };
      out.set(`${mip}:${x}:${y}`, req);
    }
  return out;
}
await new Promise((r) => setTimeout(r, 0));
await renderer.renderAsync(scene, camera);
store.attachRenderer(renderer);
renderer.setAnimationLoop(async (now) => {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  if (!programmatic) {
    const speed = 1.4 / zoom * dt;
    camX = Math.max(0, Math.min(1, camX + ((keys.has("d") ? 1 : 0) - (keys.has("a") ? 1 : 0)) * speed));
    camY = Math.max(0, Math.min(1, camY + ((keys.has("w") ? 1 : 0) - (keys.has("s") ? 1 : 0)) * speed));
  }
  const result = store.processFeedback(feedback());
  store.poll();
  quad.geometry.attributes.uv.array.set([camX - 0.5 / zoom, camY + 0.5 / zoom, camX + 0.5 / zoom, camY + 0.5 / zoom, camX - 0.5 / zoom, camY - 0.5 / zoom, camX + 0.5 / zoom, camY - 0.5 / zoom]);
  quad.geometry.attributes.uv.needsUpdate = true;
  await renderer.renderAsync(scene, camera);
  frame++;
  for (let i = waiters.length - 1;i >= 0; i--)
    if (frame >= waiters[i].target) {
      waiters[i].resolve();
      waiters.splice(i, 1);
    }
  const d = store.getStats();
  document.getElementById("info").innerHTML = `<b>afterglow — Engine Virtual Texture</b><br>262,144² terrain · 256 GiB logical RGBA<br>Production VirtualTextureStore · ${store.atlasWidth}² shared atlas<br>UV ${camX.toFixed(3)}, ${camY.toFixed(3)} · zoom ${zoom.toFixed(1)}×<br>Resident ${d.atlasSlotsUsed}/${d.atlasSlotsTotal} · pending ${d.pendingPages}<br>Requests ${result.totalRequests} · errors ${errors.length}<br><br>WASD pan · wheel zoom · O overview · P pixel`;
});
addEventListener("keydown", (e) => {
  if (programmatic)
    return;
  keys.add(e.key.toLowerCase());
  if (e.key.toLowerCase() === "o")
    zoom = 0.5;
  if (e.key.toLowerCase() === "p")
    zoom = VIRTUAL_SIZE;
});
addEventListener("keyup", (e) => keys.delete(e.key.toLowerCase()));
addEventListener("wheel", (e) => {
  if (!programmatic)
    zoom = Math.max(0.5, Math.min(VIRTUAL_SIZE, zoom * Math.exp(e.deltaY * 0.001)));
});
addEventListener("resize", () => renderer.setSize(innerWidth, innerHeight));
var step = (n) => new Promise((resolve) => waiters.push({ target: frame + Math.max(1, n | 0), resolve }));
window.__afterglowVtGpuTest = { snapshot: () => ({ gpuReady: Boolean(store.gpuAtlasTexture), resident: store.getDebugSnapshot().atlasSlotsUsed, loaded: store.getDebugSnapshot().atlasSlotsUsed, errors: [...errors] }), setCamera: (x, y, z) => {
  programmatic = true;
  keys.clear();
  camX = Math.max(0, Math.min(1, x));
  camY = Math.max(0, Math.min(1, y));
  zoom = Math.max(0.5, Math.min(VIRTUAL_SIZE, z));
}, run: async () => {
  programmatic = true;
  for (let i = 0;i < 90; i++)
    await step(1);
  if (!store.gpuAtlasTexture)
    throw new Error("engine atlas not attached");
  const feedbackRuns = [];
  for (const direction of ["east", "west", "rotated"])
    feedbackRuns.push(await testRawFeedback(renderer.backend.device, direction));
  const residencyRuns = [], scenarios = [{ name: "eastbound", points: [{ x: 0.08, y: 0.25, z: 8 }, { x: 0.5, y: 0.25, z: 16 }, { x: 0.92, y: 0.25, z: 32 }] }, { name: "westbound", points: [{ x: 0.92, y: 0.75, z: 32 }, { x: 0.5, y: 0.75, z: 8 }, { x: 0.08, y: 0.75, z: 2 }] }, { name: "diagonal-lod", points: [{ x: 0.08, y: 0.08, z: 0.5 }, { x: 0.5, y: 0.5, z: 64 }, { x: 0.92, y: 0.92, z: VIRTUAL_SIZE }] }];
  for (const scenario of scenarios) {
    const before = store.getDebugSnapshot().atlasSlotsUsed, checkpoints = [];
    for (const p of scenario.points) {
      camX = p.x;
      camY = p.y;
      zoom = p.z;
      await step(35);
      checkpoints.push({ ...p, pages: store.getDebugSnapshot().atlasSlotsUsed });
    }
    residencyRuns.push({ name: scenario.name, before, after: store.getDebugSnapshot().atlasSlotsUsed, checkpoints });
  }
  const uploads = await testUploadLocations(renderer.backend.device, store.atlasWidth, store.atlasHeight, VT.SLOT_SIZE);
  await renderer.backend.device.queue.onSubmittedWorkDone();
  if (errors.length)
    throw new Error(errors.join(`
`));
  return { ok: true, feedbackRuns, uploads, residencyRuns, resident: store.getDebugSnapshot().atlasSlotsUsed, virtualSize: VIRTUAL_SIZE, atlas: [store.atlasWidth, store.atlasHeight] };
} };
console.log("afterglow-engine: engine-backed VT demo started");
