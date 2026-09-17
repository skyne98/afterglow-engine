// Demo scene + binary BVH builder for the probe-GI demo.
// Ported from prototype/probe-gi-bench (traversal validated against a CPU
// brute-force reference there). The shipped engine design calls for CWBVH8
// (80-byte compressed nodes, see docs/research/probe-gi-placement-leaks-packing.md
// §C.1); this demo uses a plain binary BVH with the same traversal shape, which
// the benchmark showed already reaches ~270-490 Mrays/s on the 680M.

const MAX_LEAF = 4;

const FACES = [[0, 2, 6, 4], [1, 3, 7, 5], [0, 1, 5, 4], [2, 3, 7, 6], [0, 1, 3, 2], [4, 5, 7, 6]];

/**
 * Minimal known-answer scene: one ground plane and one 2 m cube, centred in x/z and
 * sitting on the plane. Every surface is axis-aligned and its normal is unambiguous,
 * so a render can be checked by inspection: the lit face should be bright, the other
 * faces should receive bounce light only, and the plane should show a soft shadow.
 */
export function buildSimpleScene() {
  const t = [];
  box(t, 0, -0.5, 0, 10, 0.5, 10);        // ground: 20 x 20 m, top face at y = 0
  box(t, 0, 1, 0, 1, 1, 1);               // one cube: 2 x 2 x 2, y in [0, 2]
  const tris = new Float32Array(t);
  let mnx = Infinity, mny = Infinity, mnz = Infinity;
  let mxx = -Infinity, mxy = -Infinity, mxz = -Infinity;
  for (let i = 0; i < tris.length; i += 3) {
    mnx = Math.min(mnx, tris[i]); mxx = Math.max(mxx, tris[i]);
    mny = Math.min(mny, tris[i + 1]); mxy = Math.max(mxy, tris[i + 1]);
    mnz = Math.min(mnz, tris[i + 2]); mxz = Math.max(mxz, tris[i + 2]);
  }
  return { tris, triCount: tris.length / 9, sceneMin: [mnx, mny, mnz], sceneMax: [mxx, mxy, mxz] };
}

/** Append one box's 12 triangles to `out`. */
function box(out, cx, cy, cz, ex, ey, ez) {
  const corner = i => [
    cx + (i & 1 ? ex : -ex),
    cy + (i & 2 ? ey : -ey),
    cz + (i & 4 ? ez : -ez),
  ];
  const tri = (a, b, c) => {
    for (const v of [a, b, c]) out.push(v[0], v[1], v[2]);
  };
  for (const f of FACES) {
    const q = f.map(i => corner(i));
    tri(q[0], q[1], q[2]);
    tri(q[0], q[2], q[3]);
  }
}

/**
 * A small dungeon-ish scene: ground, a room with a doorway and a window,
 * crates inside, pillars outside. Deliberately includes a thin wall (the
 * doorway pillar) and enclosed space so indirect light and probe validity
 * both have something to do.
 */
export function buildScene() {
  const t = [];
  // ground (thin, large)
  box(t, 0, -0.5, 0, 34, 0.5, 34);
  // room: 16 x 4 x 16 at origin, walls 0.25 thick
  const W = 16, H = 4, T = 0.25;
  const hx = W / 2, hz = W / 2;
  // -Z wall (solid, with window)
  box(t, 0, H / 2, -hz, hx, H / 2, T);                    // solid lower/full: keep simple
  // +Z wall (solid)
  box(t, 0, H / 2, hz, hx, H / 2, T);
  // -X wall (solid)
  box(t, -hx, H / 2, 0, T, H / 2, hz);
  // +X wall with a doorway gap: left, right, lintel
  box(t, hx, H / 2, -hz + 3.5, T, H / 2, 3.5);            // left of door
  box(t, hx, H / 2, hz - 3.5, T, H / 2, 3.5);             // right of door
  box(t, hx, H - 0.5, 0, T, 0.5, hz);                     // lintel above door
  // Optional: fill the doorway gap exactly (y in [0, H-0.5], z in [-1, 1]) to make the
  // room airtight. Used by configs/sealed.js as the leak test: with no opening, a
  // correct probe field leaves the interior unlit, because neither the light nor the sky
  // can reach it and the probes have nothing to carry in.
  if (globalThis.__probeGiConfig?.seal === true) {
    box(t, hx, H / 2 - 0.25, 0, T, H / 2 - 0.25, 1.0);
  }
  // roof
  box(t, 0, H, 0, hx, T, hz);
  // crates inside
  box(t, -4, 1, -4, 1, 1, 1);
  box(t, -2.5, 0.75, -5.5, 0.75, 0.75, 0.75);
  box(t, 4, 1.25, 3, 1.25, 1.25, 1.25);
  // a thin interior partition (thinner than the near cascade spacing) — tests
  // that probes inside geometry deactivate instead of leaking
  box(t, 0, 1.5, 4, 6, 1.5, 0.08);
  // pillars outside
  for (let i = 0; i < 6; i++) box(t, 26, 2.5, -15 + i * 6, 0.6, 2.5, 0.6);
  // outer wall with a gap, to give the far cascade something to occlude
  box(t, 40, 2, 0, 0.4, 2, 40);
  box(t, 0, 2, 40, 40, 2, 0.4);

  const tris = new Float32Array(t);
  let mnx = Infinity, mny = Infinity, mnz = Infinity;
  let mxx = -Infinity, mxy = -Infinity, mxz = -Infinity;
  for (let i = 0; i < tris.length; i += 3) {
    mnx = Math.min(mnx, tris[i]); mxx = Math.max(mxx, tris[i]);
    mny = Math.min(mny, tris[i + 1]); mxy = Math.max(mxy, tris[i + 1]);
    mnz = Math.min(mnz, tris[i + 2]); mxz = Math.max(mxz, tris[i + 2]);
  }
  return { tris, triCount: tris.length / 9, sceneMin: [mnx, mny, mnz], sceneMax: [mxx, mxy, mxz] };
}

function boundsOfRange(tris, index, triMin, triMax, start, end) {
  const mn = [Infinity, Infinity, Infinity];
  const mx = [-Infinity, -Infinity, -Infinity];
  for (let i = start; i < end; i++) {
    const t = index[i];
    for (let a = 0; a < 3; a++) {
      if (triMin[t * 3 + a] < mn[a]) mn[a] = triMin[t * 3 + a];
      if (triMax[t * 3 + a] > mx[a]) mx[a] = triMax[t * 3 + a];
    }
  }
  return [mn, mx];
}

/** Median-split binary BVH. Nodes are 32 bytes: min(3f)+leftFirst(u32), max(3f)+triCount(u32). */
export function buildBVH(tris, triCount) {
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

  function build(start, end) {
    const [mn, mx] = boundsOfRange(tris, index, triMin, triMax, start, end);
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
  // Node = 2 x vec4: [min.xyz, leftFirst(u32)] [max.xyz, triCount(u32)].
  // The integer fields MUST be written through a u32 view: storing an integer in
  // a Float32Array and reading it as u32 in WGSL yields the float's bit pattern
  // (1.0 -> 1065353216), which corrupts child indices and spins the traversal.
  const nodeBytes = new ArrayBuffer(nodeCount * 32);
  const nodes = new Float32Array(nodeBytes);
  const nodeU32 = new Uint32Array(nodeBytes);
  for (let i = 0; i < nodeCount; i++) {
    const o = i * 8;
    nodes[o] = nmin[i][0]; nodes[o + 1] = nmin[i][1]; nodes[o + 2] = nmin[i][2];
    nodeU32[o + 3] = nleft[i];
    nodes[o + 4] = nmax[i][0]; nodes[o + 5] = nmax[i][1]; nodes[o + 6] = nmax[i][2];
    nodeU32[o + 7] = ncount[i];
  }
  const order = new Uint32Array(triCount);
  for (let i = 0; i < triCount; i++) order[i] = index[i];
  // right-child table (interior nodes only; -1 for leaves)
  const rights = new Int32Array(nodeCount);
  for (let i = 0; i < nodeCount; i++) rights[i] = ncount[i] === 0 ? nright[i] : -1;
  return { nodes, nodeU32, nodeCount, order, rights };
}

/** Exact single-triangle closest hit. */
export function triHit(tris, i, o, d, tMax = Infinity) {
  const p = i * 9;
  const ax = tris[p], ay = tris[p + 1], az = tris[p + 2];
  const e1x = tris[p + 3] - ax, e1y = tris[p + 4] - ay, e1z = tris[p + 5] - az;
  const e2x = tris[p + 6] - ax, e2y = tris[p + 7] - ay, e2z = tris[p + 8] - az;
  const px = d[1] * e2z - d[2] * e2y, py = d[2] * e2x - d[0] * e2z, pz = d[0] * e2y - d[1] * e2x;
  const det = e1x * px + e1y * py + e1z * pz;
  if (Math.abs(det) < 1e-12) return -1;
  const inv = 1 / det;
  const tx = o[0] - ax, ty = o[1] - ay, tz = o[2] - az;
  const u = (tx * px + ty * py + tz * pz) * inv;
  if (u < 0 || u > 1) return -1;
  const qx = ty * e1z - tz * e1y, qy = tz * e1x - tx * e1z, qz = tx * e1y - ty * e1x;
  const v = (d[0] * qx + d[1] * qy + d[2] * qz) * inv;
  if (v < 0 || u + v > 1) return -1;
  const tt = (e2x * qx + e2y * qy + e2z * qz) * inv;
  return tt > 0.001 && tt <= tMax ? tt : -1;
}

/** CPU closest-hit over every triangle, used to self-test the BVH. */
export function bruteHit(tris, triCount, o, d, tMax = Infinity) {
  let best = Infinity;
  for (let i = 0; i < triCount; i++) {
    const t = triHit(tris, i, o, d, tMax);
    if (t > 0 && t < best) best = t;
  }
  return best;
}

/**
 * CPU BVH traversal mirroring the WGSL. Takes both views of the node buffer:
 * bounds are floats, the child index / triangle count are u32.
 */
export function bvhHit(nodes, nodeU32, rights, order, tris, o, d, tMax = Infinity) {
  const stack = new Int32Array(64);
  let sp = 0;
  stack[sp++] = 0;
  let best = Infinity;
  const inv = [1 / d[0], 1 / d[1], 1 / d[2]];
  while (sp > 0) {
    const ni = stack[--sp];
    const n = ni * 8;
    if (ni < 0 || ni >= rights.length) continue;
    let tmin = -Infinity, tmax = Infinity;
    for (let a = 0; a < 3; a++) {
      const t0 = (nodes[n + a] - o[a]) * inv[a];
      const t1 = (nodes[n + 4 + a] - o[a]) * inv[a];
      const lo = Math.min(t0, t1), hi = Math.max(t0, t1);
      if (lo > tmin) tmin = lo;
      if (hi < tmax) tmax = hi;
    }
    if (tmin > tmax || tmax < 0.001 || tmin > best) continue;
    const left = nodeU32[n + 3], count = nodeU32[n + 7];
    if (count > 0) {
      for (let k = 0; k < count; k++) {
        const tri = order[left + k];
        const t = triHit(tris, tri, o, d);
        if (t > 0 && t < best) best = t;
      }
    } else {
      if (sp + 2 > stack.length) return best;   // guard: never let the mirror spin
      stack[sp++] = rights[ni];
      stack[sp++] = left;
    }
  }
  return best;
}

/** Self-test: BVH traversal must agree with brute force on random rays. */
export function selfTest(scene, bvh, samples = 512) {
  let seed = 12345;
  const rnd = () => (seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296;
  let checked = 0, bad = 0, hits = 0;
  for (let i = 0; i < samples; i++) {
    const o = [
      scene.sceneMin[0] + rnd() * (scene.sceneMax[0] - scene.sceneMin[0]),
      scene.sceneMin[1] + rnd() * (scene.sceneMax[1] - scene.sceneMin[1]),
      scene.sceneMin[2] + rnd() * (scene.sceneMax[2] - scene.sceneMin[2]),
    ];
    const th = rnd() * Math.PI * 2, z = rnd() * 2 - 1, r = Math.sqrt(Math.max(0, 1 - z * z));
    const d = [Math.cos(th) * r, Math.sin(th) * r, z];
    const a = bvhHit(bvh.nodes, bvh.nodeU32, bvh.rights, bvh.order, scene.tris, o, d);
    const b = bruteHit(scene.tris, scene.triCount, o, d);
    checked++;
    if (Number.isFinite(a) || Number.isFinite(b)) hits++;
    const ta = Math.min(a, 1e29), tb = Math.min(b, 1e29);
    if (!(Math.abs(ta - tb) <= 1e-3 + 1e-3 * Math.abs(tb))) bad++;
  }
  return { checked, bad, hits };
}
