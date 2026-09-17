// Formulas sampled from "The Lantern Vault" (single-file Three.js/WebGPU DDGI, three
// 0.180.0), implemented here so each one can be checked against known properties before
// anything is adopted. Their file is the DDGI convention, which differs from ours in
// four places that matter:
//
//   1. the atlas stores IRRADIANCE (E), not E/PI:
//        integrate: color *= PI / cosineSum
//        surface:   radiance = albedo * E / PI
//      Ours stored the cosine-weighted mean radiance and multiplied by albedo, which is
//      the same number but makes "is this irradiance or radiance" ambiguous at every
//      call site. Their convention is self-documenting and matches Three's BRDF_Lambert.
//   2. no border texels: an 8x8 octahedral map with a SEAM-MIRRORING fetch
//      (ddOctWrap), instead of a 1-texel gutter. This is what removes the half-texel
//      border sampling error the old demo had (`tile + 1.0 + uv*interior` lands on a
//      texel edge, not a centre).
//   3. the per-fragment cage weight is a wrapped normal weight that never reaches zero
//      (`((dot+1)/2)^2 + 0.04`) plus a sharpening term below 0.2, instead of a hard
//      `max(dot, 0)` backface cutoff.
//   4. leak suppression is three-fold: Chebyshev cubed, the weight sharpening, and a
//      probe-position bias of 0.19 * n. (We added a ray-traced AO pass to fix the same
//      enclosed-room leak; these act on the sample instead.)
//
// Their distance-moment filter uses pow(cosine, 16) rather than the irradiance filter's
// cosine, which is also the DDGI prescription.

export const OCT = 8;
export const TEXELS = OCT * OCT;

/** Exact port of ddOctWrap: mirror an out-of-range octahedral texel across the seam. */
export function octWrap(x, y) {
  let cx = x, cy = y;
  if (cx < 0) { cx = -cx - 1; cy = 7 - cy; }
  if (cx > 7) { cx = 15 - cx; cy = 7 - cy; }
  if (cy < 0) { cx = 7 - cx; cy = -cy - 1; }
  if (cy > 7) { cx = 7 - cx; cy = 15 - cy; }
  return clamp(cx, 0, 7) + 8 * clamp(cy, 0, 7);
}

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);

/** Octahedral encode, byte-identical to their ddOctEncode. */
export function octEncode(d) {
  const l = Math.max(Math.abs(d[0]) + Math.abs(d[1]) + Math.abs(d[2]), 0.00001);
  const v = [d[0] / l, d[1] / l, d[2] / l];
  let p = [v[0], v[1]];
  if (v[2] < 0) p = [(1 - Math.abs(p[1])) * (p[0] >= 0 ? 1 : -1), (1 - Math.abs(p[0])) * (p[1] >= 0 ? 1 : -1)];
  return [p[0] * 0.5 + 0.5, p[1] * 0.5 + 0.5];
}

/**
 * Bilinear octahedral fetch with seam mirroring: no border texels, no edge sampling.
 * `cache` is a flat Float32Array of vec4 texels (`stride` floats each); rgb is returned.
 */
export function fetch(cache, base, dir, stride = 4) {
  const uv = octEncode(dir);
  const px = uv[0] * 8 - 0.5, py = uv[1] * 8 - 0.5;
  const ix = Math.floor(px), iy = Math.floor(py);
  const fx = px - ix, fy = py - iy;
  const at = (x, y) => {
    const o = (base + octWrap(x, y)) * stride;
    return [cache[o], cache[o + 1], cache[o + 2]];
  };
  const a = at(ix, iy), b = at(ix + 1, iy), c = at(ix, iy + 1), d = at(ix + 1, iy + 1);
  const lerp = (u, v, t) => u + (v - u) * t;
  return [0, 1, 2].map(k => lerp(lerp(a[k], b[k], fx), lerp(c[k], d[k], fx), fy));
}

const mix = (a, b, t) => a + (b - a) * t;

/** Per-fragment cage weight: trilinear x wrapped normal x Chebyshev^3, then sharpened. */
export function cageWeight(bary, dirToProbe, n, chebyshev) {
  let weight = bary;
  const wrap = Math.max(0, (dot(dirToProbe, n) + 1) * 0.5);
  weight *= wrap * wrap + 0.04;
  if (chebyshev !== null) weight *= chebyshev * chebyshev * chebyshev;
  if (weight < 0.2) weight *= weight * weight / 0.04;
  return weight;
}

const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

/** Chebyshev visibility from distance moments, with their variance floor. */
export function chebyshevVisibility(moment, distance) {
  const variance = Math.max(moment[1] - moment[0] * moment[0], 0.002);
  const delta = Math.max(0, distance - moment[0]);
  return variance / (variance + delta * delta);
}

/**
 * Their integrate step for one texel: cosine convolution normalised so a constant
 * radiance field integrates to exactly PI * L, plus pow(cos,16)-weighted distance
 * moments.
 */
export function integrateTexel(rays, axis, hysteresis, previous) {
  let color = [0, 0, 0], cosineSum = 0, m1 = 0, m2 = 0, momentSum = 0;
  for (const r of rays) {
    const cosine = Math.max(0, dot(axis, r.direction)) * (r.valid ?? 1);
    color = color.map((c, i) => c + r.radiance[i] * cosine);
    cosineSum += cosine;
    const mw = Math.pow(cosine, 16);
    m1 += r.distance * mw; m2 += r.distance * r.distance * mw; momentSum += mw;
  }
  color = color.map(c => c * Math.PI / Math.max(cosineSum, 0.00001));
  const moments = [m1 / Math.max(momentSum, 0.00001), m2 / Math.max(momentSum, 0.00001)];
  const old = previous ?? null;
  let h = old ? hysteresis : 0;
  if (old) {
    // length(color - old) / max(length(old), 0.08): the magnitude of the difference.
    const diff = color.map((c, i) => c - old.color[i]);
    const change = Math.hypot(...diff) / Math.max(Math.hypot(...old.color), 0.08);
    if (change > 0.5) h = Math.min(h, 0.70);
  }
  let mh = Math.min(hysteresis, 0.88);
  if (!old) mh = 0;
  else if (Math.abs(moments[0] - old.moments[0]) > 0.7) mh = Math.min(mh, 0.60);
  return {
    color: old ? color.map((c, i) => mix(c, old.color[i], h)) : color,
    moments: old ? moments.map((m, i) => mix(m, old.moments[i], mh)) : moments,
    raw: color,
  };
}
