// CPU-only self-test for the demo's scene + BVH + probe field math.
// Run: node prototype/probe-gi-demo/selftest.mjs
// Verifies that the BVH traversal the WGSL mirrors agrees with brute force, and
// that the cascade/slot/atlas indexing the shaders rely on is in range.

import { buildScene, buildBVH, selfTest, bruteHit, triHit } from './bvh.js';

const PROBE_X = 8, PROBE_Y = 4, PROBE_Z = 8;
const PER_CASCADE = PROBE_X * PROBE_Y * PROBE_Z;
const CASCADES = 3;
const SPACINGS = [1, 3, 9];
const TOTAL_PROBES = CASCADES * PER_CASCADE;
const RAD_TILE = 10, RAD_ATLAS = 512, RAD_SLOTS = Math.floor(RAD_ATLAS / RAD_TILE);
const DIST_TILE = 18, DIST_ATLAS = 1024, DIST_SLOTS = Math.floor(DIST_ATLAS / DIST_TILE);

let failures = 0;
const check = (name, ok, detail = '') => {
  if (!ok) failures++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? '  ' + detail : ''}`);
};

// --- single-triangle hit vs brute force over all triangles
const scene = buildScene();
const bvh = buildBVH(scene.tris, scene.triCount);
let seed = 987654321;
const rnd = () => (seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296;
const span = [0, 1, 2].map(a => scene.sceneMax[a] - scene.sceneMin[a]);
const randPoint = () => [0, 1, 2].map(a => scene.sceneMin[a] + rnd() * span[a]);
const randDir = () => {
  const th = rnd() * Math.PI * 2, z = rnd() * 2 - 1, r = Math.sqrt(Math.max(0, 1 - z * z));
  return [Math.cos(th) * r, Math.sin(th) * r, z];
};

// --- BVH traversal must equal brute force
const st = selfTest(scene, bvh, 1024);
check('BVH traversal matches brute force', st.bad === 0, `${st.checked - st.bad}/${st.checked} rays, ${st.hits} hits`);

// --- triHit is exact against an independent Möller–Trumbore formulation
const referenceTriHit = (tris, i, o, d) => {
  const p = i * 9;
  const a = [tris[p], tris[p + 1], tris[p + 2]];
  const b = [tris[p + 3], tris[p + 4], tris[p + 5]];
  const c = [tris[p + 6], tris[p + 7], tris[p + 8]];
  const sub = (u, v) => [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
  const cross = (u, v) => [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
  const dot = (u, v) => u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
  const e1 = sub(b, a), e2 = sub(c, a);
  const h = cross(d, e2);
  const det = dot(e1, h);
  if (Math.abs(det) < 1e-12) return -1;
  const s = sub(o, a);
  const u = dot(s, h) / det;
  if (u < 0 || u > 1) return -1;
  const q = cross(s, e1);
  const v = dot(d, q) / det;
  if (v < 0 || u + v > 1) return -1;
  const t = dot(e2, q) / det;
  return t > 0.001 ? t : -1;
};
let triBad = 0;
for (let n = 0; n < 2000; n++) {
  const o = randPoint(), d = randDir();
  const i = Math.floor(rnd() * scene.triCount);
  const a = triHit(scene.tris, i, o, d);
  const b = referenceTriHit(scene.tris, i, o, d);
  const ta = a > 0 ? a : 1e29, tb = b > 0 ? b : 1e29;
  if (Math.abs(ta - tb) > 1e-4 * tb) triBad++;
}
check('triHit agrees with an independent implementation', triBad === 0, `${2000 - triBad}/2000`);

// --- BVH structural soundness: no cycles, all nodes reachable, leaves in range
const seen = new Uint8Array(bvh.nodeCount);
const stack = [0];
let cycles = 0, badLeaf = 0, maxDepth = 0;
const depth = new Int32Array(bvh.nodeCount);
while (stack.length) {
  const ni = stack.pop();
  if (ni < 0 || ni >= bvh.nodeCount || seen[ni]) { cycles++; continue; }
  seen[ni] = 1;
  const n = ni * 8;
  const left = bvh.nodeU32[n + 3], count = bvh.nodeU32[n + 7];
  if (count > 0) {
    if (left + count > bvh.order.length) badLeaf++;
    for (let k = 0; k < count; k++) if (bvh.order[left + k] >= scene.triCount) badLeaf++;
  } else {
    depth[left] = depth[ni] + 1; depth[bvh.rights[ni]] = depth[ni] + 1;
    if (depth[left] > maxDepth) maxDepth = depth[left];
    stack.push(left, bvh.rights[ni]);
  }
}
check('BVH has no cycles and no bad leaves', cycles === 0 && badLeaf === 0, `cycles=${cycles} badLeaves=${badLeaf} depth=${maxDepth}`);

// --- node metadata is u32-exact (the bug that previously hung the GPU)
check('node metadata is u32-exact', bvh.nodeU32[3] === 1 && bvh.nodes[3] !== 1, `leftFirst u32=${bvh.nodeU32[3]} float=${bvh.nodes[3]}`);

// --- atlas capacity covers the resident field
check('radiance atlas holds the field', RAD_SLOTS * RAD_SLOTS >= TOTAL_PROBES, `${RAD_SLOTS * RAD_SLOTS} slots >= ${TOTAL_PROBES} probes`);
check('distance atlas holds the field', DIST_SLOTS * DIST_SLOTS >= TOTAL_PROBES, `${DIST_SLOTS * DIST_SLOTS} slots`);

// --- cascade/slot mapping: every resident slot maps to a unique cell per cascade
function cellFor(camPos) {
  const cells = [];
  for (let c = 0; c < CASCADES; c++) {
    const sp = SPACINGS[c];
    const counts = [PROBE_X, PROBE_Y, PROBE_Z];
    const o = [camPos[0] - counts[0] * sp * 0.5, camPos[1] - counts[1] * sp * 0.5, camPos[2] - counts[2] * sp * 0.5];
    const base = o.map(v => Math.floor(v / sp));
    const set = new Set();
    for (let s = 0; s < PER_CASCADE; s++) {
      const local = [s % PROBE_X, Math.floor(s / PROBE_X) % PROBE_Y, Math.floor(s / (PROBE_X * PROBE_Y))];
      const key = local.map((l, a) => base[a] + (((l - base[a]) % counts[a]) + counts[a]) % counts[a]).join(',');
      set.add(key);
    }
    cells.push(set.size);
  }
  return cells;
}
const unique = cellFor([12.3, 4.7, -8.1]);
check('each cascade maps slots to unique cells', unique.every(n => n === PER_CASCADE), `unique=${unique.join('/')} of ${PER_CASCADE}`);

// --- a scroll step must only invalidate the wrapped band
function movedBetween(a, b) {
  let moved = 0;
  for (let c = 0; c < CASCADES; c++) {
    const sp = SPACINGS[c];
    const counts = [PROBE_X, PROBE_Y, PROBE_Z];
    const base = p => p.map((v, i) => Math.floor((v - counts[i] * sp * 0.5) / sp));
    const ba = base(a), bb = base(b);
    for (let s = 0; s < PER_CASCADE; s++) {
      const local = [s % PROBE_X, Math.floor(s / PROBE_X) % PROBE_Y, Math.floor(s / (PROBE_X * PROBE_Y))];
      const cellA = local.map((l, i) => ba[i] + (((l - ba[i]) % counts[i]) + counts[i]) % counts[i]);
      const cellB = local.map((l, i) => bb[i] + (((l - bb[i]) % counts[i]) + counts[i]) % counts[i]);
      if (cellA.join() !== cellB.join()) moved++;
    }
  }
  return moved;
}
// move exactly one near-cascade cell in x
const m = movedBetween([12.0, 4.0, -8.0], [13.0, 4.0, -8.0]);
check('one scroll step re-seeds only the wrapped band', m <= PER_CASCADE * CASCADES * 0.25, `${m} of ${TOTAL_PROBES} probes re-seeded`);


// --- atlas mapping round trip: the blend writes a texel for a direction, and the render
// fetches by direction. They must agree, or samples land between the border texel and the
// first interior texel (which is exactly the half-texel bug this replaced).
{
  const INTERIOR = 8, TILE = 10, SLOTS = 28;
  const octDecode = (u, v) => {
    const fx = u * 2 - 1, fy = v * 2 - 1;
    let nx = fx, ny = fy, nz = 1 - Math.abs(fx) - Math.abs(fy);
    const t = Math.min(Math.max(-nz, 0), 1);
    nx += nx >= 0 ? -t : t; ny += ny >= 0 ? -t : t;
    const l = Math.hypot(nx, ny, nz); return [nx / l, ny / l, nz / l];
  };
  const octEncode = n => {
    const l = Math.abs(n[0]) + Math.abs(n[1]) + Math.abs(n[2]) || 1e-6;
    let fx = n[0] / l, fy = n[1] / l;
    if (n[2] < 0) {
      const ax = 1 - Math.abs(fy), ay = 1 - Math.abs(fx);
      fx = (fx >= 0 ? 1 : -1) * ax; fy = (fy >= 0 ? 1 : -1) * ay;
    }
    return [fx * 0.5 + 0.5, fy * 0.5 + 0.5];
  };
  // The blend's direction for tile-local texel (px, py).
  const texelDirection = (px, py) => octDecode((px - 0.5) / INTERIOR, (py - 0.5) / INTERIOR);
  // The render's fetch coordinate in atlas texels for probe 0 and a direction.
  const fetchTexel = dir => {
    const uv = octEncode(dir);
    return [1.0 + uv[0] * INTERIOR, 1.0 + uv[1] * INTERIOR];
  };
  let worst = 0, worstAt = '';
  for (let py = 1; py <= INTERIOR; py++) {
    for (let px = 1; px <= INTERIOR; px++) {
      const c = fetchTexel(texelDirection(px, py));
      // A fetch must land on the centre of the texel whose direction it came from: the
      // interior texel (px, py) sits at atlas texel px (its centre at px + 0.5).
      worst = Math.max(worst, Math.abs(c[0] - (px + 0.5)), Math.abs(c[1] - (py + 0.5)));
      if (worst > 1e-9) worstAt = `(${px},${py}) -> ${c}`;
    }
  }
  check('atlas fetch returns the texel centre of the blended direction', worst < 1e-9,
    `worst offset ${worst.toExponential(2)} texels${worstAt ? ' at ' + worstAt : ''}`);
  // The interior's outer edge deliberately lands half a texel outside, on the border
  // texel: that neighbour is what keeps the bilinear reconstruction continuous across the
  // octahedral seam. A fetch that instead clamped to texel centres would lose it.
  const edge = (() => {
    const uv = octEncode(texelDirection(1, 1));   // first interior texel's direction
    return 1.0 + uv[0] * INTERIOR;                 // its fetch coordinate
  })();
  check('the interior edge blends with the border texel by design', Math.abs(edge - 1.5) < 1e-9,
    `first interior texel direction fetches at ${edge.toFixed(3)} (centre 1.5, border 0.5)`);
}

console.log(failures === 0 ? '\nall checks passed' : `\n${failures} check(s) FAILED`);
process.exit(failures === 0 ? 0 : 1);
