// Tiny scene for the probe-GI reference.
//
// Everything here is chosen so the expected values are exact decimals that can be
// checked by hand (no BVH, no approximation, no noise):
//   * ground quad at y = 0, x/z in [-3, 3]      (2 triangles)
//   * one 1x1x1 cube, x/z in [-0.5, 0.5], y in [0, 1]  (12 triangles)
//   * a point light at (1.5, 2.5, 1.5) with white colour
//   * a CONSTANT sky radiance (a gradient would make every probe value untestable)
//
// Tracing is brute force over the triangle list on purpose: a reference must not
// share code, an acceleration structure, or a bug with the thing it verifies.

export const LIGHT = {
  position: [1.5, 2.5, 1.5],
  // White, so a hit radiance is albedo * (attenuation / PI) and nothing else.
  color: [1, 1, 1],
  // Engine point-light model: irradiance = I / (1 + 0.09 d^2). Arbitrary but the
  // GPU side must use the same constant.
  intensity: 8,
  falloffK: 0.09,
};

/** Constant sky radiance. A probe ray that escapes the scene records exactly this. */
export const SKY = [0.20, 0.21, 0.23];

export const SKY_INTENSITY = 1;

export function skyRadiance(_dir) {
  return [SKY[0] * SKY_INTENSITY, SKY[1] * SKY_INTENSITY, SKY[2] * SKY_INTENSITY];
}

/** Orientation-driven albedo, identical to triAlbedo() in the WGSL. */
export function triAlbedo(normal) {
  const horizontal = Math.abs(normal[1]) >= 0.7;
  return horizontal ? [0.30, 0.29, 0.28] : [0.40, 0.38, 0.36];
}

function tri(a, b, c, out) {
  out.push(a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]);
}

/** Two triangles for an axis-aligned, outward-facing quad. */
function quad(p0, p1, p2, p3, out) {
  tri(p0, p1, p2, out);
  tri(p0, p2, p3, out);
}

export function buildScene() {
  const t = [];
  // Ground: normal +Y, counter-clockwise seen from above. Centred at (0, +0.2) rather
  // than the origin so neither diagonal passes through a probe grid line: a ray that
  // hits a shared triangle edge exactly is a tie, and an f64 CPU and an f32 GPU are
  // allowed to break a tie differently. test-cpu.mjs asserts this property directly.
  quad([-3, 0, 3.2], [3, 0, 3.2], [3, 0, -2.8], [-3, 0, -2.8], t);

  // One box, no bottom face, centred under the x = 1, z = -1 grid line but offset in z
  // so the top quad's diagonal misses that line. Probe 1 is 0.1 m above its top and
  // probe 3 is 1.1 m above it, so a single -Y ray exercises "nearest hit is the box, not
  // the ground" with exact values.
  box(t, 1, 0.2, -1.1, 0.2, 0.2, 0.2);   // x in [0.8,1.2], y in [0,0.4], z in [-1.3,-0.9]

  const tris = Float32Array.from(t);
  return { tris, triCount: tris.length / 9 };
}

/** Five faces (no bottom), outward normals. Unit coordinates are [-1,1] on every axis. */
function box(out, cx, cy, cz, ex, ey, ez) {
  const v = (x, y, z) => [cx + x * ex, cy + y * ey, cz + z * ez];
  quad(v(-1, 1, 1), v(1, 1, 1), v(1, 1, -1), v(-1, 1, -1), out);     // top  (+Y)
  quad(v(1, -1, -1), v(-1, -1, -1), v(-1, 1, -1), v(1, 1, -1), out); // -Z
  quad(v(-1, -1, 1), v(1, -1, 1), v(1, 1, 1), v(-1, 1, 1), out);     // +Z
  quad(v(-1, -1, -1), v(-1, -1, 1), v(-1, 1, 1), v(-1, 1, -1), out); // -X
  quad(v(1, -1, 1), v(1, -1, -1), v(1, 1, -1), v(1, 1, 1), out);     // +X
}

export function triNormal(tris, index) {
  const o = index * 9;
  const ax = tris[o], ay = tris[o + 1], az = tris[o + 2];
  const ux = tris[o + 3] - ax, uy = tris[o + 4] - ay, uz = tris[o + 5] - az;
  const vx = tris[o + 6] - ax, vy = tris[o + 7] - ay, vz = tris[o + 8] - az;
  const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
  const len = Math.hypot(nx, ny, nz) || 1;
  return [nx / len, ny / len, nz / len];
}

/** Moller-Trumbore. Returns t > 0 or -1. */
export function hitTriangle(tris, index, o, d, tMax) {
  const p = index * 9;
  const ax = tris[p], ay = tris[p + 1], az = tris[p + 2];
  const bax = tris[p + 3], bay = tris[p + 4], baz = tris[p + 5];
  const cax = tris[p + 6], cay = tris[p + 7], caz = tris[p + 8];
  const e1 = [bax - ax, bay - ay, baz - az];
  const e2 = [cax - ax, cay - ay, caz - az];
  const px = d[1] * e2[2] - d[2] * e2[1];
  const py = d[2] * e2[0] - d[0] * e2[2];
  const pz = d[0] * e2[1] - d[1] * e2[0];
  const det = e1[0] * px + e1[1] * py + e1[2] * pz;
  if (Math.abs(det) < 1e-12) return -1;
  const inv = 1 / det;
  const tx = o[0] - ax, ty = o[1] - ay, tz = o[2] - az;
  const u = (tx * px + ty * py + tz * pz) * inv;
  if (u < 0 || u > 1) return -1;
  const qx = ty * e1[2] - tz * e1[1];
  const qy = tz * e1[0] - tx * e1[2];
  const qz = tx * e1[1] - ty * e1[0];
  const v = (d[0] * qx + d[1] * qy + d[2] * qz) * inv;
  if (v < 0 || u + v > 1) return -1;
  const t = (e2[0] * qx + e2[1] * qy + e2[2] * qz) * inv;
  if (t <= 1e-3 || t > tMax) return -1;
  LAST_BARY = [u, v, 1 - u - v];
  return t;
}

/** Barycentric weights of the most recent hitTriangle() hit, for tie detection. */
export let LAST_BARY = [0, 0, 0];

/** Closest hit over every triangle: { t, tri, point, normal, albedo } or a miss. */
export function traceClosest(scene, origin, dir, tMax = Infinity) {
  let best = -1, bestTri = -1;
  for (let i = 0; i < scene.triCount; i++) {
    const t = hitTriangle(scene.tris, i, origin, dir, tMax);
    if (t > 0 && (best < 0 || t < best)) { best = t; bestTri = i; }
  }
  if (best < 0) return { hit: false, t: tMax, tri: -1, point: null, normal: null, albedo: null };
  const normal = triNormal(scene.tris, bestTri);
  return {
    hit: true,
    t: best,
    tri: bestTri,
    point: [origin[0] + dir[0] * best, origin[1] + dir[1] * best, origin[2] + dir[2] * best],
    normal,
    albedo: triAlbedo(normal),
  };
}

/** Any-hit shadow test toward the light. */
export function shadowed(scene, origin, dir, tMax) {
  for (let i = 0; i < scene.triCount; i++) {
    if (hitTriangle(scene.tris, i, origin, dir, tMax) > 0) return true;
  }
  return false;
}

/**
 * Radiance leaving a lit surface: albedo/PI * E, with E the point light's
 * irradiance. Zero on a backface, per the DDGI backface rule (a probe ray that
 * hits geometry from behind records nothing rather than the surface's lighting).
 */
export function shadeHit(scene, hit, rayDir) {
  const n = hit.normal;
  if (n[0] * rayDir[0] + n[1] * rayDir[1] + n[2] * rayDir[2] > 0) return [0, 0, 0];
  return shadeSurfaceDirect(scene, hit.point, n, hit.albedo);
}

export function shadeSurfaceDirect(scene, point, n, albedo) {
  const toLight = [
    LIGHT.position[0] - point[0], LIGHT.position[1] - point[1], LIGHT.position[2] - point[2],
  ];
  const dist = Math.hypot(...toLight);
  const l = [toLight[0] / dist, toLight[1] / dist, toLight[2] / dist];
  const ndotl = n[0] * l[0] + n[1] * l[1] + n[2] * l[2];
  if (ndotl <= 0) return [0, 0, 0];
  const o = [point[0] + n[0] * 0.02, point[1] + n[1] * 0.02, point[2] + n[2] * 0.02];
  if (shadowed(scene, o, l, dist - 0.05)) return [0, 0, 0];
  const e = LIGHT.intensity / (1 + LIGHT.falloffK * dist * dist) * ndotl;
  return albedo.map(a => a * e * LIGHT.color[0] / Math.PI);
}

/** Unoccluded sky/hit fraction of a cosine hemisphere, by hit distance (RTAO). */
export function aoFactor(scene, point, n, rays, maxDistance) {
  const up = Math.abs(n[1]) > 0.99 ? [1, 0, 0] : [0, 1, 0];
  const tangent = normalize(cross(n, up));
  const bitangent = cross(n, tangent);
  let occluded = 0;
  for (let i = 0; i < rays.length; i++) {
    const [sinTheta, cosTheta, phi] = rays[i];
    const d = normalize([
      tangent[0] * Math.cos(phi) * sinTheta + bitangent[0] * Math.sin(phi) * sinTheta + n[0] * cosTheta,
      tangent[1] * Math.cos(phi) * sinTheta + bitangent[1] * Math.sin(phi) * sinTheta + n[1] * cosTheta,
      tangent[2] * Math.cos(phi) * sinTheta + bitangent[2] * Math.sin(phi) * sinTheta + n[2] * cosTheta,
    ]);
    const hit = traceClosest(scene, [point[0] + n[0] * 0.02, point[1] + n[1] * 0.02, point[2] + n[2] * 0.02], d, maxDistance);
    if (hit.hit) occluded += 1 - Math.min(hit.t / maxDistance, 1);
  }
  return 1 - occluded / rays.length;
}

export function normalize(v) {
  const len = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / len, v[1] / len, v[2] / len];
}

export function cross(a, b) {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}
