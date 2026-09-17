// Probe-GI compute passes (WGSL), split into modules because the atlas storage
// texture formats differ per pass:
//   giCore   - probeInit (validity/virtual offset/deactivation) + probeTrace
//   giBlendR - blendRadiance  -> rgba16float atlas
//   giBlendD - blendDistance  -> rg16float atlas
//
// Layout notes:
//   * the atlas is a 2D packed tile grid, NOT a texture array, so resident
//     capacity never depends on a raised maxTextureArrayLayers limit.
//   * every probe tile carries a 1-texel border so bilinear filtering crosses the
//     octahedral seam without touching a neighbour probe.
//   * gamma-5 perception encoding matches RTXGI; the atlas stays rgba16float.

const PRELUDE = /* wgsl */ `
struct Params {
  counts: vec4u,          // x probesX, y probesY, z probesZ, w total probes
  tile: vec4u,            // x radiance interior, y radiance tile, z distance interior, w distance tile
  atlas: vec4u,           // x radiance size, y distance size, z radiance slots/row, w distance slots/row
  misc: vec4f,            // x raysPerProbe, y hysteresis, z selfShadowBias (spacing units), w sky intensity
  cascade0: vec4f,        // xyz min corner, w spacing
  cascade1: vec4f,
  cascade2: vec4f,
  light: vec4f,           // xyz position, w intensity
};

@group(0) @binding(0) var<storage, read> nodes: array<vec4f>;    // 2 vec4 per node
@group(0) @binding(1) var<storage, read> triPos: array<vec4f>;  // 3 vec4 per triangle
@group(0) @binding(2) var<storage, read> triIdx: array<u32>;
@group(0) @binding(3) var<storage, read> rights: array<i32>;
@group(0) @binding(4) var<storage, read_write> probePos: array<vec4f>;  // xyz world, w valid
@group(0) @binding(5) var<storage, read_write> probeFlags: array<u32>;  // bit0 valid, bit1 freshly seeded
@group(0) @binding(6) var<storage, read_write> rayData: array<vec4f>;   // rgb radiance, a hit distance
@group(0) @binding(7) var<uniform> P: Params;
/** Per-probe index whose PREVIOUS atlas a reseeded probe should inherit, or 0xffffffff. */
@group(0) @binding(8) var<storage, read> seedFrom: array<u32>;

const PI: f32 = 3.141592653589793;
const RAY_MISS: f32 = 1e4;
const RAY_CAP: f32 = 30.0;   // max distance stored in the visibility moments

fn cascadeOf(probe: u32) -> u32 {
  return probe / (P.counts.x * P.counts.y * P.counts.z);
}

fn cascadeOrigin(c: u32) -> vec4f {
  if (c == 0u) { return P.cascade0; }
  if (c == 1u) { return P.cascade1; }
  return P.cascade2;
}

fn probeCenter(probe: u32) -> vec3f {
  let perCascade = P.counts.x * P.counts.y * P.counts.z;
  let local = probe % perCascade;
  let ix = f32(local % P.counts.x);
  let iy = f32((local / P.counts.x) % P.counts.y);
  let iz = f32(local / (P.counts.x * P.counts.y));
  let c = cascadeOrigin(cascadeOf(probe));
  return c.xyz + vec3f(ix, iy, iz) * c.w;
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

struct Hit { t: f32, tri: u32 };

fn traverseClosest(o: vec3f, d: vec3f, tMax: f32) -> Hit {
  var stack: array<u32, 40>;
  var sp = 0u;
  var bestT = tMax;
  var bestTri = 0xffffffffu;
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
    if (tt.x > tt.y || tt.y < 0.001 || tt.x > bestT) { continue; }
    if (triCount > 0u) {
      var k = 0u;
      loop {
        if (k >= triCount) { break; }
        let tri = triIdx[leftFirst + k];
        let h = hitTri(o, d, tri, bestT);
        if (h > 0.0) { bestT = h; bestTri = tri; }
        k = k + 1u;
      }
    } else {
      stack[sp] = u32(rights[ni]); sp = sp + 1u;
      stack[sp] = leftFirst; sp = sp + 1u;
    }
  }
  return Hit(bestT, bestTri);
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

fn skyRadiance(d: vec3f) -> vec3f {
  // Near-neutral sky: a saturated blue here tints every indirect-lit surface.
  let up = clamp(d.y * 0.5 + 0.5, 0.0, 1.0);
  return mix(vec3f(0.10, 0.10, 0.11), vec3f(0.30, 0.32, 0.36), up) * P.misc.w;
}

fn triNormal(i: u32) -> vec3f {
  let a = triPos[i * 3u].xyz;
  let b = triPos[i * 3u + 1u].xyz;
  let c = triPos[i * 3u + 2u].xyz;
  return normalize(cross(b - a, c - a));
}

fn triAlbedo(tri: u32) -> vec3f {
  // Orientation-driven albedo so floors, walls and crates read as distinct
  // surfaces instead of per-triangle noise.
  let n = triNormal(tri);
  let h = fract(sin(f32(tri) * 12.9898) * 43758.5453);
  let horizontal = step(0.7, abs(n.y));
  let floorCol = vec3f(0.30, 0.29, 0.28);
  let wallCol = vec3f(0.40, 0.38, 0.36);
  let crateCol = vec3f(0.46, 0.33, 0.22);
  let base = mix(wallCol, floorCol, horizontal);
  // y-height tint: a little variation so tall geometry reads warmer
  let a = triPos[tri * 3u].xyz;
  let warm = clamp((a.y - 1.0) * 0.10, 0.0, 0.18);
  return mix(base, crateCol, warm);
}

fn shadePoint(pos: vec3f, n: vec3f, tri: u32) -> vec3f {
  let toLight = P.light.xyz - pos;
  let dist = length(toLight);
  let l = toLight / max(dist, 1e-4);
  var direct = 0.0;
  if (dot(n, l) > 0.0) {
    if (!occluded(pos + n * 0.02, l, dist - 0.05)) {
      direct = max(dot(n, l), 0.0) * (P.light.w / (1.0 + 0.09 * dist * dist));
    }
  }
  // Diffuse radiance leaving the hit: albedo/PI * irradiance.
  //
  // No ambient/sky term here. The sky reaches a probe only through its *miss* rays,
  // where visibility is known. Adding skyRadiance(n) to every hit - which is what this
  // used to do - hands sky-level radiance to surfaces that cannot see the sky at all,
  // so a roofed room was lit as if it were open (the interior walls rendered a flat
  // sky-coloured value regardless of the light). One bounce, with the sky only where
  // the ray actually escaped, is what makes the interior darkness meaningful.
  let albedo = triAlbedo(tri);
  return albedo * (vec3f(1.0, 0.86, 0.68) * direct / PI);
}

fn rayDirection(r: u32, n: f32) -> vec3f {
  let i = f32(r) + 0.5;
  let cosTheta = 1.0 - 2.0 * i / n;
  let theta = 2.0 * PI * i * 0.6180339887498949;
  let sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
  return vec3f(cos(theta) * sinTheta, sin(theta) * sinTheta, cosTheta);
}

fn octEncode(n: vec3f) -> vec2f {
  let l = abs(n.x) + abs(n.y) + abs(n.z);
  var f = n.xy / max(l, 1e-6);
  if (n.z < 0.0) {
    f = (vec2f(1.0) - abs(f.yx)) * select(vec2f(-1.0), vec2f(1.0), f >= vec2f(0.0));
  }
  return f * 0.5 + vec2f(0.5);
}

fn octDecode(uv: vec2f) -> vec3f {
  let f = uv * 2.0 - vec2f(1.0, 1.0);
  var n = vec3f(f.x, f.y, 1.0 - abs(f.x) - abs(f.y));
  let t = clamp(-n.z, 0.0, 1.0);
  n.x = n.x + select(t, -t, n.x >= 0.0);
  n.y = n.y + select(t, -t, n.y >= 0.0);
  return normalize(n);
}

/** Direction of one tile-local texel. Tile-local 0 and interior+1 are the border, so the
 *  interior texels are 1..interior and the direction is (px - 0.5) / interior: texel 1
 *  carries 0.5/interior and the last interior texel carries (interior - 0.5)/interior,
 *  which is the convention the render-side fetch inverts. */
fn texelDirection(px: u32, py: u32, interior: u32) -> vec3f {
  let u = (f32(px) - 0.5) / f32(interior);
  let v = (f32(py) - 0.5) / f32(interior);
  return octDecode(vec2f(u, v));
}

/** Atlas texel coordinate of a tile-local texel (the blend writes every tile texel,
 *  border included, so no separate border pass is needed). */
fn atlasTexel(probe: u32, px: u32, py: u32, tileSize: u32, slotsPerRow: u32) -> vec2i {
  let sx = probe % slotsPerRow;
  let sy = probe / slotsPerRow;
  return vec2i(i32(sx * tileSize + px), i32(sy * tileSize + py));
}

`;

export const GI_CORE_WGSL = PRELUDE + /* wgsl */ `
@compute @workgroup_size(64)
fn probeInit(@builtin(global_invocation_id) gid: vec3u) {
  let probe = gid.x;
  if (probe >= P.counts.w) { return; }
  // Base position is seeded by the host (grid-aligned, re-seeded on scroll);
  // this pass relocates it out of geometry and records validity.
  let center = probePos[probe].xyz;
  let keepFresh = probeFlags[probe] & 2u;
  let spacing = cascadeOrigin(cascadeOf(probe)).w;
  let reach = spacing * 1.5;

  var enclosed = 0u;
  var farthest = -1.0;
  var farDir = vec3f(0.0, 1.0, 0.0);
  for (var i = 0u; i < 8u; i = i + 1u) {
    let dx = select(1.0, -1.0, (i & 1u) != 0u);
    let dy = select(1.0, -1.0, (i & 2u) != 0u);
    let dz = select(1.0, -1.0, (i & 4u) != 0u);
    let d = normalize(vec3f(dx, dy, dz));
    let h = traverseClosest(center, d, reach);
    if (h.tri == 0xffffffffu) {
      farthest = reach * 10.0;
      farDir = d;
    } else {
      // ANY backface hit means the ray left a solid, i.e. this probe is inside geometry. This
      // used to also require h.t < spacing * 0.5, which is 0.5 m at the near cascade: a probe
      // half a metre inside the ground slab (its surface is exactly at the limit) scored 0
      // enclosed and stayed VALID with an all-backface sweep and E = 0 - a zero-radiance vote
      // in every cage around it, and 64 of cascade 0's 256 probes sat there. Front faces are
      // not counted, so a probe merely close to a wall is unaffected.
      if (dot(triNormal(h.tri), d) > 0.0) { enclosed = enclosed + 1u; }
      if (h.t > farthest) { farthest = h.t; farDir = d; }
    }
  }

  var pos = center;
  var valid = 1u;
  if (enclosed >= 5u) {
    // Virtual offset (Unity) / relocation (DDGI): pull the probe out along the most open
    // sampling direction, then re-test once.
    //
    // The offset is BOUNDED by one cell. farthest is reach*10 for any missing ray, so
    // the unbounded form moved probes tens of metres (measured: 190 relocated probes,
    // max 24 m in the simple scene), out of their own cell and frequently out of the
    // enclosure they started inside - which then interpolates that region's lighting back
    // into the space it was supposed to be excluded from.
    let offset = min(max(farthest, spacing * 0.5), spacing) * 0.6;
    pos = center + farDir * offset;
    var stillEnclosed = 0u;
    for (var i = 0u; i < 8u; i = i + 1u) {
      let dx = select(1.0, -1.0, (i & 1u) != 0u);
      let dy = select(1.0, -1.0, (i & 2u) != 0u);
      let dz = select(1.0, -1.0, (i & 4u) != 0u);
      let d = normalize(vec3f(dx, dy, dz));
      let h = traverseClosest(pos, d, reach);
      if (h.tri != 0xffffffffu && dot(triNormal(h.tri), d) > 0.0) {
        stillEnclosed = stillEnclosed + 1u;
      }
    }
    // Fail closed: a probe that cannot escape geometry deactivates rather than leaking.
    if (stillEnclosed >= 5u) { valid = 0u; }
  }

  probePos[probe] = vec4f(pos, f32(valid));
  probeFlags[probe] = valid | keepFresh;
}

@compute @workgroup_size(64)
fn probeTrace(@builtin(global_invocation_id) gid: vec3u) {
  let ray = gid.x;
  let raysPerProbe = u32(P.misc.x);
  if (ray >= P.counts.w * raysPerProbe) { return; }
  let probe = ray / raysPerProbe;
  let r = ray % raysPerProbe;

  let pr = probePos[probe];
  if (pr.w < 0.5) {
    rayData[ray] = vec4f(0.0, 0.0, 0.0, RAY_MISS);
    return;
  }
  let d = rayDirection(r, P.misc.x);
  // Ray-origin bias is a WORLD offset, not a fraction of the cascade spacing. Scaling it
  // by spacing gives the outer cascades a 2 m offset, which teleports ray origins through
  // 0.25 m walls (straight through the scene, and out of a sealed room) and reports open
  // sky from inside a closed space. 0.02 m is enough to leave the surface the ray starts
  // next to and is harmless at every cascade scale.
  let o = pr.xyz + d * 0.02;

  let h = traverseClosest(o, d, RAY_MISS);
  if (h.tri == 0xffffffffu) {
    // Miss: sky radiance, and a capped distance so distant/empty directions do not
    // dominate the distance moments (a raw 1e4 made Chebyshev reject everything).
    rayData[ray] = vec4f(skyRadiance(d), RAY_CAP);
    return;
  }
  let pos = o + d * h.t;
  var n = triNormal(h.tri);
  // Backface rule (DDGI/RTXGI): a ray that hits a backface records zero irradiance and
  // a depth shortened by 80%, i.e. the probe treats it as "no hit". Shading it instead
  // - the two-sided convention the render pass uses, which is correct there - lets a
  // probe collect the lighting of the surface it is behind. That is the documented
  // light-leak / dark-band failure mode, and this scene is full of thin walls.
  if (dot(n, d) > 0.0) {
    rayData[ray] = vec4f(0.0, 0.0, 0.0, h.t * 0.2);
    return;
  }
  rayData[ray] = vec4f(shadePoint(pos, n, h.tri), min(h.t, RAY_CAP));
}
`;

const BLEND_COMMON = /* wgsl */ `
/** Usable seed source for a reseeded probe, or 0xffffffff.
 *
 *  A freshly (re)seeded probe starts from the current frame with no history at bootstrap,
 *  where there is nothing to inherit. After a scroll that is wrong: the band is one frame
 *  old while its neighbours average ~14 frames under hysteresis, so with the light moving
 *  the band boundary is a visible step. The inward neighbour, one cell back along the
 *  scrolled axis, holds a converged atlas from one cell away, so the fresh probe inherits
 *  it and uses normal hysteresis.
 *
 *  A DEACTIVATED probe's tile is never written, so inheriting from one reads undefined
 *  texture contents - it produced NaN and wholly black frames before this check. */
fn seedSource(probe: u32) -> u32 {
  if ((probeFlags[probe] & 2u) == 0u) { return 0xffffffffu; }
  let seed = seedFrom[probe];
  if (seed == 0xffffffffu || seed >= P.counts.w || probePos[seed].w < 0.5) { return 0xffffffffu; }
  return seed;
}

fn probeHysteresis(probe: u32) -> f32 {
  // Temporal accumulation is the DEFAULT (P.misc.y = 0.93, ~14 frames). The ONLY probe that
  // starts from the current frame is one reseeded this frame with nothing to inherit - it has
  // no usable history. Testing the seed source alone (which is what this did) returned 0 for
  // every ordinary probe: the atlas then held a single frame's 64-ray estimate, so each
  // wrapped band snapped to its own value one frame after inheriting its neighbour instead of
  // converging, and every scroll read as an abrupt relight while flying.
  //
  // blendRadiance clears the fresh bit last, and blendDistance is dispatched before it, so
  // both passes see the same value. Do not reorder the two dispatches.
  if ((probeFlags[probe] & 2u) != 0u && seedSource(probe) == 0xffffffffu) { return 0.0; }
  return P.misc.y;
}

/** Tile coordinate to read the previous atlas from: the seed source for a reseeded
 *  probe, its own tile otherwise. */
fn previousTile(probe: u32, px: u32, py: u32, tileSize: u32, slotsPerRow: u32) -> vec2i {
  let seed = seedSource(probe);
  if (seed == 0xffffffffu) { return atlasTexel(probe, px, py, tileSize, slotsPerRow); }
  return atlasTexel(seed, px, py, tileSize, slotsPerRow);
}
`;

export const GI_BLEND_RADIANCE_WGSL = PRELUDE + BLEND_COMMON + /* wgsl */ `
@group(1) @binding(0) var texIn: texture_2d<f32>;
@group(1) @binding(1) var texOut: texture_storage_2d<rgba16float, write>;

// 1 KB per workgroup, NOT per invocation: caching 64 rays in function-local
// memory would reserve 64 KB of workgroup storage and hang the 680M.
var<workgroup> shRays: array<vec4f, 64>;

@compute @workgroup_size(64)
fn blendRadiance(@builtin(workgroup_id) wid: vec3u, @builtin(local_invocation_id) lid: vec3u) {
  let probe = wid.x;
  let lane = lid.x;
  if (probe >= P.counts.w) { return; }
  let raysPerProbe = u32(P.misc.x);
  if (probePos[probe].w >= 0.5 && lane < raysPerProbe) {
    shRays[lane] = rayData[probe * raysPerProbe + lane];
  }
  workgroupBarrier();
  if (probePos[probe].w < 0.5) { return; }

  let interior = P.tile.x;
  let tileSize = P.tile.y;
  let h = probeHysteresis(probe);
  let texels = tileSize * tileSize;
  for (var t = lane; t < texels; t = t + 64u) {
    let px = t % tileSize;
    let py = t / tileSize;
    let dir = texelDirection(px, py, interior);
    var sum = vec3f(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < raysPerProbe; i = i + 1u) {
      let w = max(dot(dir, rayDirection(i, P.misc.x)), 0.0);
      let ray = shRays[i];
      // Backface rays carry zero radiance and a shortened distance (see probeTrace),
      // which is what keeps a probe from lighting itself off the surface it is behind.
      sum = sum + ray.rgb * w;
      wsum = wsum + w;
    }
    // The atlas stores IRRADIANCE, not mean radiance: E = PI * (sum of cosine-weighted
    // radiance). Storing the normalised mean instead leaves "is this E or E/PI"
    // ambiguous at every call site, which is how the render and the tracer drifted
    // apart by a factor of PI here. The surface then reflects albedo * E / PI.
    var e = vec3f(0.0);
    if (wsum > 1e-5) { e = sum * (PI / wsum); }
    e = min(e, vec3f(35.0));
    // No perception encoding: the atlas is rgba16float, whose ~3 significant digits at
    // these magnitudes make gamma-5 pointless, and dropping it removes a pow() round
    // trip on both the write and every fetch.
    // Read from the seed source when this probe was reseeded, but always WRITE to this
    // probe's own tile: the seed is only the starting point of the temporal blend.
    let old = textureLoad(texIn, previousTile(probe, px, py, tileSize, P.atlas.z), 0).rgb;
    textureStore(texOut, atlasTexel(probe, px, py, tileSize, P.atlas.z), vec4f(mix(e, old, h), 1.0));
  }
  // Clear the seeded flag once all lanes are done writing this probe's tile.
  workgroupBarrier();
  if (lane == 0u) { probeFlags[probe] = probeFlags[probe] & ~2u; }
}
`;

export const GI_BLEND_DISTANCE_WGSL = PRELUDE + BLEND_COMMON + /* wgsl */ `
@group(1) @binding(0) var texIn: texture_2d<f32>;
// rgba16float, not rg16float: rg16float storage needs texture-formats-tier1
// (absent on the 680M). See docs/research/probe-gi-placement-leaks-packing.md §C.5.
@group(1) @binding(1) var texOut: texture_storage_2d<rgba16float, write>;

var<workgroup> shRays: array<vec4f, 64>;

@compute @workgroup_size(64)
fn blendDistance(@builtin(workgroup_id) wid: vec3u, @builtin(local_invocation_id) lid: vec3u) {
  let probe = wid.x;
  let lane = lid.x;
  if (probe >= P.counts.w) { return; }
  let raysPerProbe = u32(P.misc.x);
  if (probePos[probe].w >= 0.5 && lane < raysPerProbe) {
    shRays[lane] = rayData[probe * raysPerProbe + lane];
  }
  workgroupBarrier();
  if (probePos[probe].w < 0.5) { return; }

  let interior = P.tile.z;
  let tileSize = P.tile.w;
  let h = probeHysteresis(probe);
  let texels = tileSize * tileSize;
  for (var t = lane; t < texels; t = t + 64u) {
    let px = t % tileSize;
    let py = t / tileSize;
    let dir = texelDirection(px, py, interior);
    var d1 = 0.0;
    var d2 = 0.0;
    var wsum = 0.0;
    for (var i = 0u; i < raysPerProbe; i = i + 1u) {
      let w = max(dot(dir, rayDirection(i, P.misc.x)), 0.0);
      let dist = min(shRays[i].a, 30.0);
      // Distance moments use cos^16 rather than the cosine the irradiance convolution
      // uses: the DDGI prescription, and it keeps the visibility estimate local instead
      // of averaging distances across the whole hemisphere. Written as repeated squaring
      // because pow() here is 4.9M calls per frame (768 probes x 100 texels x 64 rays)
      // and measured as the single most expensive line in the update.
      let w2 = w * w;
      let w4 = w2 * w2;
      let w8 = w4 * w4;
      let mw = w8 * w8;
      d1 = d1 + mw * dist;
      d2 = d2 + mw * dist * dist;
      wsum = wsum + mw;
    }
    var mean = 0.0;
    var mean2 = 0.0;
    if (wsum > 1e-5) { mean = d1 / wsum; mean2 = d2 / wsum; }
    let old = textureLoad(texIn, previousTile(probe, px, py, tileSize, P.atlas.w), 0).rg;
    textureStore(texOut, atlasTexel(probe, px, py, tileSize, P.atlas.w),
      vec4f(mix(vec2f(mean, mean2), old, h), 0.0, 1.0));
  }
}
`;
