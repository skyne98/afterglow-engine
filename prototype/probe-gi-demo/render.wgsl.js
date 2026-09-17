// Probe-GI demo render pass (WGSL).
// Forward shading: direct point light with a BVH shadow ray (the same BVH the
// probes trace against), plus indirect diffuse sampled from the probe atlas with
// trilinear cage weights x backface weight x Chebyshev visibility.

export const RENDER_WGSL = /* wgsl */ `
struct Camera {
  viewProj: mat4x4f,
  eye: vec4f,
};

struct Params {
  counts: vec4u,
  tile: vec4u,
  atlas: vec4u,
  misc: vec4f,          // x raysPerProbe, y hysteresis, z selfShadowBias, w sky intensity
  cascade0: vec4f,
  cascade1: vec4f,
  cascade2: vec4f,
  light: vec4f,
};

@group(0) @binding(0) var<storage, read> nodes: array<vec4f>;
@group(0) @binding(1) var<storage, read> triPos: array<vec4f>;
@group(0) @binding(2) var<storage, read> triIdx: array<u32>;
@group(0) @binding(3) var<storage, read> rights: array<i32>;
@group(0) @binding(4) var<storage, read> probePos: array<vec4f>;
@group(0) @binding(5) var<uniform> P: Params;
@group(0) @binding(6) var radianceAtlas: texture_2d<f32>;
@group(0) @binding(7) var distanceAtlas: texture_2d<f32>;
@group(0) @binding(8) var atlasSampler: sampler;
@group(1) @binding(0) var<uniform> cam: Camera;

// Diagnostic view selector, bound by demo.js. x = mode, y = cascade mask, zw reserved.
//   0 combined (normal)   2 indirect irradiance   3 cage weight sum
//   4 indirect from one cascade (DBG.y)   6..8 drop classes of outside probe (leak test)
@group(1) @binding(1) var<uniform> DBG: vec4f;

const PI: f32 = 3.141592653589793;

// Display exposure, applied before the tone curve (the renderer-level equivalent of
// Three's toneMappingExposure / an AgX exposure factor).
const EXPOSURE: f32 = 1.0;

/** ACES filmic tone curve (Narkowicz fit). Three ships this shape as
 *  ACESFilmicToneMapping; the demo hand-rolls it because it owns no renderer output
 *  pipeline. Replaces a plain x/(1+x), which has no shoulder and never reaches white. */
fn tonemapACES(x: vec3f) -> vec3f {
  let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
  return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3f(0.0), vec3f(1.0));
}

/** Piecewise linear-sRGB -> sRGB, identical to Three's sRGBTransferOETF. A plain
 *  pow(c, 1/2.2) is NOT sRGB: it lifts darks by 3x at c = 0.001 and 1.5x at c = 0.01,
 *  which is what made every frame read flat. */
fn sRGBEncode(c: vec3f) -> vec3f {
  let lo = c * 12.92;
  let hi = 1.055 * pow(max(c, vec3f(0.0)), vec3f(0.41666)) - 0.055;
  return select(hi, lo, c <= vec3f(0.0031308));
}

fn slab(o: vec3f, invD: vec3f, bmin: vec3f, bmax: vec3f) -> vec2f {
  let t0 = (bmin - o) * invD;
  let t1 = (bmax - o) * invD;
  return vec2f(
    max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z)),
    min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z)),
  );
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

/** First blocking hit distance, or -1 when the ray is clear (diagnostic). */
fn occludedT(o: vec3f, d: vec3f, tMax: f32) -> f32 {
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
    let tt = slab(o, invD, nMin, nMax);
    if (tt.x > tt.y || tt.y < 0.001 || tt.x > tMax) { continue; }
    if (triCount > 0u) {
      var k = 0u;
      loop {
        if (k >= triCount) { break; }
        let h = hitTri(o, d, triIdx[leftFirst + k], tMax);
        if (h > 0.0) { return h; }
        k = k + 1u;
      }
    } else {
      stack[sp] = u32(rights[ni]); sp = sp + 1u;
      stack[sp] = leftFirst; sp = sp + 1u;
    }
  }
  return -1.0;
}

fn occluded(o: vec3f, d: vec3f, tMax: f32) -> bool {
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
    let tt = slab(o, invD, nMin, nMax);
    if (tt.x > tt.y || tt.y < 0.001 || tt.x > tMax) { continue; }
    if (triCount > 0u) {
      var k = 0u;
      loop {
        if (k >= triCount) { break; }
        if (hitTri(o, d, triIdx[leftFirst + k], tMax) > 0.0) { return true; }
        k = k + 1u;
      }
    } else {
      stack[sp] = u32(rights[ni]); sp = sp + 1u;
      stack[sp] = leftFirst; sp = sp + 1u;
    }
  }
  return false;
}

/** Geometric normal of a triangle, from the same buffer the shadow test traverses. */
fn triNormalOf(tri: u32) -> vec3f {
  let a = triPos[tri * 3u].xyz;
  let b = triPos[tri * 3u + 1u].xyz;
  let c = triPos[tri * 3u + 2u].xyz;
  return normalize(cross(b - a, c - a));
}

fn octEncode(n: vec3f) -> vec2f {
  let l = abs(n.x) + abs(n.y) + abs(n.z);
  var f = n.xy / max(l, 1e-6);
  if (n.z < 0.0) {
    f = (vec2f(1.0) - abs(f.yx)) * select(vec2f(-1.0), vec2f(1.0), f >= vec2f(0.0));
  }
  return f * 0.5 + vec2f(0.5);
}

fn cascadeOrigin(c: u32) -> vec4f {
  if (c == 0u) { return P.cascade0; }
  if (c == 1u) { return P.cascade1; }
  return P.cascade2;
}

/** Atlas UV for a probe-interior UV in [0,1] (border texels give the bilinear margin). */
const OCT_SIZE: u32 = 8u;

/** Atlas tile origin of one probe, in atlas texels. */
fn atlasTileOrigin(probe: u32, tileSize: u32, slotsPerRow: u32) -> vec2i {
  let sx = probe % slotsPerRow;
  let sy = probe / slotsPerRow;
  return vec2i(i32(sx * tileSize), i32(sy * tileSize));
}

/** One hardware-filtered octahedral fetch of a probe's tile.
 *
 *  Tile texel 0 and interior+1 are the border; the interior texels are 1..interior and
 *  tile-local texel px holds the direction (px - 0.5) / interior, so its atlas centre is
 *  px + 0.5 and the coordinate is slot*tile + 1.0 + uv*interior. That is exact at texel
 *  centres, and at the interior's edges (uv = 0 or 1) it lands half a texel outside, on
 *  the border texel - which is exactly what the border is for: the bilinear footprint
 *  needs a neighbour texel for the interpolation to stay continuous across the
 *  octahedral seam. (A "+1.5 centre-only" variant looks tidier and is half a texel wrong;
 *  selftest.mjs pins the mapping with a round trip against the blend's texel directions.) */
fn atlasFetch(atlas: texture_2d<f32>, probe: u32, tileSize: u32, slotsPerRow: u32,
  atlasSize: f32, interior: u32, dir: vec3f) -> vec4f {
  let uv = octEncode(dir);
  let origin = atlasTileOrigin(probe, tileSize, slotsPerRow);
  let texel = vec2f(origin) + vec2f(1.0) + uv * f32(interior);
  return textureSampleLevel(atlas, atlasSampler, texel / atlasSize, 0.0);
}

struct ProbeIrradiance { e: vec3f, w: f32, fade: f32 };

/** One cascade's contribution at a world position. Returns weight 0 if outside. */
fn sampleCascade(c: u32, pos: vec3f, n: vec3f) -> ProbeIrradiance {
  let co = cascadeOrigin(c);
  let spacing = co.w;
  let ext = vec3f(P.counts.xyz) * spacing;
  let rel = (pos - co.xyz) / spacing - vec3f(0.5);
  if (any(rel < vec3f(-0.5)) || any(rel > vec3f(P.counts.xyz) - vec3f(0.5))) {
    return ProbeIrradiance(vec3f(0.0), 0.0, 0.0);
  }
  // Distance to this cascade's boundary, in cells. Adjacent cascades must BLEND across
  // an overlap: switching hard at the edge makes the seam visible as a jagged step,
  // because the two cascades resolve different geometry at different densities.
  let toEdge = min(rel + vec3f(0.5), vec3f(P.counts.xyz) - vec3f(0.5) - rel);
  let edge = min(toEdge.x, min(toEdge.y, toEdge.z));
  // The handover ramp spans TWO AND A HALF cells, not one. Adjacent cascades disagree in
  // level - a 3 m cage inside a room sees the interior differently from a 1 m one - so a
  // one-cell ramp leaves a visible band whose edge reads as a dark rectangle tracking the
  // camera on a floor. Spreading it costs nothing (the ramp is a weight) and lets the two
  // levels cross gradually.
  let fade = smoothstep(0.0, 2.5, edge);
  let base = floor(rel);
  let frac = rel - base;
  let biasedPos = pos + n * (P.misc.z * spacing);
  var e = vec3f(0.0);
  var wsum = 0.0;
  let tilesX = P.atlas.z;
  for (var i = 0u; i < 8u; i = i + 1u) {
    let dx = f32(i & 1u);
    let dy = f32((i >> 1u) & 1u);
    let dz = f32((i >> 2u) & 1u);
    let tri = vec3f(mix(1.0 - frac.x, frac.x, dx), mix(1.0 - frac.y, frac.y, dy), mix(1.0 - frac.z, frac.z, dz));
    let wTri = tri.x * tri.y * tri.z;
    if (wTri <= 1e-6) { continue; }
    let local = base + vec3f(dx, dy, dz);
    if (any(local < vec3f(0.0)) || any(local > vec3f(P.counts.xyz) - vec3f(1.0))) { continue; }
    let li = vec3u(local);
    let probe = c * (P.counts.x * P.counts.y * P.counts.z)
      + (li.z * P.counts.y + li.y) * P.counts.x + li.x;
    let pp = probePos[probe];
    if (pp.w < 0.5) { continue; }   // deactivated probes contribute nothing
    // DIAGNOSTIC (DBG.x 6..8): which class of outside probe carries the leak.
    //   6 = drop everything outside the room AABB   7 = drop only probes above the roof
    //   8 = drop only probes outside in x/z
    if (DBG.x > 5.5) {
      let outsideXZ = any(pp.xyz < vec3f(-8.0, -1e9, -8.0)) || any(pp.xyz > vec3f(8.0, 1e9, 8.0));
      let aboveRoof = pp.y > 4.25;
      let belowFloor = pp.y < 0.0;
      var drop = false;
      if (DBG.x < 6.5) { drop = outsideXZ || aboveRoof || belowFloor; }
      else if (DBG.x < 7.5) { drop = aboveRoof; }
      else { drop = outsideXZ; }
      if (drop) { continue; }
    }
    let toProbe = pp.xyz - pos;
    let dist = max(length(toProbe), 1e-4);
    // Wrapped normal weight: ((dot+1)/2)^2 + 0.04. A hard max(dot, 0) cutoff, which is
    // what this used to be, drops side probes entirely and darkens exactly the surfaces
    // where the cage is one-sided; the wrapped form never reaches zero.
    let wrap = max(0.0, (dot(n, toProbe / dist) + 1.0) * 0.5);
    var w = wTri * (wrap * wrap + 0.04);
    // Chebyshev visibility from the pow(cos,16) distance moments, read along the surface
    // normal (RTXGI's convention).
    //
    // Reading them along the query->probe direction is the geometrically right form - it can
    // see a wall between the point and the probe - and it gates out the probes behind walls
    // in the CPU reference (probecheck.mjs, exact cone integrals). Re-measured on the GPU
    // after the atlas-fetch fix, it moves the sealed-room leak only 80.5 -> 78.2 mean luma:
    // the 8x8 distance map is bilinearly interpolated, which smears the moments the CPU
    // function computes exactly. ~3 % is not worth making every fetch depend on the pixel, so
    // the normal direction stays and the visible fix for the cage was the nested-cascade
    // weight below.
    let visDir = toProbe / dist;
    let moments = atlasFetch(distanceAtlas, probe, P.tile.w, P.atlas.w, f32(P.atlas.y),
      P.tile.z, visDir).rg;
    let mean = moments.r;
    let mean2 = moments.g;
    let d = distance(biasedPos, pp.xyz);
    let md = max(d - mean, 0.0);
    let sigma2 = max(mean2 - mean * mean, 0.002);
    let cheb = clamp(sigma2 / (sigma2 + md * md), 0.0, 1.0);
    // The gate is used UN cubed and unsharpened. Both amplifiers are in the sampled
    // reference for leak suppression, and here they produced the artifact the user
    // reported: bright blobs at cage-cell centres with dark seams at the cell boundaries,
    // tiling every dim surface. The cause is positional - a query near a cell boundary is
    // up to 1.5 spacings from its cage probes, so their Chebyshev distance term is larger
    // and the shared weight dips there - and squaring/cubing that dip three times, then
    // raising small weights to a third power, turns a gentle lattice into hard-edged
    // blocks. Measured leak benefit in this engine: none (sealing the room's doorway and
    // comparing Chebyshev on/off moved the leaking wall by 1 %), so the amplifiers cost
    // visible quality for nothing. See §7.6.
    w = w * cheb;
    // The atlas stores irradiance E, so the surface reflects albedo * E / PI (BRDF
    // Lambert). Dividing by PI here again, as this did while the atlas held a mean
    // radiance, is what made the whole GI contribution vanish into quantization.
    let linearE = atlasFetch(radianceAtlas, probe, P.tile.y, tilesX, f32(P.atlas.x),
      P.tile.x, n).rgb;
    e = e + linearE * w;
    wsum = wsum + w;
  }
  if (wsum <= 1e-5) { return ProbeIrradiance(vec3f(0.0), 0.0, fade); }
  return ProbeIrradiance(e / wsum, wsum, fade);
}

struct VSOut {
  @builtin(position) clip: vec4f,
  @location(0) wpos: vec3f,
  @location(1) nrm: vec3f,
  @location(2) albedo: vec3f,
};

/** Orientation-driven albedo, identical to the GI path's rule. */
fn triAlbedoOf(tri: u32) -> vec3f {
  let n = triNormalOf(tri);
  let h = fract(sin(f32(tri) * 12.9898) * 43758.5453);
  let horizontal = step(0.7, abs(n.y));
  let floorCol = vec3f(0.30, 0.29, 0.28);
  let wallCol = vec3f(0.40, 0.38, 0.36);
  let crateCol = vec3f(0.46, 0.33, 0.22);
  let base = mix(wallCol, floorCol, horizontal);
  let a = triPos[tri * 3u].xyz;
  let warm = clamp((a.y - 1.0) * 0.10, 0.0, 0.18);
  return mix(base, crateCol, warm);
}

@vertex
fn vs(
  @location(0) p: vec3f,
  @location(1) n: vec3f,
  @location(2) a: vec3f,
) -> VSOut {
  var out: VSOut;
  out.clip = cam.viewProj * vec4f(p, 1.0);
  out.wpos = p;
  out.nrm = n;
  out.albedo = a;
  return out;
}

@fragment
fn fs(in: VSOut) -> @location(0) vec4f {
  // Two-sided shading, from the triangle the traversal also uses, so direct and
  // indirect shading agree on every surface by construction.
  var n = normalize(in.nrm);
  if (dot(n, in.wpos - cam.eye.xyz) > 0.0) { n = -n; }

  // Direct: point light with one shadow ray through the same BVH. Reads the same
  // uniform the probe trace reads, so direct and indirect shading agree by
  // construction (this used to be a hardcoded position from a bisect).
  let lightPos = P.light.xyz;
  let toLight = lightPos - in.wpos;
  let lightDist = length(toLight);
  let l = toLight / max(lightDist, 1e-4);
  var direct = vec3f(0.0);
  if (dot(n, l) > 0.0 && !occluded(in.wpos + n * 0.02, l, lightDist - 0.05)) {
    direct = vec3f(1.0, 0.86, 0.68) * (max(dot(n, l), 0.0) * P.light.w / (1.0 + 0.09 * lightDist * lightDist));
  }

  // Indirect: innermost cascade that contains the point, else the outermost.
  // ind.e is a cosine-weighted mean radiance (the probe blend already divides by
  // the cone's total cosine weight), so it is the irradiance/PI term itself: divide
  // by PI again and the whole GI contribution lands below 8-bit quantization.
  var ind = ProbeIrradiance(vec3f(0.0), 0.0, 0.0);
  var masked = ProbeIrradiance(vec3f(0.0), 0.0, 0.0);
  var accE = vec3f(0.0);
  var accW = 0.0;
  // Nested-cascade blend: each cascade claims only the share the finer ones left it.
  //
  // Using each cascade's own fade as its share (which this did) means a coarse cascade
  // whose own box is huge is at fade ~1 for a point deep inside the fine cascade, so it
  // outvoted the fine cascade: a floor point 1 m from the camera took 74 % of its light
  // from the 9 m cascade measured at E = 1.27 against the honest in-room 0.08, i.e. the
  // floor was ~7x too bright and its brightness depended on where the camera was. Weighting
  // by the fade also cannot be fixed by normalising, because the shares do not sum to 1.
  var prev = 1.0;
  for (var c = 0u; c < 3u; c = c + 1u) {
    let s = sampleCascade(c, in.wpos, n);
    if (DBG.y >= 0.0 && f32(c) == DBG.y && s.w > 0.0) { masked = s; }
    if (s.w <= 0.0) { continue; }
    let share = s.fade * prev;
    if (share <= 1e-6) { continue; }
    accE = accE + s.e * share;
    accW = accW + share;
    prev = prev * (1.0 - s.fade);
    // A point fully inside a cascade needs no blend with the coarse one above it, and
    // skipping that keeps the common case at one cascade's worth of fetches.
    if (prev <= 1e-3) { break; }
  }
  if (accW > 1e-5) { ind = ProbeIrradiance(accE / accW, accW, 1.0); }
  // mode 9: albedo only (is the surface itself dark?)   mode 5: R = direct, G = indirect
  if (DBG.x > 8.5) { return vec4f(saturate(in.albedo), 1.0); }
  if (DBG.x > 4.5 && DBG.x < 5.5) {
    return vec4f(saturate(direct.x * 0.25), saturate(ind.e.x), 0.5, 1.0);
  }
  if (DBG.x > 0.5 && DBG.x < 4.5) {
    // Diagnostic modes, written directly (no tone curve) so the numbers are the values.
    if (DBG.x < 2.5) { let v = ind.e / (ind.e + vec3f(1.0)); return vec4f(v, 1.0); }
    if (DBG.x < 3.5) { let v = clamp(ind.w, 0.0, 1.0); return vec4f(v, v, v, 1.0); }
    let v = masked.e / (masked.e + vec3f(1.0));
    return vec4f(v, 1.0);
  }

  // No direct sky/ambient term: all ambient arrives through the probes, which is the
  // only way this render can demonstrate that probe GI is doing anything. Kill the
  // probe trace and the ambient goes to zero instead of being papered over by
  // a direct skyRadiance(n) ambient term.
  //
  // Units: the direct term is an irradiance and ind.e is the probe blend's cosine-
  // weighted mean radiance, which is that same irradiance/PI. Diffuse out is
  // albedo/PI * E, so both terms are divided by PI exactly once here. (Without the
  // /PI on the direct term the punctual term is PI x too strong relative to the
  // indirect one and no exposure makes the GI readable.)
  // ind.e is irradiance, so both terms are reflected as albedo * E / PI.
  var color = in.albedo * ((direct + ind.e) / PI);
  color = tonemapACES(color * EXPOSURE);
  return vec4f(sRGBEncode(color), 1.0);
}
`;
