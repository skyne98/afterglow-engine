// CPU reference for the probe-GI chain. Mirrors the WGSL formulas literally, so the
// GPU stages can be diffed against it one at a time:
//
//   1. probe positions        (grid + modulo slot mapping)
//   2. probe ray trace        (closest hit, backface rule, direct shading, sky)
//   3. atlas blend            (octahedral texel directions, cosine weights, gamma-5)
//   4. cage sample            (trilinear x backface x Chebyshev, biased position)
//   5. fragment shade         (direct/PI + indirect*AO, ACES, sRGB)
//
// Every constant here is the same literal the WGSL uses. If the two disagree, that is
// the finding.

import {
  LIGHT, skyRadiance, shadeHit, shadeSurfaceDirect, traceClosest, aoFactor,
  normalize, triAlbedo,
} from './scene.js';

export const PI = 3.141592653589793;
export const GAMMA_INV = 0.2;          // atlas stores pow(radiance, 1/5)
export const GAMMA = 5.0;              // and decodes pow(stored, 5)
export const HYSTERESIS = 0.93;
export const VARIANCE_FLOOR = 0.02;    // matches the Chebyshev sigma2 floor in WGSL
export const SELF_SHADOW_BIAS = 0.225; // in spacing units

export const FIELD = {
  counts: [2, 2, 2],
  spacing: 1,
  // Probe centres land at x in {0,1}, y in {0.5,1.5}, z in {-1,0}: all outside the box,
  // two of them directly above it.
  origin: [-0.5, 0, -1.5],
  raysPerProbe: 6,
};

export const RAY_DIRECTIONS = [
  [1, 0, 0], [-1, 0, 0],   // +X -X
  [0, 1, 0], [0, -1, 0],   // +Y -Y
  [0, 0, 1], [0, 0, -1],   // +Z -Z
].map(normalize);

export const ATLAS = { interior: 4, tile: 6 };  // 1-texel border, so 6x6 per probe

// ---------------------------------------------------------------- 1. probe positions

export function probeIndex(cell) {
  return (cell[2] * FIELD.counts[1] + cell[1]) * FIELD.counts[0] + cell[0];
}

export function probeCell(index) {
  const [cx, cy] = FIELD.counts;
  return [index % cx, Math.floor(index / cx) % cy, Math.floor(index / (cx * cy))];
}

export function probePosition(index) {
  const cell = probeCell(index);
  return cell.map((c, axis) => FIELD.origin[axis] + (c + 0.5) * FIELD.spacing);
}

export function probePositions() {
  const total = FIELD.counts[0] * FIELD.counts[1] * FIELD.counts[2];
  return Array.from({ length: total }, (_, i) => probePosition(i));
}

// ---------------------------------------------------------------- 2. probe ray trace

/**
 * One probe's rays. Returns per-ray { direction, hit, t, radiance }.
 * `bias` multiplies the spacing for the ray-origin offset (0 = exact, hand-checkable).
 */
export function traceProbeRays(scene, index, bias = SELF_SHADOW_BIAS) {
  const position = probePosition(index);
  return RAY_DIRECTIONS.map((d, ray) => {
    const o = [
      position[0] + d[0] * bias * FIELD.spacing,
      position[1] + d[1] * bias * FIELD.spacing,
      position[2] + d[2] * bias * FIELD.spacing,
    ];
    const hit = traceClosest(scene, o, d);
    if (!hit.hit) return { ray, direction: d, hit: false, t: Infinity, radiance: skyRadiance(d) };
    return { ray, direction: d, hit: true, t: hit.t, tri: hit.tri, point: hit.point, radiance: shadeHit(scene, hit, d) };
  });
}

export function traceAllProbes(scene, bias = SELF_SHADOW_BIAS) {
  return probePositions().map((_, index) => traceProbeRays(scene, index, bias));
}

// ---------------------------------------------------------------- 3. atlas blend

/** Octahedral decode, identical to octDecode() in the WGSL. */
export function octDecode(u, v) {
  const fx = u * 2 - 1, fy = v * 2 - 1;
  let nx = fx, ny = fy, nz = 1 - Math.abs(fx) - Math.abs(fy);
  const t = Math.min(Math.max(-nz, 0), 1);
  nx += nx >= 0 ? -t : t;
  ny += ny >= 0 ? -t : t;
  return normalize([nx, ny, nz]);
}

/** Octahedral encode, identical to octEncode() in the WGSL (returns [0,1]^2). */
export function octEncode(n) {
  const l = Math.abs(n[0]) + Math.abs(n[1]) + Math.abs(n[2]) || 1e-6;
  let fx = n[0] / l, fy = n[1] / l;
  if (n[2] < 0) {
    const ax = 1 - Math.abs(fy), ay = 1 - Math.abs(fx);
    fx = (fx >= 0 ? 1 : -1) * ax;
    fy = (fy >= 0 ? 1 : -1) * ay;
  }
  return [fx * 0.5 + 0.5, fy * 0.5 + 0.5];
}

/** Direction of one atlas texel, identical to texelDirection() in the WGSL. */
export function texelDirection(px, py) {
  return octDecode((px - 0.5) / ATLAS.interior, (py - 0.5) / ATLAS.interior);
}

/** Cosine-weighted mean radiance per texel, gamma-5 encoded, then hysteresis mixed. */
export function blendProbe(rays, previous = null, hysteresis = 0) {
  const tile = [];
  for (let py = 0; py < ATLAS.tile; py++) {
    const row = [];
    for (let px = 0; px < ATLAS.tile; px++) {
      const dir = texelDirection(px, py);
      let sum = [0, 0, 0], wsum = 0;
      for (const r of rays) {
        const w = Math.max(dir[0] * r.direction[0] + dir[1] * r.direction[1] + dir[2] * r.direction[2], 0);
        if (w <= 0) continue;
        sum = [sum[0] + r.radiance[0] * w, sum[1] + r.radiance[1] * w, sum[2] + r.radiance[2] * w];
        wsum += w;
      }
      const e = wsum > 1e-5 ? sum.map(s => s / wsum) : [0, 0, 0];
      const encoded = e.map(c => Math.pow(Math.max(c, 1e-6), GAMMA_INV));
      const old = previous ? previous[py][px] : [0, 0, 0];
      row.push(encoded.map((c, i) => c * (1 - hysteresis) + old[i] * hysteresis));
    }
    tile.push(row);
  }
  return tile;
}

/** Distance moments per texel: mean, mean squared (a = hit distance). */
export function blendDistance(rays, previous = null, hysteresis = 0) {
  const tile = [];
  for (let py = 0; py < ATLAS.tile; py++) {
    const row = [];
    for (let px = 0; px < ATLAS.tile; px++) {
      const dir = texelDirection(px, py);
      let d1 = 0, d2 = 0, wsum = 0;
      for (const r of rays) {
        const w = Math.max(dir[0] * r.direction[0] + dir[1] * r.direction[1] + dir[2] * r.direction[2], 0);
        if (w <= 0) continue;
        const dist = Math.min(r.hit ? r.hitDistance : r.t, 30);
        d1 += w * dist; d2 += w * dist * dist; wsum += w;
      }
      const mean = wsum > 1e-5 ? d1 / wsum : 0;
      const mean2 = wsum > 1e-5 ? d2 / wsum : 0;
      const old = previous ? previous[py][px] : [0, 0];
      row.push([
        mean * (1 - hysteresis) + old[0] * hysteresis,
        mean2 * (1 - hysteresis) + old[1] * hysteresis,
      ]);
    }
    row.push;
    tile.push(row);
  }
  return tile;
}

/** Bilinear read of an atlas tile at an interior UV in [0,1], border included. */
export function sampleTileBilinear(tile, u, v) {
  const x = 1 + u * ATLAS.interior;   // same mapping as probeSampleUV() in the WGSL
  const y = 1 + v * ATLAS.interior;
  const x0 = Math.floor(x - 0.5), y0 = Math.floor(y - 0.5);
  const fx = (x - 0.5) - x0, fy = (y - 0.5) - y0;
  const clamp = (i) => Math.min(Math.max(i, 0), ATLAS.tile - 1);
  const a = tile[clamp(y0)][clamp(x0)], b = tile[clamp(y0)][clamp(x0 + 1)];
  const c = tile[clamp(y0 + 1)][clamp(x0)], d = tile[clamp(y0 + 1)][clamp(x0 + 1)];
  return [0, 1, 2].map(i =>
    a[i] * (1 - fx) * (1 - fy) + b[i] * fx * (1 - fy) + c[i] * (1 - fx) * fy + d[i] * fx * fy);
}

// ---------------------------------------------------------------- 4. cage sample

/**
 * One cascade at a world position. Returns { e, w }: e is the cosine-weighted mean
 * radiance (irradiance/PI), w the accumulated weight (0 = outside the field).
 */
export function sampleCascade(radianceAtlas, distanceAtlas, valid, position, normal) {
  const counts = FIELD.counts;
  const spacing = FIELD.spacing;
  const rel = position.map((p, a) => (p - FIELD.origin[a]) / spacing - 0.5);
  for (let a = 0; a < 3; a++) {
    if (rel[a] < -0.5 || rel[a] > counts[a] - 0.5) return { e: [0, 0, 0], w: 0 };
  }
  const base = rel.map(Math.floor);
  const frac = rel.map((r, a) => r - base[a]);
  const biased = position.map((p, a) => p + normal[a] * SELF_SHADOW_BIAS * spacing);
  const nUV = octEncode(normal);
  let e = [0, 0, 0], wsum = 0;
  for (let i = 0; i < 8; i++) {
    const dx = i & 1, dy = (i >> 1) & 1, dz = (i >> 2) & 1;
    const wTri = (dx ? frac[0] : 1 - frac[0]) * (dy ? frac[1] : 1 - frac[1]) * (dz ? frac[2] : 1 - frac[2]);
    if (wTri <= 1e-6) continue;
    const local = [base[0] + dx, base[1] + dy, base[2] + dz];
    if (local.some((v, a) => v < 0 || v > counts[a] - 1)) continue;
    const index = probeIndex(local);
    if (!valid[index]) continue;
    const pp = probePosition(index);
    const toProbe = [pp[0] - position[0], pp[1] - position[1], pp[2] - position[2]];
    const dist = Math.max(Math.hypot(...toProbe), 1e-4);
    const backface = Math.max((normal[0] * toProbe[0] + normal[1] * toProbe[1] + normal[2] * toProbe[2]) / dist, 0);
    if (backface <= 0) continue;
    const moments = sampleTileBilinear(distanceAtlas[index], nUV[0], nUV[1]);
    const d = Math.hypot(biased[0] - pp[0], biased[1] - pp[1], biased[2] - pp[2]);
    const md = Math.max(d - moments[0], 0);
    const sigma2 = Math.max(moments[1] - moments[0] * moments[0], VARIANCE_FLOOR);
    const cheb = Math.min(Math.max(sigma2 / (sigma2 + md * md), 0), 1);
    const w = wTri * backface * cheb;
    const encoded = sampleTileBilinear(radianceAtlas[index], nUV[0], nUV[1]);
    const linearE = encoded.map(c => Math.pow(Math.max(c, 0), GAMMA));
    for (let c = 0; c < 3; c++) e[c] += linearE[c] * w;
    wsum += w;
  }
  if (wsum <= 1e-5) return { e: [0, 0, 0], w: 0 };
  return { e: e.map(v => v / wsum), w: wsum };
}

// ---------------------------------------------------------------- 5. fragment shade

export function tonemapACES(x) {
  const a = 2.51, b = 0.03, c = 2.43, d = 0.59, e = 0.14;
  return x.map(v => Math.min(Math.max((v * (a * v + b)) / (v * (c * v + d) + e), 0), 1));
}

export function sRGBEncode(c) {
  return c.map(v => v <= 0.0031308 ? v * 12.92 : 1.055 * Math.pow(Math.max(v, 0), 0.41666) - 0.055);
}

/** Direct-only shading of a surface point (the same formula the fragment uses). */
export function shadeDirect(scene, point, normal, albedo) {
  const toLight = [
    LIGHT.position[0] - point[0], LIGHT.position[1] - point[1], LIGHT.position[2] - point[2],
  ];
  const dist = Math.hypot(...toLight);
  const l = toLight.map(v => v / dist);
  const ndotl = normal[0] * l[0] + normal[1] * l[1] + normal[2] * l[2];
  if (ndotl <= 0) return [0, 0, 0];
  const o = point.map((p, a) => p + normal[a] * 0.02);
  const hit = traceClosest(scene, o, l, dist - 0.05);
  if (hit.hit) return [0, 0, 0];
  const e = LIGHT.intensity / (1 + LIGHT.falloffK * dist * dist) * ndotl;
  return albedo.map(a => a * e / PI);
}

/**
 * Full fragment evaluation at a surface point, matching the WGSL fragment:
 * colour = albedo * (direct/PI + ind.e * ao), then ACES and the sRGB OETF.
 */
export function shadeFragment(scene, radianceAtlas, distanceAtlas, valid, point, normal, albedo, ao = 1) {
  const direct = shadeSurfaceDirect(scene, point, normal, albedo).map(v => v * PI); // irradiance form
  const ind = sampleCascade(radianceAtlas, distanceAtlas, valid, point, normal);
  const color = albedo.map((a, c) => a * (direct[c] / PI + ind.e[c] * ao));
  const mapped = tonemapACES(color);
  return { color, mapped, srgb: sRGBEncode(mapped), ind, direct };
}

/** Whole CPU pipeline in the order the GPU runs it. */
export function runPipeline(scene, { bias = SELF_SHADOW_BIAS, hysteresis = HYSTERESIS } = {}) {
  const positions = probePositions();
  const valid = positions.map(() => true);
  const rays = traceAllProbes(scene, bias);
  const radianceAtlas = rays.map(r => blendProbe(r, null, 0));          // frame 1: fresh
  const distanceAtlas = rays.map(r => blendDistance(r, null, 0));
  const radianceAtlas2 = rays.map((r, i) => blendProbe(r, radianceAtlas[i], hysteresis));
  const distanceAtlas2 = rays.map((r, i) => blendDistance(r, distanceAtlas[i], hysteresis));
  return { positions, valid, rays, radianceAtlas, distanceAtlas, radianceAtlas2, distanceAtlas2 };
}
