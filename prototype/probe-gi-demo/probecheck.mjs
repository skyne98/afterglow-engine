// CPU reference for the probe field, mirroring gi.wgsl.js + render.wgsl.js closely enough
// to answer "why does the lighting change drastically as the camera moves?" without a GPU
// run per hypothesis. Nothing here is used by the demo.
//
// Three things it measures:
//   1. per-cascade probe sweep - how the 64 rays land (front face / back face / miss) and
//      the mean irradiance, split by which side of the room shell the probe is on. This is
//      how a cascade that is dark, or that is lit only through a wall, shows up.
//   2. the render's cage for a query point, term by term: per-cascade E, weight, and the
//      share of that weight coming from probes outside the room.
//   3. SWING - for a query point fixed in the world, the spread of its blended irradiance
//      across camera positions. That is the number the user's report is about: a surface
//      whose GI depends on where the camera is, is a surface that "switches as I move".
//
//   node prototype/probe-gi-demo/probecheck.mjs [scene] [spacings] [mode]
//     scene     complex (default) | simple
//     spacings  "1,3,9" (default)
//     mode      all (default) | swing
//
// The octa map is skipped deliberately: a fetch at direction u IS the cos-weighted average
// over the rays inside u's cone, so evaluating that directly gives the same quantity without
// the 8x8 discretization, which would only add sampling error to an A/B of the *rule*.
import { buildScene, buildSimpleScene } from './bvh.js';

globalThis.__probeGiConfig = globalThis.__probeGiConfig ?? {};
const argv = process.argv.slice(2);
const SCENE = argv[0] ?? 'complex';
const SPACINGS = (argv[1] ?? '1,3,9').split(',').map(Number);
const MODE = argv[2] ?? 'all';
const scene = SCENE === 'complex' ? buildScene() : buildSimpleScene();
const T = scene.tris, NT = scene.triCount;

const PI = Math.PI, RAYS = 64, SKY = 1.4;
const LIGHT = SCENE === 'complex' ? [3.0, 3.0, 0.0] : [0.0, 4.0, 3.0];
const LIGHT_I = SCENE === 'complex' ? 5.0 : 12.5;
const EYE = SCENE === 'complex' ? [0.0, 1.6, -3.0] : [5.0, 3.2, 5.0];
const COUNTS = [8, 4, 8];
const ROOM = { x: 8, y0: 0, y1: 4, z: 8 };   // the demo room's shell, for the inside/outside split

const vtx = (i, v) => [T[i * 9 + v * 3], T[i * 9 + v * 3 + 1], T[i * 9 + v * 3 + 2]];
const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
function norm(v) { const l = Math.hypot(v[0], v[1], v[2]) || 1; return [v[0] / l, v[1] / l, v[2] / l]; }
function smoothstep(a, b, x) { const t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); }

/** Mirror of triNormal. */
function normalOf(i) {
  const a = vtx(i, 0);
  return norm(cross(sub(vtx(i, 1), a), sub(vtx(i, 2), a)));
}
/** Mirror of hitTri: Moller-Trumbore, no culling, same epsilon. */
function hitTri(o, d, i, tMax) {
  const a = vtx(i, 0), e1 = sub(vtx(i, 1), a), e2 = sub(vtx(i, 2), a);
  const p = cross(d, e2), det = dot(e1, p);
  if (Math.abs(det) < 1e-12) return -1;
  const inv = 1 / det, tv = sub(o, a);
  const u = dot(tv, p) * inv;
  if (u < 0 || u > 1) return -1;
  const q = cross(tv, e1), v = dot(d, q) * inv;
  if (v < 0 || u + v > 1) return -1;
  const t = dot(e2, q) * inv;
  return (t < 0.001 || t > tMax) ? -1 : t;
}
/** Brute-force traverseClosest: 240 triangles make a BVH unnecessary in a reference. */
function closest(o, d, tMax) {
  let bestT = tMax, bestTri = -1;
  for (let i = 0; i < NT; i++) {
    const t = hitTri(o, d, i, bestT);
    if (t > 0) { bestT = t; bestTri = i; }
  }
  return { t: bestT, tri: bestTri };
}
function occluded(o, d, tMax) {
  for (let i = 0; i < NT; i++) if (hitTri(o, d, i, tMax) > 0) return true;
  return false;
}
function skyRadiance(d) {
  const up = Math.min(1, Math.max(0, d[1] * 0.5 + 0.5));
  const lo = [0.10, 0.10, 0.11], hi = [0.30, 0.32, 0.36];
  return [0, 1, 2].map(k => (lo[k] + (hi[k] - lo[k]) * up) * SKY);
}
/** Mirror of triAlbedo (orientation-driven, so walls/floor/crates differ). */
function albedoOf(i) {
  const n = normalOf(i);
  const horizontal = Math.abs(n[1]) >= 0.7 ? 1 : 0;
  const floorCol = [0.30, 0.29, 0.28], wallCol = [0.40, 0.38, 0.36], crateCol = [0.46, 0.33, 0.22];
  const warm = Math.min(0.18, Math.max(0, (vtx(i, 0)[1] - 1.0) * 0.10));
  return [0, 1, 2].map(k => {
    const base = wallCol[k] + (floorCol[k] - wallCol[k]) * horizontal;
    return base + (crateCol[k] - base) * warm;
  });
}
/** Mirror of shadePoint: one bounce, direct light only, no ambient term. */
function shadePoint(pos, n, i) {
  const toL = sub(LIGHT, pos), dist = Math.hypot(...toL);
  const l = toL.map(v => v / dist);
  const nl = dot(n, l);
  if (nl <= 0) return [0, 0, 0];
  if (occluded(pos.map((v, k) => v + n[k] * 0.02), l, dist - 0.05)) return [0, 0, 0];
  const e = nl * (LIGHT_I / (1 + 0.09 * dist * dist)), a = albedoOf(i);
  return [a[0] * e / PI, a[1] * 0.86 * e / PI, a[2] * 0.68 * e / PI];
}
function rayDirection(r, n) {
  const i = r + 0.5, cosTheta = 1 - 2 * i / n, theta = 2 * PI * i * 0.6180339887498949;
  const sinTheta = Math.sqrt(Math.max(0, 1 - cosTheta * cosTheta));
  return [Math.cos(theta) * sinTheta, Math.sin(theta) * sinTheta, cosTheta];
}

/** Mirror of probeTrace: ray radiance and stored distance, backface rule included. */
function probeRays(p) {
  const rad = new Float32Array(RAYS * 3), dist = new Float32Array(RAYS);
  for (let r = 0; r < RAYS; r++) {
    const d = rayDirection(r, RAYS);
    const o = p.map((v, k) => v + d[k] * 0.02);
    const h = closest(o, d, 1e4);
    if (h.tri < 0) {
      const s = skyRadiance(d);
      rad[r * 3] = s[0]; rad[r * 3 + 1] = s[1]; rad[r * 3 + 2] = s[2];
      dist[r] = 30.0;
      continue;
    }
    const n = normalOf(h.tri), pos = o.map((v, k) => v + d[k] * h.t);
    if (dot(n, d) > 0) { dist[r] = h.t * 0.2; continue; }
    const s = shadePoint(pos, n, h.tri);
    rad[r * 3] = s[0]; rad[r * 3 + 1] = s[1]; rad[r * 3 + 2] = s[2];
    dist[r] = Math.min(h.t, 30);
  }
  return { rad, dist };
}
/** Mirror of atlasFetch for radiance: the atlas stores E = PI * sum(w L) / sum(w). */
function fetchRadiance(rays, u) {
  let s0 = 0, s1 = 0, s2 = 0, wsum = 0;
  for (let r = 0; r < RAYS; r++) {
    const w = Math.max(0, dot(u, rayDirection(r, RAYS)));
    if (w <= 0) continue;
    s0 += rays.rad[r * 3] * w; s1 += rays.rad[r * 3 + 1] * w; s2 += rays.rad[r * 3 + 2] * w;
    wsum += w;
  }
  if (wsum <= 1e-5) return [0, 0, 0];
  return [PI * s0 / wsum, PI * s1 / wsum, PI * s2 / wsum];
}
/** Mirror of atlasFetch for the distance moments (cos^16 by repeated squaring). */
function fetchMoments(rays, u) {
  let d1 = 0, d2 = 0, wsum = 0;
  for (let r = 0; r < RAYS; r++) {
    const w = Math.max(0, dot(u, rayDirection(r, RAYS)));
    if (w <= 0) continue;
    const w2 = w * w, w4 = w2 * w2, w8 = w4 * w4, mw = w8 * w8;
    d1 += mw * rays.dist[r]; d2 += mw * rays.dist[r] * rays.dist[r]; wsum += mw;
  }
  if (wsum <= 1e-5) return [0, 0];
  return [d1 / wsum, d2 / wsum];
}

/** The world grid the host would publish for this camera position and these spacings. */
function cellOf(c, camPos, k) {
  const sp = SPACINGS[c];
  return Math.floor((camPos[k] - COUNTS[k] * sp * 0.5) / sp);
}
function buildField(camPos) {
  return SPACINGS.map((sp, c) => {
    const list = [];
    for (let iz = 0; iz < COUNTS[2]; iz++) for (let iy = 0; iy < COUNTS[1]; iy++) for (let ix = 0; ix < COUNTS[0]; ix++) {
      const local = [ix, iy, iz];
      const cel = [0, 1, 2].map(k => {
        const cell = cellOf(c, camPos, k);
        return cell + (((local[k] - cell) % COUNTS[k]) + COUNTS[k]) % COUNTS[k];
      });
      const p = [0, 1, 2].map(k => (cel[k] + 0.5) * sp);
      list.push({ p, rays: probeRays(p) });
    }
    return list;
  });
}
const insideRoom = p => Math.abs(p[0]) < ROOM.x && p[1] > ROOM.y0 && p[1] < ROOM.y1 && Math.abs(p[2]) < ROOM.z;

/** Mirror of sampleCascade. rule: { dir:'normal'|'probe', sigmaClamp, sigmaFloor }. */
function sampleCascadeCPU(field, camPos, c, pos, n, rule) {
  const sp = SPACINGS[c];
  const origin = [0, 1, 2].map(k => cellOf(c, camPos, k) * sp);
  const rel = pos.map((v, k) => (v - origin[k]) / sp - 0.5);
  for (let k = 0; k < 3; k++) if (rel[k] < -0.5 || rel[k] > COUNTS[k] - 0.5) return null;
  const toEdge = rel.map((v, k) => Math.min(v + 0.5, COUNTS[k] - 0.5 - v));
  const fade = smoothstep(0, rule.ramp, Math.min(...toEdge));
  const base = rel.map(Math.floor), frac = rel.map((v, k) => v - base[k]);
  const biasedPos = pos.map((v, k) => v + n[k] * (0.225 * sp));
  let e = [0, 0, 0], wsum = 0, outW = 0;
  for (let i = 0; i < 8; i++) {
    const d3 = [(i & 1), (i >> 1) & 1, (i >> 2) & 1];
    const wTri = (1 - frac[0] + (2 * frac[0] - 1) * d3[0]) * (1 - frac[1] + (2 * frac[1] - 1) * d3[1]) *
      (1 - frac[2] + (2 * frac[2] - 1) * d3[2]);
    if (wTri <= 1e-6) continue;
    const local = base.map((v, k) => v + d3[k]);
    if (local.some((v, k) => v < 0 || v > COUNTS[k] - 1)) continue;
    // local is a CELL coordinate; atlas order is by SLOT (the wrapping offset from origin).
    const idx = local.map((v, k) => (((v - cellOf(c, camPos, k)) % COUNTS[k]) + COUNTS[k]) % COUNTS[k]);
    const pr = field[c][(idx[2] * COUNTS[1] + idx[1]) * COUNTS[0] + idx[0]];
    const toP = sub(pr.p, pos), dist = Math.max(Math.hypot(...toP), 1e-4);
    const u = toP.map(v => v / dist);
    const wrap = Math.max(0, (dot(n, u) + 1) * 0.5);
    let w = wTri * (wrap * wrap + 0.04);
    const [mean, mean2] = fetchMoments(pr.rays, rule.dir === 'probe' ? u : n);
    const dd = Math.hypot(...sub(biasedPos, pr.p));
    const md = Math.max(dd - mean, 0);
    let sigma2 = Math.max(mean2 - mean * mean, 0.002);
    if (rule.sigmaClamp) sigma2 = Math.min(sigma2, rule.sigmaClamp);
    w *= Math.min(1, Math.max(0, sigma2 / (sigma2 + md * md)));
    // Optional probe-visibility ray: query -> probe through the same BVH. This is the one
    // test that can see a wall *at the query*, which is what the depth-moment gate cannot:
    // a probe two metres outside a 0.25 m wall stores open sky along the query's normal and
    // reports the query as visible.
    if (rule.occl) {
      const o = biasedPos.map((v, k) => v + n[k] * 0.02);
      const toPr = sub(pr.p, o), dd2 = Math.hypot(...toPr);
      const u2 = toPr.map(v => v / dd2);
      const t = closest(o, u2, dd2 - 0.05).t;
      if (t > 0) {
        // Soft: a hit within the last 20% of the way still passes partially, so a probe
        // grazing a wall edge does not flip its weight on and off across a hard line.
        w = rule.occl === 'soft' ? w * smoothstep(dd2 * 0.75, dd2, t) : 0;
      }
    }
    if (!insideRoom(pr.p)) outW += w;
    const E = fetchRadiance(pr.rays, n);
    e = e.map((v, k) => v + E[k] * w);
    wsum += w;
  }
  if (wsum <= 1e-5) return { e: [0, 0, 0], w: 0, fade, outW: 0 };
  return { e: e.map(v => v / wsum), w: wsum, fade, outW };
}

/** Mirror of the fragment's cascade loop. weights: 'fade' (current) or 'complement'. */
function blendAt(field, camPos, pos, n, rule) {
  let accE = [0, 0, 0], accW = 0, prev = 1;
  const per = [];
  for (let c = 0; c < SPACINGS.length; c++) {
    const s = sampleCascadeCPU(field, camPos, c, pos, n, rule);
    if (!s || s.w <= 0) { per.push(null); continue; }
    const share = rule.weights === 'complement' ? prev * s.fade : s.fade;
    per.push({ ...s, share });
    if (share <= 1e-6) continue;
    accE = accE.map((v, k) => v + s.e[k] * share);
    accW += share;
    prev *= (1 - s.fade);
    if (rule.weights === 'complement') { if (prev <= 1e-3) break; }
    else if (s.fade >= 0.999) break;
  }
  const mean = accW > 1e-5 ? accE.map(v => v / accW) : [0, 0, 0];
  return { e: mean, w: accW, per };
}
const luma = e => (e[0] + e[1] + e[2]) / 3;

// ---------------------------------------------------------------- 1. per-cascade sweep
if (MODE !== 'swing') {
  console.log(`scene ${SCENE}: ${NT} tris, light ${LIGHT.map(v => v.toFixed(1))} I=${LIGHT_I}, ` +
    `eye ${EYE.join(',')}, spacings ${SPACINGS.join(',')}`);
  for (let c = 0; c < SPACINGS.length; c++) {
    const sp = SPACINGS[c];
    const cell = [0, 1, 2].map(k => Math.floor((EYE[k] - COUNTS[k] * sp * 0.5) / sp));
    let F = 0, B = 0, M = 0, et = 0, worst = null;
    const acc = { in: { n: 0, e: 0 }, out: { n: 0, e: 0 } };
    for (let iz = 0; iz < COUNTS[2]; iz++) for (let iy = 0; iy < COUNTS[1]; iy++) for (let ix = 0; ix < COUNTS[0]; ix++) {
      const local = [ix, iy, iz];
      const cel = [0, 1, 2].map(k => cell[k] + (((local[k] - cell[k]) % COUNTS[k]) + COUNTS[k]) % COUNTS[k]);
      const p = [0, 1, 2].map(k => (cel[k] + 0.5) * sp);
      const rays = probeRays(p);
      let front = 0, back = 0, miss = 0, e = 0;
      for (let r = 0; r < RAYS; r++) {
        const dir = rayDirection(r, RAYS);
        const o = p.map((v, k) => v + dir[k] * 0.02);
        const h = closest(o, dir, 1e4);
        if (h.tri < 0) { miss++; const s = skyRadiance(dir); e += (s[0] + s[1] + s[2]) / 3; continue; }
        if (dot(normalOf(h.tri), dir) > 0) { back++; continue; }
        const s = shadePoint(o.map((v, k) => v + dir[k] * h.t), normalOf(h.tri), h.tri);
        front++; e += (s[0] + s[1] + s[2]) / 3;
      }
      const E = PI * e / RAYS;
      F += front; B += back; M += miss; et += E;
      const b = acc[insideRoom(p) ? 'in' : 'out'];
      b.n++; b.e += E;
      if (!worst || E < worst.E) worst = { E, p };
      void rays;
    }
    const n = COUNTS[0] * COUNTS[1] * COUNTS[2];
    console.log(`cascade ${c} spacing ${sp}m: mean E ${(et / n).toFixed(3)}  ` +
      `rays front/back/miss ${(F / (n * RAYS) * 100).toFixed(0)}/${(B / (n * RAYS) * 100).toFixed(0)}/` +
      `${(M / (n * RAYS) * 100).toFixed(0)}%  |  probes inside the room ${acc.in.n}: mean E ` +
      `${(acc.in.e / Math.max(acc.in.n, 1)).toFixed(3)}; outside ${acc.out.n}: ` +
      `${(acc.out.e / Math.max(acc.out.n, 1)).toFixed(3)}  |  darkest ${worst.p.join(',')} E=${worst.E.toFixed(3)}`);
  }
}

// ---------------------------------------------------------------- 2. swing
// A query point fixed in the world, evaluated for several camera positions. A cascade field
// that is doing its job gives the same answer for every camera position; the ratio between
// the brightest and dimmest is exactly what the eye reads as "the lighting switched".
const rules = [];
for (const weights of ['fade', 'complement']) {
  for (const ramp of [1.0, 2.5]) {
    rules.push({ name: `${weights} ramp ${ramp}`, weights, ramp, dir: 'normal' });
  }
}
rules.push({ name: 'probeDir fade 2.5  ', weights: 'fade', ramp: 2.5, dir: 'probe' });
rules.push({ name: 'probeDir compl 2.5 ', weights: 'complement', ramp: 2.5, dir: 'probe' });
rules.push({ name: 'probeDir compl 1.0 ', weights: 'complement', ramp: 1.0, dir: 'probe' });
rules.push({ name: 'probeDir fade 1.0  ', weights: 'fade', ramp: 1.0, dir: 'probe' });
rules.push({ name: 'occl-soft fade 2.5 ', weights: 'fade', ramp: 2.5, dir: 'normal', occl: 'soft' });
rules.push({ name: 'occl-soft compl 2.5', weights: 'complement', ramp: 2.5, dir: 'normal', occl: 'soft' });
rules.push({ name: 'occl-hard fade 2.5 ', weights: 'fade', ramp: 2.5, dir: 'normal', occl: 'binary' });
const query = SCENE === 'complex' ? { pos: [0.0, 0.0, 0.0], n: [0, 1, 0], label: 'room floor centre' }
  : { pos: [0.0, 0.0, 1.5], n: [0, 1, 0], label: 'ground in front of the cube' };
console.log(`\nswing of ${query.label} at ${query.pos.join(',')} over camera positions ` +
  `(x -2..2, y 1.6 and 2.6, z -3):`);
const cams = [];
for (const y of [1.6, 2.1, 2.6]) for (const x of [-1.0, 0.0, 1.0]) cams.push([x, y, -3.0]);
for (const rule of rules) {
  const vals = [];
  for (const cam of cams) {
    const field = buildField(cam);
    vals.push(luma(blendAt(field, cam, query.pos, query.n, rule).e));
  }
  const lo = Math.min(...vals), hi = Math.max(...vals);
  console.log(`  ${rule.name.padEnd(20)} E ${vals.map(v => v.toFixed(3)).join(' ')}  ` +
    `min ${lo.toFixed(3)} max ${hi.toFixed(3)} swing ${(hi / Math.max(lo, 1e-5)).toFixed(2)}x`);
}

// ---------------------------------------------------------------- 3. floor profile
// A fixed camera and a query walking away from it along the floor. A cascade handover that
// is doing its job leaves a smooth profile; a step in it is the band the eye reads as an
// artifact, and because the cascade boxes are camera-locked, that band moves with the camera.
{
  const cam = EYE.slice();
  const field = buildField(cam);
  console.log(`\nfloor profile (camera ${cam.join(',')}), query walks away along -Z:`);
  for (const rule of rules) {
    const row = [];
    for (let d = 1.0; d <= 8.0; d += 0.5) {
      const pos = [cam[0], 0.0, cam[2] + 0.6 + d];
      const b = blendAt(field, cam, pos, [0, 1, 0], rule);
      row.push(luma(b.e));
    }
    const lo = Math.min(...row), hi = Math.max(...row);
    console.log(`  ${rule.name.padEnd(20)} ${row.map(v => v.toFixed(3)).join(' ')}  max/min ${(hi / Math.max(lo, 1e-5)).toFixed(2)}x`);
  }
}

// ---------------------------------------------------------------- 4. per-probe cage detail
if (MODE === 'detail') {
  const cam = EYE.slice();
  const field = buildField(cam);
  const pos = [cam[0], 0.0, cam[2] + 5.6], n = [0, 1, 0];
  const rule = { weights: 'fade', ramp: 2.5, dir: 'normal', occl: 'soft' };
  console.log(`\ncage detail for floor query ${pos.join(',')} n=+Y`);
  for (let c = 0; c < SPACINGS.length; c++) {
    const sp = SPACINGS[c];
    const origin = [0, 1, 2].map(k => cellOf(c, cam, k) * sp);
    const rel = pos.map((v, k) => (v - origin[k]) / sp - 0.5);
    const inside = rel.every((v, k) => v >= -0.5 && v <= COUNTS[k] - 0.5);
    console.log(` cascade ${c}: rel ${rel.map(v => v.toFixed(2)).join(',')} ${inside ? 'inside' : 'OUTSIDE box'}`);
    if (!inside) continue;
    const biasedPos = pos.map((v, k) => v + n[k] * (0.225 * sp));
    const base = rel.map(Math.floor), frac = rel.map((v, k) => v - base[k]);
    for (let i = 0; i < 8; i++) {
      const d3 = [(i & 1), (i >> 1) & 1, (i >> 2) & 1];
      const local = base.map((v, k) => v + d3[k]);
      if (local.some((v, k) => v < 0 || v > COUNTS[k] - 1)) { console.log(`   corner ${i}: out of grid`); continue; }
      const idx = local.map((v, k) => (((v - cellOf(c, cam, k)) % COUNTS[k]) + COUNTS[k]) % COUNTS[k]);
      const pr = field[c][(idx[2] * COUNTS[1] + idx[1]) * COUNTS[0] + idx[0]];
      const toP = sub(pr.p, pos), dist = Math.hypot(...toP), u = toP.map(v => v / dist);
      const [mean, mean2] = fetchMoments(pr.rays, n);
      const dd = Math.hypot(...sub(biasedPos, pr.p));
      const md = Math.max(dd - mean, 0);
      const sigma2 = Math.max(mean2 - mean * mean, 0.002);
      const cheb = Math.min(1, Math.max(0, sigma2 / (sigma2 + md * md)));
      const o = biasedPos.map((v, k) => v + n[k] * 0.02);
      const toPr = sub(pr.p, o), dd2 = Math.hypot(...toPr), u2 = toPr.map(v => v / dd2);
      const tHit = closest(o, u2, dd2 - 0.05).t;
      const cheb2 = Math.min(1, Math.max(0, sigma2 / (sigma2 + md * md)));
      console.log(`   corner ${i} probe ${pr.p.map(v => v.toFixed(1)).join(',')} ` +
        `${insideRoom(pr.p) ? 'in-room ' : 'OUTSIDE '} d=${dist.toFixed(2)} mean=${mean.toFixed(2)} ` +
        `sd=${Math.sqrt(sigma2).toFixed(2)} cheb=${cheb.toFixed(3)} occl=${tHit > 0 ? 'BLOCKED t=' + tHit.toFixed(2) : 'clear'}` +
        ` | byNormal E=${luma(fetchRadiance(pr.rays, n)).toFixed(3)}`);
      void cheb2; void u; void dist;
    }
  }
}
