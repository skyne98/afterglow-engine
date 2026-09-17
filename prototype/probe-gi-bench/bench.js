// Probe-GI BVH traversal benchmark — raw WebGPU compute, no Three.
// Measures rays/ms for a closest-hit BVH traversal shaped like a DDGI probe
// update (P probes x 64 rays, spherical-Fibonacci directions), with and without
// a hit shading evaluation. Validates the BVH + traversal against a CPU
// brute-force reference before timing anything.

const out = document.getElementById('out');
const lines = [];
function say(s) { lines.push(s); out.textContent = lines.join('\n'); console.log(s); }

const RAYS_PER_PROBE = 64;
const MAX_LEAF = 4;

// ---------------------------------------------------------------- scene
// Axis-aligned boxes on a cubic lattice => "walls/rooms". 12 tris per box.
function buildScene(boxCount) {
  const side = Math.max(2, Math.round(Math.cbrt(boxCount)));
  const boxes = [];
  const cell = 4.0;
  let n = 0;
  for (let z = 0; z < side && n < boxCount; z++)
    for (let y = 0; y < side && n < boxCount; y++)
      for (let x = 0; x < side && n < boxCount; x++, n++) {
        // slabs: thin walls so rays travel and actually traverse the BVH
        const thin = (x + y + z) % 3 === 0;
        const ex = thin ? 0.25 : 1.4, ey = thin ? 1.4 : 0.25, ez = thin ? 1.4 : 1.4;
        boxes.push([x * cell, y * cell, z * cell, ex, ey, ez]);
      }
  const tris = new Float32Array(boxes.length * 12 * 9);
  let o = 0;
  const corner = (b, i) => [
    b[0] + (i & 1 ? b[3] : -b[3]),
    b[1] + (i & 2 ? b[4] : -b[4]),
    b[2] + (i & 4 ? b[5] : -b[5]),
  ];
  const FACES = [[0, 2, 6, 4], [1, 3, 7, 5], [0, 1, 5, 4], [2, 3, 7, 6], [0, 1, 3, 2], [4, 5, 7, 6]];
  for (const b of boxes)
    for (const f of FACES) {
      const q = f.map(i => corner(b, i));
      const tri = (a, c, d) => {
        for (const v of [a, c, d]) { tris[o++] = v[0]; tris[o++] = v[1]; tris[o++] = v[2]; }
      };
      tri(q[0], q[1], q[2]); tri(q[0], q[2], q[3]);
    }
  const sceneMin = [Infinity, Infinity, Infinity], sceneMax = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < o; i += 3)
    for (let a = 0; a < 3; a++) {
      if (tris[i + a] < sceneMin[a]) sceneMin[a] = tris[i + a];
      if (tris[i + a] > sceneMax[a]) sceneMax[a] = tris[i + a];
    }
  return { tris: tris.subarray(0, o), triCount: o / 9, sceneMin, sceneMax };
}

// ---------------------------------------------------------------- BVH build
// Median split on the longest centroid axis. Returns flat node arrays.
function buildBVH(tris, triCount) {
  const triMin = new Float32Array(triCount * 3);
  const triMax = new Float32Array(triCount * 3);
  const cent = new Float32Array(triCount * 3);
  for (let i = 0; i < triCount; i++) {
    const o = i * 9;
    for (let a = 0; a < 3; a++) {
      let mn = Infinity, mx = -Infinity;
      for (let v = 0; v < 3; v++) {
        const c = tris[o + v * 3 + a];
        if (c < mn) mn = c;
        if (c > mx) mx = c;
      }
      triMin[i * 3 + a] = mn; triMax[i * 3 + a] = mx; cent[i * 3 + a] = (mn + mx) * 0.5;
    }
  }
  const index = new Uint32Array(triCount);
  for (let i = 0; i < triCount; i++) index[i] = i;

  const nmin = [], nmax = [], nleft = [], ncount = [], nright = [];
  const boundsOf = (start, end) => {
    const mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
    for (let i = start; i < end; i++) {
      const t = index[i];
      for (let a = 0; a < 3; a++) {
        if (triMin[t * 3 + a] < mn[a]) mn[a] = triMin[t * 3 + a];
        if (triMax[t * 3 + a] > mx[a]) mx[a] = triMax[t * 3 + a];
      }
    }
    return [mn, mx];
  };

  function build(start, end) {
    const [mn, mx] = boundsOf(start, end);
    const self = nmin.length;
    nmin.push(mn); nmax.push(mx); nleft.push(0); ncount.push(0); nright.push(0);
    const count = end - start;
    if (count <= MAX_LEAF) {
      nleft[self] = start; ncount[self] = count;
      return self;
    }
    let axis = 0, ext = mx[0] - mn[0];
    for (let a = 1; a < 3; a++) if (mx[a] - mn[a] > ext) { ext = mx[a] - mn[a]; axis = a; }
    const sub = Array.from(index.subarray(start, end));
    sub.sort((a, b) => cent[a * 3 + axis] - cent[b * 3 + axis]);
    index.set(sub, start);
    const mid = (start + end) >> 1;
    const left = build(start, mid);
    const right = build(mid, end);
    nleft[self] = left; ncount[self] = 0; nright[self] = right;
    return self;
  }
  build(0, triCount);

  const nodeCount = nmin.length;
  const nodes = new Float32Array(nodeCount * 12);
  for (let i = 0; i < nodeCount; i++) {
    const o = i * 12;
    nodes[o] = nmin[i][0]; nodes[o + 1] = nmin[i][1]; nodes[o + 2] = nmin[i][2];
    nodes[o + 3] = nleft[i];
    nodes[o + 4] = nmax[i][0]; nodes[o + 5] = nmax[i][1]; nodes[o + 6] = nmax[i][2];
    nodes[o + 7] = ncount[i];
    nodes[o + 8] = nright[i];
  }
  const order = new Uint32Array(triCount);
  for (let i = 0; i < triCount; i++) order[i] = index[i];
  return { nodes, nodeCount, order };
}

// CPU brute-force reference (Möller–Trumbore over every triangle).
function bruteHit(tris, triCount, o, d) {
  let best = Infinity;
  for (let i = 0; i < triCount; i++) {
    const p = i * 9;
    const ax = tris[p], ay = tris[p + 1], az = tris[p + 2];
    const bx = tris[p + 3], by = tris[p + 4], bz = tris[p + 5];
    const cx = tris[p + 6], cy = tris[p + 7], cz = tris[p + 8];
    const e1x = bx - ax, e1y = by - ay, e1z = bz - az;
    const e2x = cx - ax, e2y = cy - ay, e2z = cz - az;
    const px = d[1] * e2z - d[2] * e2y, py = d[2] * e2x - d[0] * e2z, pz = d[0] * e2y - d[1] * e2x;
    const det = e1x * px + e1y * py + e1z * pz;
    if (Math.abs(det) < 1e-12) continue;
    const inv = 1 / det;
    const tx = o[0] - ax, ty = o[1] - ay, tz = o[2] - az;
    const u = (tx * px + ty * py + tz * pz) * inv;
    if (u < 0 || u > 1) continue;
    const qx = ty * e1z - tz * e1y, qy = tz * e1x - tx * e1z, qz = tx * e1y - ty * e1x;
    const v = (d[0] * qx + d[1] * qy + d[2] * qz) * inv;
    if (v < 0 || u + v > 1) continue;
    const t = (e2x * qx + e2y * qy + e2z * qz) * inv;
    if (t > 0.001 && t < best) best = t;
  }
  return best;
}

// ---------------------------------------------------------------- WGSL
const WGSL = `
struct Node {
  minx: f32, miny: f32, minz: f32, leftFirst: u32,
  maxx: f32, maxy: f32, maxz: f32, triCount: u32,
  rightChild: u32, pad0: u32, pad1: u32, pad2: u32,
};
struct Params {
  originSpacing: vec4f,   // xyz origin, w spacing
  counts: vec4u,          // xyz probe counts, w probes
  misc: vec4u,            // x raysPerProbe, y shade, z totalRays, w triCount
};
@group(0) @binding(0) var<storage, read> nodes: array<Node>;
@group(0) @binding(1) var<storage, read> triPos: array<vec4f>;
@group(0) @binding(2) var<storage, read> triIdx: array<u32>;
@group(0) @binding(3) var<storage, read_write> outT: array<f32>;
@group(0) @binding(4) var<storage, read_write> outTri: array<u32>;
@group(0) @binding(5) var<storage, read_write> outShade: array<vec4f>;
@group(0) @binding(6) var<uniform> params: Params;

fn slab(o: vec3f, invD: vec3f, bmin: vec3f, bmax: vec3f) -> vec2f {
  let t0 = (bmin - o) * invD;
  let t1 = (bmax - o) * invD;
  let tmin = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
  let tmax = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
  return vec2f(tmin, tmax);
}

fn hitTri(o: vec3f, d: vec3f, i: u32, tMax: f32) -> f32 {
  let a = triPos[i * 3u].xyz;
  let b = triPos[i * 3u + 1u].xyz;
  let c = triPos[i * 3u + 2u].xyz;
  let e1 = b - a;
  let e2 = c - a;
  let p = cross(d, e2);
  let det = dot(e1, p);
  if (abs(det) < 1e-12) { return -1.0; }
  let inv = 1.0 / det;
  let tv = o - a;
  let u = dot(tv, p) * inv;
  if (u < 0.0 || u > 1.0) { return -1.0; }
  let q = cross(tv, e1);
  let v = dot(d, q) * inv;
  if (v < 0.0 || u + v > 1.0) { return -1.0; }
  let t = dot(e2, q) * inv;
  if (t < 0.001 || t > tMax) { return -1.0; }
  return t;
}

@compute @workgroup_size(64)
fn traverse(@builtin(global_invocation_id) gid: vec3u) {
  let ray = gid.x;
  let totalRays = params.misc.z;
  if (ray >= totalRays) { return; }

  let raysPerProbe = params.misc.x;
  let probe = ray / raysPerProbe;
  let r = ray % raysPerProbe;

  let counts = params.counts.xyz;
  let plane = counts.x * counts.y;
  let px = f32(probe % counts.x);
  let py = f32((probe / counts.x) % counts.y);
  let pz = f32(probe / plane);
  let span = vec3f(counts) * params.originSpacing.w;
  let origin = params.originSpacing.xyz + vec3f(px, py, pz) * params.originSpacing.w - span * 0.5;

  // Spherical Fibonacci directions (no RNG state).
  let i = f32(r) + 0.5;
  let n = f32(raysPerProbe);
  let cosTheta = 1.0 - 2.0 * i / n;
  let theta = 6.283185307179586 * i * 0.6180339887498949;
  let sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
  let d = vec3f(cos(theta) * sinTheta, sin(theta) * sinTheta, cosTheta);

  let o = origin;
  let invD = 1.0 / d;
  var stack: array<u32, 48>;
  var sp = 0u;
  var bestT = 1e30;
  var bestTri = 0xffffffffu;
  stack[sp] = 0u;
  sp = 1u;

  loop {
    if (sp == 0u) { break; }
    sp = sp - 1u;
    let ni = stack[sp];
    let node = nodes[ni];
    let tt = slab(o, invD, vec3f(node.minx, node.miny, node.minz), vec3f(node.maxx, node.maxy, node.maxz));
    if (tt.x <= tt.y && tt.y >= 0.001 && tt.x <= bestT) {
      if (node.triCount == 0u) {
        stack[sp] = node.rightChild;
        sp = sp + 1u;
        stack[sp] = node.leftFirst;
        sp = sp + 1u;
      } else {
        var k = 0u;
        loop {
          if (k >= node.triCount) { break; }
          let t = triIdx[node.leftFirst + k];
          let h = hitTri(o, d, t, bestT);
          if (h > 0.0 && h < bestT) { bestT = h; bestTri = t; }
          k = k + 1u;
        }
      }
    }
  }

  outT[ray] = select(1e30, bestT, bestTri != 0xffffffffu);
  outTri[ray] = bestTri;

  if (params.misc.y == 1u && bestTri != 0xffffffffu) {
    // Hit shading: the part of a real GI ray the traversal-only number omits.
    let a = triPos[bestTri * 3u].xyz;
    let b = triPos[bestTri * 3u + 1u].xyz;
    let c = triPos[bestTri * 3u + 2u].xyz;
    let nrm = normalize(cross(b - a, c - a));
    let l1 = normalize(vec3f(0.4, 0.9, 0.3));
    let l2 = normalize(vec3f(-0.6, 0.2, -0.5));
    var e = vec3f(0.0);
    e = e + vec3f(1.0, 0.95, 0.85) * max(dot(nrm, l1), 0.0);
    e = e + vec3f(0.25, 0.30, 0.45) * max(dot(nrm, l2), 0.0);
    let w = max(dot(d, nrm), 0.0);
    let albedo = vec3f(0.35 + 0.4 * fract(f32(probe) * 0.017), 0.32, 0.28);
    outShade[ray] = vec4f(e * albedo * w, bestT);
  } else {
    outShade[ray] = vec4f(0.0);
  }
}
`;

// ---------------------------------------------------------------- main
async function run() {
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) { say('NO ADAPTER'); return; }
  const info = adapter.info ?? {};
  const device = await adapter.requestDevice();
  const lim = device.limits;
  say(`adapter: ${info.description ?? '?'} vendor=${info.vendor ?? '?'} device=${info.device ?? '?'}`);
  say(`limits: maxStorageBufferBindingSize=${lim.maxStorageBufferBindingSize} maxComputeInvocationsPerWorkgroup=${lim.maxComputeInvocationsPerWorkgroup} maxStorageBuffersPerShaderStage=${lim.maxStorageBuffersPerShaderStage}`);
  say(`timestamp-query feature: ${adapter.features.has('timestamp-query')}`);

  const module = device.createShaderModule({ code: WGSL });
  let info2 = { messages: [] };
  try { info2 = await module.getCompilationInfo(); } catch (e) { say('getCompilationInfo unavailable: ' + e); }
  const errs = (info2.messages || []).filter(m => m.type === 'error');
  if (errs.length) { say('WGSL ERRORS:\n' + errs.map(m => `${m.lineNum}:${m.linePos} ${m.message}`).join('\n')); return; }
  say(`WGSL compiled (${info2.messages.length} messages)`);

  const pipeline = device.createComputePipeline({
    layout: 'auto',
    compute: { module, entryPoint: 'traverse' },
  });

  const CONFIGS = [];
  for (const boxes of [2048, 8192])
    for (const probes of [2048, 8192])
      for (const shade of [0, 1])
        CONFIGS.push({ boxes, probes, shade });

  say('');
  say('boxes  tris    nodes   probes  rays      mode       ms/sweep  rays/ms   req@1/8');
  let validated = false;

  for (const cfg of CONFIGS) {
    const scene = buildScene(cfg.boxes);
    const bvh = buildBVH(scene.tris, scene.triCount);

    const triPos = new Float32Array(scene.triCount * 12);
    for (let i = 0; i < scene.triCount; i++)
      for (let v = 0; v < 3; v++)
        for (let a = 0; a < 3; a++)
          triPos[i * 12 + v * 4 + a] = scene.tris[i * 9 + v * 3 + a];

    const totalRays = cfg.probes * RAYS_PER_PROBE;
    const side = Math.max(2, Math.round(Math.cbrt(cfg.probes)));
    const counts = [side, Math.max(1, Math.floor(side / 2)), side];
    while (counts[0] * counts[1] * counts[2] < cfg.probes) counts[1]++;
    const spacing = 4.0;
    const center = [
      (counts[0] - 1) * spacing * 0.5,
      (counts[1] - 1) * spacing * 0.5,
      (counts[2] - 1) * spacing * 0.5,
    ];

    const mk = (data, usage) => {
      const b = device.createBuffer({ size: Math.max(16, data.byteLength), usage, mappedAtCreation: true });
      const dst = new (data.constructor)(b.getMappedRange());
      dst.set(data);
      b.unmap();
      return b;
    };
    const SS = GPUBufferUsage.STORAGE, UNI = GPUBufferUsage.UNIFORM;
    const nodeBuf = mk(bvh.nodes, SS);
    const triBuf = mk(triPos, SS);
    const idxBuf = mk(bvh.order, SS);
    const outT = device.createBuffer({ size: totalRays * 4, usage: SS | GPUBufferUsage.COPY_SRC });
    const outTri = device.createBuffer({ size: totalRays * 4, usage: SS | GPUBufferUsage.COPY_SRC });
    const outShade = device.createBuffer({ size: totalRays * 16, usage: SS | GPUBufferUsage.COPY_SRC });
    const params = new ArrayBuffer(48);
    {
      const f = new Float32Array(params), u = new Uint32Array(params);
      f[0] = center[0]; f[1] = center[1]; f[2] = center[2]; f[3] = spacing;
      u[4] = counts[0]; u[5] = counts[1]; u[6] = counts[2]; u[7] = cfg.probes;
      u[8] = RAYS_PER_PROBE; u[9] = cfg.shade; u[10] = totalRays; u[11] = scene.triCount;
    }
    const paramBuf = mk(new Uint8Array(params), UNI);

    const bind = device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: nodeBuf } },
        { binding: 1, resource: { buffer: triBuf } },
        { binding: 2, resource: { buffer: idxBuf } },
        { binding: 3, resource: { buffer: outT } },
        { binding: 4, resource: { buffer: outTri } },
        { binding: 5, resource: { buffer: outShade } },
        { binding: 6, resource: { buffer: paramBuf } },
      ],
    });

    const dispatch = () => {
      const enc = device.createCommandEncoder();
      const pass = enc.beginComputePass();
      pass.setPipeline(pipeline);
      pass.setBindGroup(0, bind);
      pass.dispatchWorkgroups(Math.ceil(totalRays / 64));
      pass.end();
      device.queue.submit([enc.finish()]);
    };

    // ---- validation once, on the smallest config, vs CPU brute force
    if (!validated && cfg.probes === 512 && cfg.shade === 0) {
      dispatch();
      await device.queue.onSubmittedWorkDone();
      const readback = device.createBuffer({ size: totalRays * 4, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
      const enc = device.createCommandEncoder();
      enc.copyBufferToBuffer(outT, 0, readback, 0, totalRays * 4);
      device.queue.submit([enc.finish()]);
      await readback.mapAsync(GPUMapMode.READ);
      const gpuT = new Float32Array(readback.getMappedRange().slice(0));
      readback.unmap();

      let bad = 0, checked = 0;
      const plane = counts[0] * counts[1];
      for (let ray = 0; ray < totalRays; ray += 977) {
        const probe = (ray / RAYS_PER_PROBE) | 0, r = ray % RAYS_PER_PROBE;
        const px = probe % counts[0], py = ((probe / counts[0]) | 0) % counts[1], pz = (probe / plane) | 0;
        const o = [center[0] + px * spacing - counts[0] * spacing * 0.5,
                   center[1] + py * spacing - counts[1] * spacing * 0.5,
                   center[2] + pz * spacing - counts[2] * spacing * 0.5];
        const i = r + 0.5, n = RAYS_PER_PROBE;
        const ct = 1 - 2 * i / n, th = 2 * Math.PI * i * 0.6180339887498949;
        const st = Math.sqrt(Math.max(0, 1 - ct * ct));
        const d = [Math.cos(th) * st, Math.sin(th) * st, ct];
        const ref = bruteHit(scene.tris, scene.triCount, o, d);
        const got = gpuT[ray];
        const a = Math.min(ref, 1e29), b = Math.min(got, 1e29);
        checked++;
        if (!(Math.abs(a - b) <= 1e-3 + 1e-3 * Math.abs(a))) bad++;
      }
      say(`validation: ${checked - bad}/${checked} rays match CPU brute force (tolerance 1e-3 relative)`);
      validated = true;
    }

    const iters = 25;
    dispatch(); // warm
    await device.queue.onSubmittedWorkDone();
    const t0 = performance.now();
    for (let i = 0; i < iters; i++) dispatch();
    await device.queue.onSubmittedWorkDone();
    const ms = (performance.now() - t0) / iters;

    const raysPerMs = totalRays / ms;
    say(`${String(cfg.boxes).padStart(5)}  ${String(scene.triCount).padStart(6)}  ${String(bvh.nodeCount).padStart(6)}  ${String(cfg.probes).padStart(6)}  ${String(totalRays).padStart(8)}  ${(cfg.shade ? 'shade' : 'trace').padEnd(8)}  ${ms.toFixed(3).padStart(8)}  ${raysPerMs.toFixed(0).padStart(8)}  ${(ms / 8).toFixed(3).padStart(7)}ms`);

    for (const b of [nodeBuf, triBuf, idxBuf, outT, outTri, outShade, paramBuf]) b.destroy();
  }

  say('');
  say('req@1/8 = per-frame cost if 1/8 of the volume updates each frame');
  say('DONE');
}

run().catch(e => {
  say('ERROR: ' + String(e && e.stack || e));
  console.log('ERROR: ' + String(e && e.stack || e));
});
