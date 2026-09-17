// Probe-GI demo: fully dynamic, no bake, map-agnostic probe field.
//
// Implements the design in docs/research/probe-based-gi.md:
//   * 3 camera-locked cascades (1 m / 3 m / 9 m), 8x4x8 probes each = 768 resident
//   * grid-snapped scrolling with modulo slot indexing, so a scroll step re-seeds
//     only the wrapped band (1/count of the field), not the whole volume
//   * 64 spherical rays per probe, closest hit through a BVH, hit shaded with the
//     direct light plus one shadow ray
//   * octahedral atlas with 1-texel borders, ping-pong rgba16float, hysteresis +
//     gamma-5 encoding
//   * per-fragment 8-probe cage: trilinear x backface x Chebyshev visibility
//   * probe validity / virtual offset / deactivation at init (fail closed)
//
// Run it as a MODULE path, not as an HTML page:
//   ./target/release/afterglow-shell prototype/probe-gi-demo/demo.js
// The module path is the shell's native_game.ts contract: the shell supplies
// globalThis.engineCanvas and the document, and startup is
//   __syncBrowserDocument(false) -> op_present_surface() -> op_engine_ready()
// which is also what opens the shell's input gate (dispatch_input drops every
// event until the runtime is ready).

import { buildScene, buildSimpleScene, buildBVH, selfTest } from './bvh.js';
import { GI_CORE_WGSL, GI_BLEND_RADIANCE_WGSL, GI_BLEND_DISTANCE_WGSL } from './gi.wgsl.js';
import { RENDER_WGSL } from './render.wgsl.js';

// ---------------------------------------------------------------- configuration
// Runtime configuration comes from a tiny entry module that sets
// globalThis.__probeGiConfig before importing demo.js, because the shell takes the
// page path verbatim and resolves no query string (see configs/).
const CFG = globalThis.__probeGiConfig ?? {};
const SCENE = CFG.scene === 'complex' ? 'complex' : 'simple';
const MOTION = CFG.motion === 'orbit' ? 'orbit' : 'static';
// Diagnostic view selector for configs/debug-*.js (in .ts order: before every bind group).
const DEBUG_MODE = Number(CFG.debug ?? 0);
const DEBUG_CASCADE = Number.isFinite(Number(CFG.debugCascade)) ? Number(CFG.debugCascade) : -1;
const debugData = new Float32Array(4);
const PROBE_X = 8, PROBE_Y = 4, PROBE_Z = 8;
const PER_CASCADE = PROBE_X * PROBE_Y * PROBE_Z;
const CASCADES = 3;
const TOTAL_PROBES = CASCADES * PER_CASCADE;
const SPACINGS = [1, 3, 9];
const RAYS_PER_PROBE = 64;
const TOTAL_RAYS = TOTAL_PROBES * RAYS_PER_PROBE;
const HYSTERESIS = 0.93;
const SELF_SHADOW_BIAS = 0.225;      // (0.75 * minSpacing) * 0.3, in spacing units
const SKY_INTENSITY = 1.4;
// Point-light intensity must be within a few x of the sky/ambient level, or the
// direct term clips to white while the indirect term stays invisible. At 45 the
// direct saturated 90% of the scene and the GI could never be seen. The room is
// ~20 m across, so it needs a scene-scaled value. Both shading paths now divide the
// direct term by PI (albedo/PI * E), so this constant is an irradiance and carries a
// compensating PI compared with the old albedo * direct form.
// A light inside the room sits 2-4 m from the surfaces, so it needs a far lower intensity
// than the same light 11 m outside did: E = I/(1+0.09d^2) is ~17 with the old 31.5 and the
// frame goes flat white. ~5 gives E ~ 3 at 3 m, which reads as a lit room with visible
// bounce instead of either a blown-out or a black one.
const LIGHT_INTENSITY = SCENE === 'complex' ? 5.0 : 12.5;
/** Scene-scaled light path. The room's only light is outside its +X doorway: everything
 *  inside it is then reached by direct light through the opening plus probe bounce, which
 *  is what makes an enclosed scene a real test of the GI rather than of the light. */
const lightPosAt = t => SCENE === 'complex'
  ? (CFG.seal === true
    // Leak test (`configs/sealed.js`) keeps the light OUTSIDE the sealed room, so the
    // interior has no legitimate source at all and any light in it is a leak.
    ? [11.0, 3.2, Math.sin(t * 0.3) * 3.0]
    // Normal room: the light is INSIDE. With it outside the doorway (the earlier setup)
    // the interior was lit only by the shaft plus one bounce, which is physically correct
    // and reads as a black room with a bright rectangle - indistinguishable from a bug.
    // Indoors, the room is lit and the GI shows up as bounce into the corners.
    : [3.0, 3.0, Math.sin(t * 0.3) * 3.0])
  : [Math.sin(t * 0.4) * 3.0, 4.0, Math.cos(t * 0.4) * 3.0];
// Atlases: an 8x8 octahedral map per probe plus a 1-texel border, so one hardware
// filtered fetch per probe works and the border absorbs the seam. The border costs 36%
// more texels than a border-free 8x8 map with a manual 4-tap mirroring fetch, and pays
// for itself: the manual fetch measured 61 fps against 120 for this one.
// The interior mapping must land on texel CENTRES (slot*tile + 1.5 + uv*interior); the
// original +1.0 landed on the edge between the border texel and the first interior one,
// so every sample was a 50/50 blend with the seam.
// 768 probes need 28 slots per row (28*28 = 784) -> 28*10 = 280, rounded to 288.
const OCT = 8;
const RAD_INTERIOR = OCT, RAD_TILE = OCT + 2, RAD_ATLAS = 288;
const RAD_SLOTS = Math.floor(RAD_ATLAS / RAD_TILE);
const DIST_INTERIOR = OCT, DIST_TILE = OCT + 2, DIST_ATLAS = 288;
const DIST_SLOTS = Math.floor(DIST_ATLAS / DIST_TILE);
if (RAD_INTERIOR !== OCT || DIST_INTERIOR !== OCT)
  throw new Error('the octahedral fetch is written for 8x8 maps');
if (RAD_SLOTS * RAD_SLOTS < TOTAL_PROBES || DIST_SLOTS * DIST_SLOTS < TOTAL_PROBES)
  throw new Error('atlas too small for the probe count');

// CPU-verifiable invariants (prototype/probe-gi-demo/selftest.mjs checks the rest).
if (RAD_SLOTS * RAD_SLOTS < TOTAL_PROBES) throw new Error('radiance atlas too small');
if (DIST_SLOTS * DIST_SLOTS < TOTAL_PROBES) throw new Error('distance atlas too small');

// ---------------------------------------------------------------- HUD
// On the module path there is no HTML, so the document has no styling at all and
// the shell's HUD composite would paint the default white body background over the
// WebGPU surface. Make the page transparent so the rendered surface shows through.
const pageStyle = document.createElement('style');
pageStyle.textContent = 'html,body{margin:0;background:transparent;overflow:hidden}' +
  'canvas{display:block;width:100%;height:100%}';
document.head.appendChild(pageStyle);
// Inline too: the shell composites the page over the WebGPU surface, so an opaque
// document background hides the render completely (the module path has no CSS).
document.documentElement.style.background = 'transparent';
document.body.style.background = 'transparent';

let hud = document.getElementById('hud');
if (!hud) {
  hud = document.createElement('div');
  hud.id = 'hud';
  hud.style.cssText = 'position:fixed;left:10px;top:10px;z-index:10;padding:8px 10px;' +
    'color:#dbe6ff;background:rgba(0,0,0,.62);border:1px solid rgba(120,160,255,.35);' +
    'border-radius:8px;font:12px/1.5 monospace;white-space:pre';
  document.body.appendChild(hud);
}
let help = document.getElementById('help');
if (!help) {
  help = document.createElement('div');
  help.id = 'help';
  help.style.cssText = 'position:fixed;right:10px;top:10px;z-index:10;padding:8px 10px;' +
    'color:#b9c7e8;background:rgba(0,0,0,.5);border-radius:8px;font:11px/1.5 monospace;white-space:pre';
  help.textContent = 'WASD / Arrows: fly\nShift: sprint\nSpace / C: up / down\n' +
    'drag: look   wheel: speed';
  document.body.appendChild(help);
}
const log = (...a) => console.log('[probe-gi]', ...a);

// ---------------------------------------------------------------- probe field (host)
const basePos = new Float32Array(TOTAL_PROBES * 4);
const probeFlagsHost = new Uint32Array(TOTAL_PROBES);
const slotCell = new Int32Array(TOTAL_PROBES * 3);
// Per-probe index whose previous atlas a reseeded probe inherits (0xffffffff = none).
const seedHost = new Uint32Array(TOTAL_PROBES).fill(0xffffffff);
const cascadeCell = new Int32Array(CASCADES * 3);
let seeded = false;

/** Grid-aligned probe positions; modulo slot mapping so a scroll step re-seeds one band. */
function updateScroll(camPos) {
  let moved = 0;
  for (let c = 0; c < CASCADES; c++) {
    const sp = SPACINGS[c];
    const counts = [PROBE_X, PROBE_Y, PROBE_Z];
    const o = [camPos[0] - counts[0] * sp * 0.5, camPos[1] - counts[1] * sp * 0.5, camPos[2] - counts[2] * sp * 0.5];
    const cell = [Math.floor(o[0] / sp), Math.floor(o[1] / sp), Math.floor(o[2] / sp)];
    for (let s = 0; s < PER_CASCADE; s++) {
      const probe = c * PER_CASCADE + s;
      const local = [s % PROBE_X, Math.floor(s / PROBE_X) % PROBE_Y, Math.floor(s / (PROBE_X * PROBE_Y))];
      let changed = !seeded;
      let movedAxis = -1, movedSign = 0;
      for (let a = 0; a < 3; a++) {
        const d = ((local[a] - cell[a]) % counts[a] + counts[a]) % counts[a];
        const cel = cell[a] + d;
        if (slotCell[probe * 3 + a] !== cel) {
          movedAxis = a;
          movedSign = Math.sign(cel - slotCell[probe * 3 + a]);
          slotCell[probe * 3 + a] = cel;
          changed = true;
        }
        basePos[probe * 4 + a] = (cel + 0.5) * sp;
      }
      // The reseeded band is the wrapped edge of the field, so its inward neighbour is one
      // slot back along the axis the camera moved. That neighbour holds a converged atlas
      // from one cell away, which the fresh probe inherits (see probeHysteresis).
      if (changed && seeded && movedAxis >= 0) {
        const stride = movedAxis === 0 ? 1 : movedAxis === 1 ? PROBE_X : PROBE_X * PROBE_Y;
        const neighbour = probe + (movedSign > 0 ? stride : -stride);
        const local0 = probe % PER_CASCADE;
        const neighbourLocal = neighbour - c * PER_CASCADE;
        seedHost[probe] = (neighbourLocal >= 0 && neighbourLocal < PER_CASCADE) ? neighbour : 0xffffffff;
      } else {
        seedHost[probe] = 0xffffffff;
      }
      basePos[probe * 4 + 3] = 0;
      // CFG.forceFreshBand marks one x-slab as reseeded every frame, which reproduces (on
      // demand) the steady-state state a scrolling camera creates: a band of probes whose
      // atlas is one frame old next to probes averaging ~14 frames under hysteresis.
      const forced = CFG.forceFreshBand === true && local[0] === 0;
      if (forced && !changed && seeded) {
        // Mirrors what a real scroll does, so the band is reproducible with a static
        // camera: seed from the inward neighbour along x.
        const neighbour = probe + 1;
        seedHost[probe] = (neighbour - c * PER_CASCADE) < PER_CASCADE ? neighbour : 0xffffffff;
      }
      probeFlagsHost[probe] = (changed || forced) ? 2 : 0;
      if (changed) moved++;
    }
    cascadeCell[c * 3] = cell[0];
    cascadeCell[c * 3 + 1] = cell[1];
    cascadeCell[c * 3 + 2] = cell[2];
  }
  seeded = true;
  return moved;
}

// ---------------------------------------------------------------- device + surface
const canvas = document.getElementById('view') ?? globalThis.engineCanvas ?? (() => {
  const c = document.createElement('canvas');
  c.id = 'view';
  document.body.appendChild(c);
  return c;
})();
// The shell manages the drawing-buffer size through resizeEngineCanvas; on the
// module path the canvas starts at the 300x150 default, so sizing must be explicit.
function sizeSurface() {
  const w = Math.max(1, window.innerWidth || globalThis.__surfaceWidth || 1280);
  const h = Math.max(1, window.innerHeight || globalThis.__surfaceHeight || 720);
  globalThis.resizeEngineCanvas?.(w, h, globalThis.devicePixelRatio || 1);
  canvas.style.position = 'fixed';
  canvas.style.inset = '0';
  canvas.style.width = '100%';
  canvas.style.height = '100%';
  canvas.style.background = 'transparent';
  canvas.width = w;
  canvas.height = h;
}
sizeSurface();
addEventListener('resize', sizeSurface);

const adapter = await navigator.gpu.requestAdapter();
if (!adapter) throw new Error('no WebGPU adapter');
const device = await adapter.requestDevice();
// A validation error (a bind group that does not match its layout, say) invalidates the
// whole command buffer, so the frame comes out black with nothing in the log. Surface it.
device.onuncapturederror = e => log('UNCAPTURED DEVICE ERROR: ' + (e.error?.message ?? e.error));
const ctx = canvas.getContext('webgpu');
const format = navigator.gpu.getPreferredCanvasFormat();
const configureSurface = () => ctx.configure({ device, format, alphaMode: 'opaque' });
configureSurface();
log('adapter', adapter.info?.description ?? '?', 'format', format, 'canvas', canvas.id || '(anon)');

// ---------------------------------------------------------------- scene + BVH
// Known-answer scene by default: one ground plane and one cube. The dungeon-ish
// room (see configs/complex.js) is the enclosed-space case: it is what exercises
// probe relocation and shows whether the probes leak the sky through walls.
const scene = SCENE === 'simple' ? buildSimpleScene() : buildScene();
const bvh = buildBVH(scene.tris, scene.triCount);
const test = selfTest(scene, bvh, 512);
log(`scene ${scene.triCount} tris, BVH ${bvh.nodeCount} nodes; self-test ${test.checked - test.bad}/${test.checked} match brute force`);
if (test.bad !== 0) log('WARNING: BVH self-test mismatches', test.bad);

const triPos = new Float32Array(scene.triCount * 12);
const triNormals = new Float32Array(scene.triCount * 3);
for (let i = 0; i < scene.triCount; i++) {
  for (let v = 0; v < 3; v++) {
    for (let a = 0; a < 3; a++) triPos[i * 12 + v * 4 + a] = scene.tris[i * 9 + v * 3 + a];
  }
  const ax = scene.tris[i * 9], ay = scene.tris[i * 9 + 1], az = scene.tris[i * 9 + 2];
  const ux = scene.tris[i * 9 + 3] - ax, uy = scene.tris[i * 9 + 4] - ay, uz = scene.tris[i * 9 + 5] - az;
  const vx = scene.tris[i * 9 + 6] - ax, vy = scene.tris[i * 9 + 7] - ay, vz = scene.tris[i * 9 + 8] - az;
  let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
  const len = Math.hypot(nx, ny, nz) || 1;
  triNormals[i * 3] = nx / len; triNormals[i * 3 + 1] = ny / len; triNormals[i * 3 + 2] = nz / len;
}

// 9 floats per vertex: position (3), normal (3), albedo (3).
const vertexData = new Float32Array(scene.triCount * 3 * 9);
for (let i = 0; i < scene.triCount; i++) {
  // Same rule as triAlbedo() in the shaders: orientation-driven, no per-triangle
  // hash (a hash makes every triangle its own tint and disagrees with indirect).
  const ny = Math.abs(triNormals[i * 3 + 1]);
  const horizontal = ny >= 0.7;
  const base = horizontal ? [0.30, 0.29, 0.28] : [0.40, 0.38, 0.36];
  const warm = Math.min(Math.max((scene.tris[i * 9 + 1] - 1.0) * 0.10, 0), 0.18);
  const crate = [0.46, 0.33, 0.22];
  const alb = base.map((v, c) => v + (crate[c] - v) * warm);
  for (let v = 0; v < 3; v++) {
    const o = (i * 3 + v) * 9;
    vertexData[o] = scene.tris[i * 9 + v * 3];
    vertexData[o + 1] = scene.tris[i * 9 + v * 3 + 1];
    vertexData[o + 2] = scene.tris[i * 9 + v * 3 + 2];
    vertexData[o + 3] = triNormals[i * 3];
    vertexData[o + 4] = triNormals[i * 3 + 1];
    vertexData[o + 5] = triNormals[i * 3 + 2];
    vertexData[o + 6] = alb[0]; vertexData[o + 7] = alb[1]; vertexData[o + 8] = alb[2];
  }
}

// ---------------------------------------------------------------- buffers
const S = GPUBufferUsage.STORAGE, U = GPUBufferUsage.UNIFORM, C = GPUBufferUsage.COPY_DST;
const SS = GPUShaderStage;
// Upload through writeBuffer rather than mappedAtCreation: the mapped-range path
// was leaving the vertex buffer's first bytes aliased with a storage buffer.
const mk = (data, usage) => {
  const b = device.createBuffer({ size: Math.max(16, data.byteLength), usage: usage | C });
  device.queue.writeBuffer(b, 0, data);
  return b;
};
const nodeBuf = mk(bvh.nodes, S);
const triBuf = mk(triPos, S);
const idxBuf = mk(bvh.order, S);
const rightsBuf = mk(bvh.rights, S);
const probePosBuf = mk(basePos, S | GPUBufferUsage.COPY_SRC);
const probeFlagsBuf = mk(probeFlagsHost, S);
const rayBuf = device.createBuffer({ size: TOTAL_RAYS * 16, usage: S });
const paramsBuf = device.createBuffer({ size: 128, usage: U | C });
const seedBuf = device.createBuffer({ size: TOTAL_PROBES * 4, usage: S | C });
const cameraBuf = device.createBuffer({ size: 80, usage: U | C });
// Upload a snapshot: the live array must not be able to change what the GPU holds.
const vertexUpload = vertexData.slice();
const vertexBuf = mk(vertexUpload, GPUBufferUsage.VERTEX | GPUBufferUsage.STORAGE);

const TEX = GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.STORAGE_BINDING |
  // COPY_SRC is for the CFG.fieldTrace readback only; the flag itself is free.
  GPUTextureUsage.COPY_SRC;
// rgba16float for both atlases: rg16float storage needs texture-formats-tier1,
// which this adapter does not expose (docs/research/probe-gi-placement-leaks-packing.md).
const radiance = [0, 1].map(() => device.createTexture({ size: [RAD_ATLAS, RAD_ATLAS], format: 'rgba16float', usage: TEX }));
const distance = [0, 1].map(() => device.createTexture({ size: [DIST_ATLAS, DIST_ATLAS], format: 'rgba16float', usage: TEX }));
const sampler = device.createSampler({ magFilter: 'linear', minFilter: 'linear', addressModeU: 'clamp-to-edge', addressModeV: 'clamp-to-edge' });
let depthTexture = device.createTexture({ size: [canvas.width, canvas.height], format: 'depth24plus', usage: GPUTextureUsage.RENDER_ATTACHMENT });
function ensureDepth() {
  if (depthTexture.width === canvas.width && depthTexture.height === canvas.height) return;
  depthTexture.destroy();
  depthTexture = device.createTexture({ size: [canvas.width, canvas.height], format: 'depth24plus', usage: GPUTextureUsage.RENDER_ATTACHMENT });
}

// ---------------------------------------------------------------- layouts + pipelines
const storageRO = { visibility: SS.COMPUTE, buffer: { type: 'read-only-storage' } };
const storageRW = { visibility: SS.COMPUTE, buffer: { type: 'storage' } };
const computeGroup0 = device.createBindGroupLayout({
  entries: [
    { binding: 0, ...storageRO }, { binding: 1, ...storageRO }, { binding: 2, ...storageRO },
    { binding: 3, ...storageRO }, { binding: 4, ...storageRW }, { binding: 5, ...storageRW },
    { binding: 6, ...storageRW },
    { binding: 7, visibility: SS.COMPUTE, buffer: { type: 'uniform' } },
    { binding: 8, ...storageRO },
  ],
});
const blendLayout = fmt => device.createBindGroupLayout({
  entries: [
    { binding: 0, visibility: SS.COMPUTE, texture: { sampleType: 'unfilterable-float' } },
    { binding: 1, visibility: SS.COMPUTE, storageTexture: { access: 'write-only', format: fmt } },
  ],
});
const blendRadLayout = blendLayout('rgba16float');
const blendDistLayout = blendLayout('rgba16float');
const renderStorage = { visibility: SS.FRAGMENT, buffer: { type: 'read-only-storage' } };
const renderGroup0 = device.createBindGroupLayout({
  entries: [
    { binding: 0, ...renderStorage }, { binding: 1, ...renderStorage },
    { binding: 2, ...renderStorage }, { binding: 3, ...renderStorage },
    { binding: 4, ...renderStorage },
    { binding: 5, visibility: SS.FRAGMENT, buffer: { type: 'uniform' } },
    { binding: 6, visibility: SS.FRAGMENT, texture: { sampleType: 'float' } },
    { binding: 7, visibility: SS.FRAGMENT, texture: { sampleType: 'float' } },
    { binding: 8, visibility: SS.FRAGMENT, sampler: { type: 'filtering' } },
  ],
});
const renderGroup1 = device.createBindGroupLayout({
  entries: [
    { binding: 0, visibility: SS.VERTEX | SS.FRAGMENT, buffer: { type: 'uniform' } },
    { binding: 1, visibility: SS.FRAGMENT, buffer: { type: 'uniform' } },
  ],
});

async function compile(code, label) {
  const m = device.createShaderModule({ code, label });
  const info = await m.getCompilationInfo();
  const errs = (info.messages || []).filter(x => x.type === 'error');
  if (errs.length) {
    for (const e of errs) log(`WGSL ERROR ${label} ${e.lineNum}:${e.linePos} ${e.message}`);
    throw new Error(`${label}: WGSL compile failed`);
  }
  return m;
}
const coreModule = await compile(GI_CORE_WGSL, 'giCore');
const blendRadModule = await compile(GI_BLEND_RADIANCE_WGSL, 'blendRadiance');
const blendDistModule = await compile(GI_BLEND_DISTANCE_WGSL, 'blendDistance');
const renderModule = await compile(RENDER_WGSL, 'render');

const computePipeline = (module, entryPoint, layouts) => device.createComputePipeline({
  layout: device.createPipelineLayout({ bindGroupLayouts: layouts }), compute: { module, entryPoint },
});
const pInit = computePipeline(coreModule, 'probeInit', [computeGroup0]);
const pTrace = computePipeline(coreModule, 'probeTrace', [computeGroup0]);
const pBlendRad = computePipeline(blendRadModule, 'blendRadiance', [computeGroup0, blendRadLayout]);
const pBlendDist = computePipeline(blendDistModule, 'blendDistance', [computeGroup0, blendDistLayout]);
const renderPipeline = device.createRenderPipeline({
  layout: device.createPipelineLayout({ bindGroupLayouts: [renderGroup0, renderGroup1] }),
  // Declare the vertex layout again: WebGPU draws need the pipeline to describe the
  // vertex stream it is fed, even when the shader ignores it.
  vertex: {
    module: renderModule, entryPoint: 'vs',
    buffers: [{
      arrayStride: 36,
      attributes: [
        { shaderLocation: 0, offset: 0, format: 'float32x3' },   // position
        { shaderLocation: 1, offset: 12, format: 'float32x3' },  // normal
        { shaderLocation: 2, offset: 24, format: 'float32x3' },  // albedo
      ],
    }],
  },
  fragment: { module: renderModule, entryPoint: 'fs', targets: [{ format }] },
  primitive: { topology: 'triangle-list', cullMode: 'none', frontFace: 'ccw' },
  // Depth testing is NOT optional: without it the last-drawn triangle wins, so the
  // cube's back faces overwrite its front faces and read as black notches.
  depthStencil: { format: 'depth24plus', depthWriteEnabled: true, depthCompare: 'less' },
});

const computeBindGroup = device.createBindGroup({
  layout: computeGroup0,
  entries: [
    { binding: 0, resource: { buffer: nodeBuf } }, { binding: 1, resource: { buffer: triBuf } },
    { binding: 2, resource: { buffer: idxBuf } }, { binding: 3, resource: { buffer: rightsBuf } },
    { binding: 4, resource: { buffer: probePosBuf } }, { binding: 5, resource: { buffer: probeFlagsBuf } },
    { binding: 6, resource: { buffer: rayBuf } }, { binding: 7, resource: { buffer: paramsBuf } },
    { binding: 8, resource: { buffer: seedBuf } },
  ],
});
const mkRenderBind = atlas => device.createBindGroup({
  layout: renderGroup0,
  entries: [
    { binding: 0, resource: { buffer: nodeBuf } }, { binding: 1, resource: { buffer: triBuf } },
    { binding: 2, resource: { buffer: idxBuf } }, { binding: 3, resource: { buffer: rightsBuf } },
    { binding: 4, resource: { buffer: probePosBuf } }, { binding: 5, resource: { buffer: paramsBuf } },
    { binding: 6, resource: radiance[atlas].createView() }, { binding: 7, resource: distance[atlas].createView() },
    { binding: 8, resource: sampler },
  ],
});
const renderBinds = [mkRenderBind(0), mkRenderBind(1)];
const debugBuf = device.createBuffer({ size: 16, usage: U | C });
const renderBind1 = device.createBindGroup({
  layout: renderGroup1,
  entries: [
    { binding: 0, resource: { buffer: cameraBuf } },
    { binding: 1, resource: { buffer: debugBuf } },
  ],
});
debugData[0] = DEBUG_MODE; debugData[1] = DEBUG_CASCADE;
device.queue.writeBuffer(debugBuf, 0, debugData);

const params = new ArrayBuffer(128);
const paramsF = new Float32Array(params);
const paramsU = new Uint32Array(params);
const camData = new Float32Array(20);

function writeParams(lightPos) {
  paramsU[0] = PROBE_X; paramsU[1] = PROBE_Y; paramsU[2] = PROBE_Z; paramsU[3] = TOTAL_PROBES;
  paramsU[4] = RAD_INTERIOR; paramsU[5] = RAD_TILE; paramsU[6] = DIST_INTERIOR; paramsU[7] = DIST_TILE;
  paramsU[8] = RAD_ATLAS; paramsU[9] = DIST_ATLAS; paramsU[10] = RAD_SLOTS; paramsU[11] = DIST_SLOTS;
  paramsF[12] = RAYS_PER_PROBE; paramsF[13] = HYSTERESIS; paramsF[14] = SELF_SHADOW_BIAS; paramsF[15] = SKY_INTENSITY;
  for (let c = 0; c < CASCADES; c++) {
    const sp = SPACINGS[c];
    paramsF[16 + c * 4 + 0] = cascadeCell[c * 3] * sp;
    paramsF[16 + c * 4 + 1] = cascadeCell[c * 3 + 1] * sp;
    paramsF[16 + c * 4 + 2] = cascadeCell[c * 3 + 2] * sp;
    paramsF[16 + c * 4 + 3] = sp;
  }
  paramsF[28] = lightPos[0]; paramsF[29] = lightPos[1]; paramsF[30] = lightPos[2]; paramsF[31] = LIGHT_INTENSITY;
  device.queue.writeBuffer(paramsBuf, 0, params);
}

// ---------------------------------------------------------------- camera + input
// Free-fly camera. Presets seed the multi-view captures: inside the room looking
// at the doorway, outside the doorway looking in, and a room-corner overview.
// Presets for the simple scene: a 3/4 view of the cube, a low grazing view of the
// ground bounce, and a top-down view. yaw 0 looks along +Z, yaw pi/2 along +X.
// CFG.camera (configs/closeup.js) replaces the first preset, for close-range inspection
// where the cage's cells cover large parts of a surface.
// CFG.cameras (configs/*) replaces the whole preset list, so one run can sweep several
// view distances and show the surfaces where the probe cage's cells are large.
const camPresets = CFG.cameras ?? (SCENE === 'complex' ? [
  // inside the room looking at the +X doorway, just outside it, room corner
  { p: [0.0, 1.6, -3.0], yaw: Math.PI * 0.5, pitch: 0.0 },
  { p: [12.0, 1.6, 0.0], yaw: -Math.PI * 0.5, pitch: 0.0 },
  { p: [5.5, 3.0, 5.5], yaw: Math.PI * 1.25, pitch: -0.32 },
] : [
  { p: [5.0, 3.2, 5.0], yaw: Math.PI * 1.25, pitch: -0.28 },
  { p: [4.2, 0.9, 2.2], yaw: Math.PI * 1.42, pitch: 0.05 },
  { p: [0.01, 9.0, 0.01], yaw: 0.0, pitch: -1.45 },
]);
const cam = {
  x: camPresets[0].p[0], y: camPresets[0].p[1], z: camPresets[0].p[2],
  yaw: camPresets[0].yaw, pitch: camPresets[0].pitch,
};
let camSpeed = 4.0;
let sprint = false;
let lastMoveT = performance.now();
let inputSeen = false;
let lastKey = '(none)';

// Key state. `keys` is the live boolean map; `keyPulse` records the last press so a
// tap still produces a short, visible movement (the shell can end a press early,
// which otherwise reads as "the key does nothing").
const keys = Object.create(null);
const keyPulse = Object.create(null);
const MIN_HOLD_MS = 170;
const seenEvents = new WeakSet();
const onInput = (type, handler, opts) => {
  const wrapped = e => { if (seenEvents.has(e)) return; seenEvents.add(e); handler(e); };
  for (const target of [document, window, canvas]) {
    if (target && typeof target.addEventListener === 'function') target.addEventListener(type, wrapped, opts);
  }
};
onInput('keydown', e => {
  keys[e.code] = true;
  keyPulse[e.code] = performance.now();
  lastKey = e.code;
  inputSeen = true;
});
onInput('keyup', e => { keys[e.code] = false; });
onInput('blur', () => { for (const code in keys) keys[code] = false; });
let dragging = false, lastX = 0, lastY = 0;
onInput('pointerdown', e => { dragging = true; inputSeen = true; lastX = e.clientX; lastY = e.clientY; });
onInput('pointerup', () => { dragging = false; });
onInput('pointermove', e => {
  if (!dragging) return;
  cam.yaw -= (e.clientX - lastX) * 0.005;
  cam.pitch = Math.max(-1.45, Math.min(1.45, cam.pitch - (e.clientY - lastY) * 0.004));
  lastX = e.clientX; lastY = e.clientY;
});
onInput('wheel', e => { inputSeen = true; camSpeed = Math.max(0.6, Math.min(40, camSpeed * (e.deltaY > 0 ? 0.9 : 1.1))); }, { passive: true });

function lookAt(eye, target, up, out) {
  const zx = eye[0] - target[0], zy = eye[1] - target[1], zz = eye[2] - target[2];
  const zl = Math.hypot(zx, zy, zz) || 1;
  const z = [zx / zl, zy / zl, zz / zl];
  let x = [up[1] * z[2] - up[2] * z[1], up[2] * z[0] - up[0] * z[2], up[0] * z[1] - up[1] * z[0]];
  const xl = Math.hypot(x[0], x[1], x[2]) || 1;
  x = [x[0] / xl, x[1] / xl, x[2] / xl];
  const y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
  const f = 1 / Math.tan(0.5 * 60 * Math.PI / 180);
  const aspect = canvas.width / Math.max(1, canvas.height);
  const near = 0.05, far = 400;
  const k = f / aspect;
  const A = far / (near - far), B = near * far / (near - far);
  // MUST include the eye translation: without it the shader views the world from the
  // origin (the CPU-side project() below subtracts the eye by hand, so a missing
  // translation here shows up only on the GPU as "the camera is inside the scene").
  const dx = x[0] * eye[0] + x[1] * eye[1] + x[2] * eye[2];
  const dy = y[0] * eye[0] + y[1] * eye[1] + y[2] * eye[2];
  const dz = z[0] * eye[0] + z[1] * eye[1] + z[2] * eye[2];
  out[0] = x[0] * k; out[1] = y[0] * f; out[2] = z[0] * A; out[3] = -z[0];
  out[4] = x[1] * k; out[5] = y[1] * f; out[6] = z[1] * A; out[7] = -z[1];
  out[8] = x[2] * k; out[9] = y[2] * f; out[10] = z[2] * A; out[11] = -z[2];
  out[12] = -k * dx; out[13] = -f * dy; out[14] = B - A * dz; out[15] = dz;
}

// ---------------------------------------------------------------- capture (for automated runs)
// CFG.captureFrames overrides the three default marks: the temporal A/B needs CONSECUTIVE
// frames (a scroll step is a one-frame event, so two frames 60 apart cannot show it).
const CAPTURE_FRAMES = CFG.captureFrames ?? [70, 130, 190];
// Readbacks are async and emit their payload with yields, so the "done" marker that the
// runner waits for must not fire while one is still streaming - it used to, and the shell
// was killed mid-capture, leaving a truncated block.
let pendingCaptures = 0;
// configs/nocapture.js turns the automated capture set off; run.sh no longer edits this
// file (a sed against source was the previous mechanism).
let captureEnabled = CFG.capture !== false;
let captureIndex = 0;
async function readbackFrame(buffer, w, h, bytesPerRow, viewIndex) {
  await buffer.mapAsync(GPUMapMode.READ);
  const src = new Uint8Array(buffer.getMappedRange());
  const dw = w >> 1, dh = h >> 1;
  const out = new Uint8Array(dw * dh * 4);
  let o = 0, sum = 0;
  for (let y = 0; y < dh; y++) {
    for (let x = 0; x < dw; x++) {
      const s = (y * 2) * bytesPerRow + (x * 2) * 4;
      for (let c = 0; c < 4; c++) out[o + c] = src[s + c];
      sum += out[o] + out[o + 1] + out[o + 2];
      o += 4;
    }
  }
  buffer.unmap();
  log(`capture ${viewIndex} ${dw}x${dh} mean luma ${(sum / (dw * dh * 3)).toFixed(1)}`);
  // Base64 is emitted in fixed-size chunks straight to the log, never into one growing
  // string. The previous version concatenated ~690 KB with += (quadratic) and then sliced
  // it, which blocked the JS thread for tens of milliseconds; that block is what made the
  // shell's surface acquire time out ("Invalid Surface Status: Timeout") and killed the
  // frame loop after the capture set. Yielding between chunks keeps the loop live.
  const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const CHARS = 24000;
  // Each chunk is logged on its own marked line, so a frame/HUD log line landing between
  // chunks (the yields below let that happen) cannot be mistaken for payload. The old
  // begin/end block format required the block to be uninterrupted, which it no longer is.
  log(`capture-begin ${viewIndex} ${dw} ${dh}`);
  let chunk = '';
  for (let i = 0; i < out.length; i += 3) {
    const b0 = out[i], b1 = out[i + 1] ?? 0, b2 = out[i + 2] ?? 0;
    chunk += B64[b0 >> 2] + B64[((b0 & 3) << 4) | (b1 >> 4)] +
      (i + 1 < out.length ? B64[((b1 & 15) << 2) | (b2 >> 6)] : '=') +
      (i + 2 < out.length ? B64[b2 & 63] : '=');
    if (chunk.length >= CHARS) { log(`capture-chunk ${viewIndex} ${chunk}`); chunk = ''; await captureYield(); }
  }
  if (chunk.length) { log(`capture-chunk ${viewIndex} ${chunk}`); await captureYield(); }
  log(`capture-end ${viewIndex}`);
}

/** Let the shell's event loop and the compositor run between capture chunks. */
function captureYield() {
  return new Promise(resolve => setTimeout(resolve, 0));
}

// ---------------------------------------------------------------- frame loop
let ping = 0, frameCount = 0;
let frames = 0, fpsT = performance.now(), fps = 0, gpuMs = 0, timing = false;
let activeProbes = TOTAL_PROBES, lastMoved = 0, relocatedProbes = 0, maxRelocation = 0;
const SIGNAL_READY = true;
let readyDone = false;

const start = performance.now();

const readback = device.createBuffer({ size: TOTAL_PROBES * 16, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
let readbackBusy = false;
async function sampleValidity() {
  if (readbackBusy) return;
  readbackBusy = true;
  const enc = device.createCommandEncoder();
  enc.copyBufferToBuffer(probePosBuf, 0, readback, 0, TOTAL_PROBES * 16);
  device.queue.submit([enc.finish()]);
  await readback.mapAsync(GPUMapMode.READ);
  const v = new Float32Array(readback.getMappedRange().slice(0));
  readback.unmap();
  let active = 0;
  let relocated = 0;
  let maxD = 0;
  // Also measure the virtual-offset/relocation displacement: a probe that gets pulled
  // out of geometry stays valid, so the active count alone cannot show it happening.
  for (let i = 0; i < TOTAL_PROBES; i++) {
    if (v[i * 4 + 3] > 0.5) active++;
    const d = Math.hypot(v[i * 4] - basePos[i * 4], v[i * 4 + 1] - basePos[i * 4 + 1],
      v[i * 4 + 2] - basePos[i * 4 + 2]);
    if (d > 1e-3) { relocated++; if (d > maxD) maxD = d; }
  }
  activeProbes = active;
  relocatedProbes = relocated;
  maxRelocation = maxD;
  readbackBusy = false;
}

let frameErrorLogged = false;

// ------------------------------------------------------- field stability trace (CFG.fieldTrace)
// The scroll wrap is a ONE-FRAME event: a band of probes is reseeded and takes its starting
// value from an inward neighbour, and whatever the next frame does with it decides whether
// the operator sees a step. Nothing in the HUD samples fast enough to catch that, so this
// reads the radiance atlas back every frame and reports how far it moved:
//
//   dMean / dMeanRel  mean |E(t) - E(t-1)| over every texel, absolute and relative to mean E
//   dMax              worst single-texel move
//
// Damped (h = 0.93) a wrap shows as a small bump that decays over ~14 frames; undamped it is
// a step the size of the neighbour's value, and the trace reads as spikes on wrap frames.
let fieldPrev = null, fieldBuf = null, fieldBusy = false;
function f16(u16) {
  const s = (u16 & 0x8000) >> 15, e = (u16 & 0x7c00) >> 10, m = u16 & 0x3ff;
  if (e === 0) return (s ? -1 : 1) * Math.pow(2, -14) * (m / 1024);
  if (e === 31) return m ? NaN : (s ? -1 : 1) * Infinity;
  return (s ? -1 : 1) * Math.pow(2, e - 15) * (1 + m / 1024);
}
async function sampleField(tex, moved) {
  if (fieldBusy) return;
  fieldBusy = true;
  fieldBuf ??= device.createBuffer({
    size: RAD_ATLAS * 8 * RAD_ATLAS,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
  });
  const enc = device.createCommandEncoder();
  enc.copyTextureToBuffer({ texture: tex }, { buffer: fieldBuf, bytesPerRow: RAD_ATLAS * 8 },
    [RAD_ATLAS, RAD_ATLAS]);
  device.queue.submit([enc.finish()]);
  await fieldBuf.mapAsync(GPUMapMode.READ);
  const u16 = new Uint16Array(fieldBuf.getMappedRange().slice(0));
  fieldBuf.unmap();
  const cur = new Float32Array(u16.length / 4 * 3);
  for (let i = 0, o = 0; i < u16.length; i += 4, o += 3) {
    cur[o] = f16(u16[i]); cur[o + 1] = f16(u16[i + 1]); cur[o + 2] = f16(u16[i + 2]);
  }
  let sum = 0;
  for (let i = 0; i < cur.length; i++) sum += cur[i];
  const mean = sum / cur.length;
  // Per-cascade atlas mean: a cascade that is dark in the atlas cannot be fixed in the
  // render, and comparing the three levels is how a cascade-specific tracer fault shows up.
  const perCascade = [0, 1, 2].map(c => {
    let s = 0, n = 0;
    for (let p = c * PER_CASCADE; p < (c + 1) * PER_CASCADE; p++) {
      const sx = p % RAD_SLOTS, sy = Math.floor(p / RAD_SLOTS);
      for (let ty = 0; ty < RAD_INTERIOR; ty++) {
        for (let tx = 0; tx < RAD_INTERIOR; tx++) {
          const off = ((sy * RAD_TILE + 1 + ty) * RAD_ATLAS + (sx * RAD_TILE + 1 + tx)) * 3;
          s += cur[off] + cur[off + 1] + cur[off + 2]; n += 3;
        }
      }
    }
    return (s / n).toFixed(3);
  }).join('/');
  if (fieldPrev !== null) {
    let d = 0, dMax = 0;
    for (let i = 0; i < cur.length; i++) {
      const a = Math.abs(cur[i] - fieldPrev[i]);
      d += a; if (a > dMax) dMax = a;
    }
    d /= cur.length;
    log(`field frame ${frameCount} moved ${moved} mean ${mean.toFixed(3)} casc ${perCascade} ` +
        `dMean ${d.toFixed(4)} dMeanRel ${(d / Math.max(mean, 1e-6)).toFixed(4)} dMax ${dMax.toFixed(3)}`);
  }
  fieldPrev = cur;
  fieldBusy = false;
}
/**
 * Frame scheduling: exactly one requestAnimationFrame per step. The shell drives
 * the rAF queue every native frame (native_game.ts relies on the same), and its
 * queue is a fixed 1024 entries - registering a second callback per frame (e.g.
 * alongside a timer) overflows it and kills the page.
 */
function frame() {
  requestAnimationFrame(frame);
  try {
    frameBody();
  } catch (e) {
    if (!frameErrorLogged) {
      frameErrorLogged = true;
      log('FRAME ERROR: ' + (e && e.stack ? e.stack : e));
    }
  }
}

function frameBody() {
  const t = (performance.now() - start) / 1000;
  const nowT = performance.now();
  const dt = Math.min(0.05, (nowT - lastMoveT) / 1000);
  lastMoveT = nowT;

  // Marker run.sh waits for: the shell's own startup timeout is ~70 s, so an
  // automated run kills it as soon as the capture set is finished (or can no
  // longer happen because input arrived).
  if (captureEnabled && captureIndex >= CAPTURE_FRAMES.length && pendingCaptures === 0) {
    captureEnabled = false;
    log('capture-all-done');
  }

  // Captures are NOT gated on input: each preset sets its own camera pose, so a click or
  // key that arrives before the third frame mark used to silently drop the last view
  // (and the shell's own startup input made the whole capture set flaky).
  const captureDue = captureEnabled &&
    captureIndex < CAPTURE_FRAMES.length && frameCount >= CAPTURE_FRAMES[captureIndex];
  // The presets themselves only apply to a static camera: an orbiting camera keeps its
  // own pose, so its captures fall on three different orbit phases.
  if (captureDue && MOTION === 'static') {
    const v = camPresets[captureIndex % camPresets.length];
    cam.x = v.p[0]; cam.y = v.p[1]; cam.z = v.p[2]; cam.yaw = v.yaw; cam.pitch = v.pitch;
  }
  // Continuous camera motion: an orbit that keeps the field scrolling every frame, so
  // the wrapped-band reseed and the atlas window seams are exercised rather than
  // sampled at three frozen poses. Captures then fall on three different phases.
  if (MOTION === 'orbit') {
    const inside = SCENE === 'complex';
    const radius = inside ? 5.0 : 8.0;
    const a = t * 0.25;
    cam.x = Math.cos(a) * radius;
    cam.z = Math.sin(a) * radius;
    cam.y = inside ? 1.6 : 4.0;
    const tx = 0.0, ty = inside ? 1.5 : 1.0, tz = 0.0;
    const dx = tx - cam.x, dy = ty - cam.y, dz = tz - cam.z;
    cam.yaw = Math.atan2(dx, dz);
    cam.pitch = Math.asin(dy / Math.max(1e-6, Math.hypot(dx, dy, dz)));
  }

  // CFG.autoFly (m/s) drives the camera along +X at a fixed height, so a scroll event - and
  // therefore the wrapped band - lands on a known frame instead of on wherever the operator
  // happened to be holding a key. `t` is the clock, so the path is identical between runs.
  if (CFG.autoFly) {
    const inside = SCENE === 'complex';
    cam.x = (CFG.autoFlyFrom ?? -4.0) + CFG.autoFly * t;
    cam.y = inside ? 1.6 : 3.0;
    cam.z = inside ? 0.0 : 2.0;
    cam.yaw = Math.PI / 2;
    cam.pitch = inside ? -0.15 : -0.2;
  }

  // WASD / arrows fly; Shift sprints; Space rises; C descends.
  const cy = Math.cos(cam.yaw), sy = Math.sin(cam.yaw), cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
  const fwd = [sy * cp, sp, cy * cp];
  const right = [-cy, 0, sy];
  const tNow = performance.now();
  const held = code => keys[code] === true ||
    (tNow - (keyPulse[code] ?? -1e9)) < MIN_HOLD_MS;
  let mx = 0, my = 0, mz = 0;
  if (held('KeyW') || held('ArrowUp')) { mx += fwd[0]; my += fwd[1]; mz += fwd[2]; }
  if (held('KeyS') || held('ArrowDown')) { mx -= fwd[0]; my -= fwd[1]; mz -= fwd[2]; }
  if (held('KeyA') || held('ArrowLeft')) { mx -= right[0]; my -= right[1]; mz -= right[2]; }
  if (held('KeyD') || held('ArrowRight')) { mx += right[0]; my += right[1]; mz += right[2]; }
  if (held('Space') || keys.Space) my += 1;
  if (held('KeyC') || keys.ControlLeft) my -= 1;
  if (keys.ShiftLeft || keys.ShiftRight) sprint = true; else sprint = false;
  const mlen = Math.hypot(mx, my, mz);
  if (mlen > 1e-4) {
    const step = (camSpeed * (sprint ? 3 : 1) * dt) / mlen;
    cam.x += mx * step; cam.y += my * step; cam.z += mz * step;
  }

  const eye = [cam.x, cam.y, cam.z];
  lookAt(eye, [eye[0] + fwd[0], eye[1] + fwd[1], eye[2] + fwd[2]], [0, 1, 0], camData);
  if (frameCount % 240 === 0) {
    // Where does a known world point land on screen? Reveals view/projection bugs.
    const project = (px, py, pz) => {
      const x = px - eye[0], y = py - eye[1], z = pz - eye[2];
      const cx = camData[0] * x + camData[4] * y + camData[8] * z + camData[12];
      const cy = camData[1] * x + camData[5] * y + camData[9] * z + camData[13];
      const cz = camData[2] * x + camData[6] * y + camData[10] * z + camData[14];
      const cw = camData[3] * x + camData[7] * y + camData[11] * z + camData[15];
      if (Math.abs(cw) < 1e-6) return 'w=0';
      return `ndc(${(cx / cw).toFixed(2)}, ${(cy / cw).toFixed(2)}, ${(cz / cw).toFixed(2)})`;
    };
    log(`project cube-center ${project(0, 1, 0)}  cube-top ${project(0, 2, 0)}  ` +
        `ground-front ${project(0, 0, 3)}  eye ${eye.map(v => v.toFixed(1)).join(',')}`);
  }
  camData[16] = eye[0]; camData[17] = eye[1]; camData[18] = eye[2]; camData[19] = 1;
  device.queue.writeBuffer(cameraBuf, 0, camData);

  // CFG.staticLight freezes the animated light: a static camera + static light lets the
  // probe field converge completely, which separates temporal artifacts (scroll bands vs
  // hysteresis lag) from geometric ones.
  const lightPos = lightPosAt(CFG.staticLight ? 0 : t);

  // CFG.scrollDrift (m/s) moves the SCROLL ORIGIN only, leaving the render camera where it
  // is. The wrapped band and the one-frame cascade-origin lag then happen with zero parallax
  // and a frozen light, so two consecutive captures differ by the field change alone - which
  // is the only way to see a one-frame scroll event in a still image.
  lastMoved = updateScroll(CFG.scrollDrift ? [eye[0] + CFG.scrollDrift * t, eye[1], eye[2]] : eye);

  // Params AFTER the scroll. This used to be written before updateScroll, which left every
  // scroll frame rendering a field whose cascade origins were one cell behind the grid the
  // atlas had just been written for: the render selected the 8 probes one cell over, so the
  // scroll frame was shaded from the wrong cages and the NEXT frame snapped back. Frozen
  // camera + frozen light + drifting scroll origin isolate it: consecutive captures differ by
  // 19.7 % and 69 % of full scale on the two wrap frames and by exactly 0 on every other
  // frame. While flying, cascade 0 wraps every metre, i.e. several times a second.
  writeParams(lightPos);
  if (lastMoved > 0) {
    // Synchronous record of every scroll: the field trace below reads its atlas back
    // asynchronously and resolves a few frames later, so it cannot be trusted to pair the
    // delta with the frame that caused it.
    if (CFG.fieldTrace === true) {
      log(`scroll frame ${frameCount} moved ${lastMoved} cam ${cam.x.toFixed(2)} ${cam.y.toFixed(2)} ${cam.z.toFixed(2)}`);
    }
    device.queue.writeBuffer(probePosBuf, 0, basePos);
    device.queue.writeBuffer(probeFlagsBuf, 0, probeFlagsHost);
    device.queue.writeBuffer(seedBuf, 0, seedHost);
  }

  const src = ping, dst = 1 - ping;
  const enc = device.createCommandEncoder();
  // CFG.probeUpdates === false (configs/noupdates.js) leaves the atlas as it is, so the
  // frame cost of the probe passes can be separated from the cost of rendering.
  const runProbeUpdate = CFG.probeUpdates !== false;
  const compute = enc.beginComputePass();
  if (runProbeUpdate) {
    compute.setPipeline(pInit); compute.setBindGroup(0, computeBindGroup);
    compute.dispatchWorkgroups(Math.ceil(TOTAL_PROBES / 64));
    compute.setPipeline(pTrace); compute.setBindGroup(0, computeBindGroup);
    compute.dispatchWorkgroups(Math.ceil(TOTAL_RAYS / 64));
    compute.setPipeline(pBlendDist);
    compute.setBindGroup(0, computeBindGroup);
    compute.setBindGroup(1, device.createBindGroup({
      layout: blendDistLayout,
      entries: [{ binding: 0, resource: distance[src].createView() }, { binding: 1, resource: distance[dst].createView() }],
    }));
    compute.dispatchWorkgroups(TOTAL_PROBES);
    compute.setPipeline(pBlendRad);
    compute.setBindGroup(0, computeBindGroup);
    compute.setBindGroup(1, device.createBindGroup({
      layout: blendRadLayout,
      entries: [{ binding: 0, resource: radiance[src].createView() }, { binding: 1, resource: radiance[dst].createView() }],
    }));
    compute.dispatchWorkgroups(TOTAL_PROBES);

  }
  compute.end();

  ensureDepth();
  const canvasTex = ctx.getCurrentTexture();
  const pass = enc.beginRenderPass({
    colorAttachments: [{
      view: canvasTex.createView(),
      clearValue: { r: 0.02, g: 0.02, b: 0.03, a: 1 },
      loadOp: 'clear', storeOp: 'store',
    }],
    depthStencilAttachment: {
      view: depthTexture.createView(),
      depthClearValue: 1.0,
      depthLoadOp: 'clear',
      depthStoreOp: 'store',
    },
  });
  pass.setPipeline(renderPipeline);
  pass.setVertexBuffer(0, vertexBuf);
  pass.setBindGroup(0, renderBinds[dst]);
  pass.setBindGroup(1, renderBind1);
  pass.draw(scene.triCount * 3);
  pass.end();

  let captureBuffer = null, captureShape = null;
  if (captureDue) {
    const w = canvas.width, h = canvas.height;
    const bytesPerRow = Math.ceil(w * 4 / 256) * 256;
    captureBuffer = device.createBuffer({ size: bytesPerRow * h, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
    captureShape = [w, h, bytesPerRow];
    enc.copyTextureToBuffer({ texture: canvasTex }, { buffer: captureBuffer, bytesPerRow }, [w, h]);
    captureIndex++;
  }
  device.queue.submit([enc.finish()]);
  if (CFG.fieldTrace === true) void sampleField(radiance[dst], lastMoved);
  if (captureBuffer) {
    pendingCaptures++;
    void readbackFrame(captureBuffer, captureShape[0], captureShape[1], captureShape[2], captureIndex - 1)
      .catch(e => { log('capture failed: ' + (e && e.message ? e.message : e)); })
      .finally(() => { pendingCaptures--; });
  }

  // Startup handshake, exactly as crates/afterglow-shell/native_game.ts does it.
  // The browser-document sync is what connects the native canvas node (without it
  // present fails with "native canvas N is not connected"), and the readiness op is
  // what opens the shell's input gate (dispatch_input drops events until then).
  // SIGNAL_READY=false: presentation works without it, and the shell's post-ready
  // surface re-sync leaves a raw page's context stale (the next present fails with
  // "surface has no acquired texture" and the composited frame is blank). Input is
  // gated on readiness, so a raw page can render OR accept input - the engine
  // runtime is what owns both.
  if (SIGNAL_READY && !readyDone) {
    readyDone = true;
    const ops = globalThis.Deno?.core?.ops;
    if (ops?.op_engine_ready) {
      try {
        globalThis.__syncBrowserDocument?.(false);
        ops.op_present_surface?.();
        ops.op_engine_ready();
        // Do NOT reconfigure here: it would drop the texture this frame already
        // acquired, the first present would fail, and the shell would never request
        // another redraw (which is what keeps our loop alive).
        log('runtime ready - real input is live');
      } catch (e) {
        log('ready signal failed: ' + e);
      }
    }
  }

  ping = dst;
  // We own the loop, so we also own presentation: the shell stops presenting once
  // readiness is signalled. Its HUD composite runs as part of this call.
  if (!timing) {
    timing = true;
    const t0 = performance.now();
    device.queue.onSubmittedWorkDone().then(() => { gpuMs = performance.now() - t0; timing = false; });
  }
  if ((frameCount & 127) === 0) void sampleValidity();
  if (frameCount % 240 === 0) {
    log(`frame ${frameCount}  ${fps.toFixed(1)} fps  gpu ${gpuMs.toFixed(2)} ms  ` +
        `active probes ${activeProbes}/${TOTAL_PROBES}  ` +
        `relocated ${relocatedProbes} (max ${maxRelocation.toFixed(2)} m)  ` +
        `reseeded ${lastMoved}  pos ${cam.x.toFixed(1)} ${cam.y.toFixed(1)} ${cam.z.toFixed(1)}  ` +
        `lastKey ${lastKey}  held [${['KeyW','KeyA','KeyS','KeyD','Space','ShiftLeft'].filter(c => keys[c] === true).join(',')}]`);
  }
  frames++;
  const now = performance.now();
  if (now - fpsT > 500) {
    fps = frames * 1000 / (now - fpsT);
    frames = 0; fpsT = now;
    hud.textContent =
      `${fps.toFixed(1)} FPS   ${TOTAL_PROBES} probes   ${TOTAL_RAYS} rays/sweep\n` +
      `pos ${cam.x.toFixed(2)} ${cam.y.toFixed(2)} ${cam.z.toFixed(2)}   speed ${camSpeed.toFixed(1)} m/s\n` +
      `active ${activeProbes}/${TOTAL_PROBES}   last key ${lastKey}`;
  }
  frameCount++;
}

// One-shot GPU-vs-CPU check of the BVH data path: the shader reads node metadata
// through array<vec4f> + bitcast, so confirm the GPU sees the same integers JS built.
async function verifyNodeData() {
  const dumpCode = `
@group(0) @binding(0) var<storage, read> nodes: array<vec4f>;
@group(0) @binding(1) var<storage, read> rights: array<i32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;
@group(0) @binding(3) var<storage, read> triIdx: array<u32>;
@group(0) @binding(4) var<storage, read> triPos: array<vec4f>;
@group(0) @binding(5) var<storage, read> vtx: array<f32>;

fn tSlab(o: vec3f, invD: vec3f, bmin: vec3f, bmax: vec3f) -> vec2f {
  let t0 = (bmin - o) * invD;
  let t1 = (bmax - o) * invD;
  return vec2f(max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z)),
               min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z)));
}
fn tHit(o: vec3f, d: vec3f, i: u32, tMax: f32) -> f32 {
  let a = triPos[i * 3u].xyz; let b = triPos[i * 3u + 1u].xyz; let c = triPos[i * 3u + 2u].xyz;
  let e1 = b - a; let e2 = c - a;
  let p = cross(d, e2); let det = dot(e1, p);
  if (abs(det) < 1e-12) { return -1.0; }
  let inv = 1.0 / det; let tv = o - a;
  let u = dot(tv, p) * inv;
  if (u < 0.0 || u > 1.0) { return -1.0; }
  let q = cross(tv, e1); let v = dot(d, q) * inv;
  if (v < 0.0 || u + v > 1.0) { return -1.0; }
  let t = dot(e2, q) * inv;
  if (t < 0.001 || t > tMax) { return -1.0; }
  return t;
}
/** First hit distance for one ray, mirroring the shader's shadow traversal. */
fn tFirst(o: vec3f, d: vec3f, tMax: f32) -> vec2f {
  var stack: array<u32, 40>;
  var sp = 0u;
  let invD = 1.0 / d;
  stack[sp] = 0u; sp = 1u;
  loop {
    if (sp == 0u) { break; }
    sp = sp - 1u;
    let ni = stack[sp];
    let nMin = nodes[ni * 2u].xyz;
    let leftFirst = bitcast<u32>(nodes[ni * 2u].w);
    let nMax = nodes[ni * 2u + 1u].xyz;
    let triCount = bitcast<u32>(nodes[ni * 2u + 1u].w);
    let tt = tSlab(o, invD, nMin, nMax);
    if (tt.x > tt.y || tt.y < 0.001 || tt.x > tMax) { continue; }
    if (triCount > 0u) {
      var k = 0u;
      loop {
        if (k >= triCount) { break; }
        let tri = triIdx[leftFirst + k];
        let h = tHit(o, d, tri, tMax);
        if (h > 0.0) { return vec2f(h, f32(tri)); }
        k = k + 1u;
      }
    } else {
      stack[sp] = u32(rights[ni]); sp = sp + 1u;
      stack[sp] = leftFirst; sp = sp + 1u;
    }
  }
  return vec2f(-1.0, -1.0);
}

@compute @workgroup_size(1)
fn dump() {
  for (var i = 0u; i < 8u; i = i + 1u) {
    out[i * 4u + 0u] = bitcast<u32>(nodes[i * 2u].w);        // leftFirst
    out[i * 4u + 1u] = bitcast<u32>(nodes[i * 2u + 1u].w);  // triCount
    out[i * 4u + 2u] = u32(rights[i]);
    out[i * 4u + 3u] = u32(nodes[i * 2u].x);                // node bounds min.x (float bits)
  }
  // raw vertex-buffer dump: 240 floats (triangles 0..7), at an offset that does
  // not collide with the ray dump below (out[32..43]).
  for (var i = 0u; i < 240u; i = i + 1u) {
    out[80u + i] = bitcast<u32>(vtx[i]);
  }
  // three probe rays with CPU-known answers (see shadowcheck.mjs)
  let light = vec3f(3.0, 4.0, 3.0);
  let rays = array<vec4f, 3>(
    vec4f(0.0, 0.02, 3.0, 0.0),    // ground toward light  -> expect clear
    vec4f(1.02, 1.0, 0.0, 0.0),    // cube +X face         -> expect clear
    vec4f(0.0, 0.02, -3.0, 0.0),   // ground far side      -> expect blocked at ~2.60
  );
  for (var r = 0u; r < 3u; r = r + 1u) {
    let o = rays[r].xyz;
    let toL = light - o;
    let dist = length(toL);
    let d = toL / dist;
    let hit = tFirst(o, d, dist - 0.05);
    out[32u + r * 4u + 0u] = bitcast<u32>(hit.x);   // t
    out[32u + r * 4u + 1u] = bitcast<u32>(hit.y);   // tri index
    out[32u + r * 4u + 2u] = bitcast<u32>(dist);
    out[32u + r * 4u + 3u] = 0u;
  }
}`;
  const mod = device.createShaderModule({ code: dumpCode });
  const out = device.createBuffer({ size: 400 * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC });
  const rb = device.createBuffer({ size: 400 * 4, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
  const bgl = device.createBindGroupLayout({ entries: [
    { binding: 0, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 1, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 2, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'storage' } },
    { binding: 3, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 4, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 5, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
  ]});
  const pipe = device.createComputePipeline({
    layout: device.createPipelineLayout({ bindGroupLayouts: [bgl] }),
    compute: { module: mod, entryPoint: 'dump' },
  });
  const bg = device.createBindGroup({ layout: bgl, entries: [
    { binding: 0, resource: { buffer: nodeBuf } },
    { binding: 1, resource: { buffer: rightsBuf } },
    { binding: 2, resource: { buffer: out } },
    { binding: 3, resource: { buffer: idxBuf } },
    { binding: 4, resource: { buffer: triBuf } },
    { binding: 5, resource: { buffer: vertexBuf } },
  ]});
  const enc = device.createCommandEncoder();
  const cp = enc.beginComputePass();
  cp.setPipeline(pipe); cp.setBindGroup(0, bg); cp.dispatchWorkgroups(1); cp.end();
  enc.copyBufferToBuffer(out, 0, rb, 0, 400 * 4);
  device.queue.submit([enc.finish()]);
  await rb.mapAsync(GPUMapMode.READ);
  const gpu = new Uint32Array(rb.getMappedRange().slice(0));
  rb.unmap();
  let mismatches = 0;
  for (let i = 0; i < 8; i++) {
    const wantLf = bvh.nodeU32[i * 8 + 3], wantTc = bvh.nodeU32[i * 8 + 7];
    const wantHmmRight = bvh.rights[i];
    const ok = gpu[i * 4] === wantLf && gpu[i * 4 + 1] === wantTc && gpu[i * 4 + 2] === (wantHmmRight >>> 0);
    if (!ok) {
      mismatches++;
      if (mismatches <= 3) {
        log(`node ${i}: GPU leftFirst=${gpu[i * 4]} triCount=${gpu[i * 4 + 1]} right=${gpu[i * 4 + 2]} ` +
            `| JS leftFirst=${wantLf} triCount=${wantTc} right=${wantHmmRight >>> 0}`);
      }
    }
  }
  log(`node data path: ${8 - mismatches}/8 nodes match between GPU and JS`);
  const f32 = new Float32Array(gpu.buffer.slice(0));
  let firstBad = -1;
  for (let i = 0; i < 240; i++) {
    if (Math.abs(f32[80 + i] - vertexUpload[i]) > 1e-6) { firstBad = i; break; }
  }
  log(`vertex buffer: ${firstBad < 0 ? 'all 240 floats match' : 'first mismatch at float ' + firstBad + ' (tri ' + Math.floor(firstBad / 30) + ')'}`);
  if (firstBad >= 0) {
    log(`  host[${firstBad}..${firstBad + 5}] = ${Array.from(vertexUpload.slice(firstBad, firstBad + 6)).map(v => v.toFixed(2)).join(',')}`);
    log(`  gpu [${firstBad}..${firstBad + 5}] = ${Array.from(f32.slice(80 + firstBad, 80 + firstBad + 6)).map(v => v.toFixed(2)).join(',')}`);
  }
  log(`  triCount=${scene.triCount} vertexData.length=${vertexData.length} (expect ${scene.triCount * 3 * 9})`);
  log(`  aliasing: vertexUpload.buffer===basePos.buffer? ${vertexUpload.buffer === basePos.buffer}` +
      `  basePos[0..3]=${Array.from(basePos.slice(0, 4)).map(v => v.toFixed(1)).join(',')}` +
      `  probePosBuf.size=${probePosBuf.size} vertexBuf.size=${vertexBuf.size}`);
  for (let r = 0; r < 3; r++) {
    log(`shadow ray ${r}: gpu t=${f32[32 + r * 4].toFixed(3)} tri=${gpu[32 + r * 4 + 1]} ` +
        `(lightDist ${f32[32 + r * 4 + 2].toFixed(2)})`);
  }
}

log(`field: ${CASCADES} cascades x ${PER_CASCADE} = ${TOTAL_PROBES} probes, ${RAYS_PER_PROBE} rays/probe = ${TOTAL_RAYS} rays/sweep`);
// One-shot movement check: hold W through the shell's own dispatcher for 30 frames
// and report the camera displacement and the frame interval distribution, so
// "keys register but the camera does not move" is a measurement rather than a guess.
let moveTest = 0, moveTestFrom = null, moveTestTimes = [], tapFrom = null;
function movementCheck() {
  const dispatch = globalThis.__dispatchBrowserKeyboardEvent;
  if (typeof dispatch !== 'function') { log('movement check skipped: no dispatcher'); return; }
  moveTest++;
  moveTestTimes.push(performance.now());
  if (moveTest === 10) {
    moveTestFrom = [cam.x, cam.y, cam.z];
    dispatch('keydown', { key: 'w', code: 'KeyW' });
  }
  if (moveTest === 42) {
    dispatch('keyup', { key: 'w', code: 'KeyW' });
    const moved = Math.hypot(cam.x - moveTestFrom[0], cam.y - moveTestFrom[1], cam.z - moveTestFrom[2]);
    const gaps = [];
    for (let i = 1; i < moveTestTimes.length; i++) gaps.push(moveTestTimes[i] - moveTestTimes[i - 1]);
    gaps.sort((a, b) => a - b);
    log(`movement check (hold): ${moved.toFixed(2)} m over 32 frames; rAF gap median ` +
        `${gaps[gaps.length >> 1].toFixed(2)} ms, max ${gaps[gaps.length - 1].toFixed(2)} ms`);
    tapFrom = [cam.x, cam.y, cam.z];
  }
  if (moveTest === 60) {   // a tap: press and release one frame apart
    dispatch('keydown', { key: 'w', code: 'KeyW' });
    dispatch('keyup', { key: 'w', code: 'KeyW' });
  }
  if (moveTest === 80) {
    const tapped = Math.hypot(cam.x - tapFrom[0], cam.y - tapFrom[1], cam.z - tapFrom[2]);
    log(`movement check (tap): ${tapped.toFixed(2)} m from a single press+release`);
    return;
  }
  if (moveTest < 85) requestAnimationFrame(movementCheck);
}
setTimeout(() => requestAnimationFrame(movementCheck), 1500);
void verifyNodeData();
requestAnimationFrame(frame);
