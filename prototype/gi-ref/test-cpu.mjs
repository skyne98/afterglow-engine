// Hand-checkable checks for the CPU reference. Every expected number below is
// derived by hand from the scene in scene.js; nothing is copied from the code under
// test. Run: node prototype/gi-ref/test-cpu.mjs

import {
  LIGHT, SKY, buildScene, traceClosest, shadeSurfaceDirect, shadeHit, triNormal, cross, normalize, LAST_BARY,
} from './scene.js';
import {
  PI, FIELD, ATLAS, RAY_DIRECTIONS, probePosition, probePositions, octDecode, octEncode,
  texelDirection, blendProbe, blendDistance, sampleTileBilinear, sampleCascade, traceProbeRays,
  tonemapACES, sRGBEncode, shadeDirect, shadeFragment, runPipeline, GAMMA_INV, GAMMA,
} from './gi-cpu.js';

const scene = buildScene();
let failures = 0;
const near = (a, b, eps = 1e-5) => Math.abs(a - b) <= eps;
const check = (name, ok, detail = '') => {
  if (!ok) failures++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? '  ' + detail : ''}`);
};
const fmt = v => Array.isArray(v) ? '[' + v.map(n => typeof n === 'number' ? +n.toFixed(6) : n).join(', ') + ']' : String(v);

console.log(`scene: ${scene.triCount} triangles  |  field: ${FIELD.counts.join('x')} probes, `
  + `${FIELD.raysPerProbe} rays each  |  atlas: ${ATLAS.tile}x${ATLAS.tile} (interior ${ATLAS.interior})`);

// ---------------------------------------------------------------- 1. geometry

{
  const expect = [
    [0, 0.5, -1], [1, 0.5, -1], [0, 1.5, -1], [1, 1.5, -1],
    [0, 0.5, 0], [1, 0.5, 0], [0, 1.5, 0], [1, 1.5, 0],
  ];
  const got = probePositions();
  check('probe grid positions are exact', got.every((p, i) => p.every((v, a) => v === expect[i][a])),
    got.map(fmt).join(' '));
}

// Straight-down rays: t is the probe height, except under the box where the box top
// (y = 0.4) is hit first. bias = 0 so distances are the pure geometry.
{
  const cases = [
    ['probe 0 -> ground', [0, 0.5, -1], [0, -1, 0], 0.5],
    ['probe 1 -> box top', [1, 0.5, -1], [0, -1, 0], 0.1],
    ['probe 3 -> box top', [1, 1.5, -1], [0, -1, 0], 1.1],
    ['probe 4 -> ground', [0, 0.5, 0], [0, -1, 0], 0.5],
  ];
  for (const [name, o, d, t] of cases) {
    const hit = traceClosest(scene, o, d);
    check(name, hit.hit && near(hit.t, t), `t=${hit.hit ? hit.t : 'miss'} expected ${t}`);
  }
  // A side face, hit along the axis: from (1, 0.2, -2) heading +Z hits the -Z face at
  // z = -1.2 => t = 0.8, normal (0, 0, -1), albedo = wall colour.
  // A side face, hit along the axis: from (1, 0.2, -2) heading +Z hits the -Z face at
  // z = -1.3 => t = 0.7, normal (0, 0, -1), albedo = wall colour.
  const side = traceClosest(scene, [1, 0.2, -2], [0, 0, 1]);
  check('box -Z face hit', side.hit && near(side.t, 0.7) && near(side.normal[2], -1),
    `t=${side.hit ? side.t : 'miss'} n=${side.hit ? fmt(side.normal) : '-'}`);
  // Invariant rather than trusting hand-wound quads: every box face normal points away
  // from the box centre, and the ground's points up.
  {
    const centre = [1, 0.2, -1.1];
    let bad = 0;
    for (let i = 0; i < scene.triCount; i++) {
      const n = triNormal(scene.tris, i);
      const o = i * 9;
      const face = [
        (scene.tris[o] + scene.tris[o + 3] + scene.tris[o + 6]) / 3 - centre[0],
        (scene.tris[o + 1] + scene.tris[o + 4] + scene.tris[o + 7]) / 3 - centre[1],
        (scene.tris[o + 2] + scene.tris[o + 5] + scene.tris[o + 8]) / 3 - centre[2],
      ];
      const nearBox = Math.hypot(...face) < 1.0;   // ground triangles are far from it
      const dot = n[0] * face[0] + n[1] * face[1] + n[2] * face[2];
      if (nearBox ? dot <= 0 : n[1] <= 0) bad++;
    }
    check('box faces are wound outward and the ground faces up', bad === 0, `${bad} bad triangle(s)`);
  }
  check('vertical face albedo is the wall colour',
    side.hit && side.albedo.every((v, i) => near(v, [0.40, 0.38, 0.36][i])), fmt(side.albedo));
  // A ray that escapes: misses every triangle.
  const miss = traceClosest(scene, [0, 0.5, -1], [0, 1, 0]);
  check('upward ray misses', !miss.hit);
}

// ---------------------------------------------------------------- 2. shading

{
  // Ground point (0, 0, -1): outside the box footprint, so nothing blocks it.
  //   light dir = (1.5, 2.5, 2.5)/d, d = sqrt(14.75), ndotl = 2.5/d,
  //   irradiance = 8/(1+0.09*d^2) * ndotl, radiance = albedo * irradiance / PI.
  const d = Math.hypot(1.5, 2.5, 2.5);
  const ndotl = 2.5 / d;
  const e = LIGHT.intensity / (1 + LIGHT.falloffK * d * d) * ndotl;
  const expect = [0.30, 0.29, 0.28].map(a => a * e / PI);
  const got = shadeSurfaceDirect(scene, [0, 0, -1], [0, 1, 0], [0.30, 0.29, 0.28]);
  check('direct radiance at a known ground point', got.every((v, i) => near(v, expect[i], 1e-6)),
    `${fmt(got)} expected ${fmt(expect)}`);

  // The ground directly under the box is in its shadow, at the same distance to the
  // light, so the direct term must be exactly zero there.
  const shadowed = shadeSurfaceDirect(scene, [1, 0, -1], [0, 1, 0], [0.30, 0.29, 0.28]);
  check('ground under the box is shadowed', shadowed.every(v => v === 0), fmt(shadowed));

  // Same point, but shaded as a probe hit reached from above => front face, shaded.
  const hit = traceClosest(scene, [1, 0.5, -1], [0, -1, 0]);
  const lit = shadeHit(scene, hit, [0, -1, 0]);
  check('front-face probe hit is shaded', lit.some(v => v > 0), fmt(lit));

  // The same triangle reached from below is a backface: the DDGI rule records zero.
  const fromBelow = noteBackface(scene);
  check('backface probe hit records zero radiance', fromBelow.every(v => v === 0), fmt(fromBelow));
}

/** Drive shadeHit with a ray that reaches the ground's top face from underneath. */
function noteBackface(scene) {
  const hit = traceClosest(scene, [0, -0.5, 0], [0, 1, 0]);
  if (!hit.hit) throw new Error('backface probe: no hit');
  return shadeHit(scene, hit, [0, 1, 0]);
}

// ---------------------------------------------------------------- 3. octahedral + gamma

{
  const cards = [[0, 0, 1], [1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, -1]];
  const expectedUV = [[0.5, 0.5], [1, 0.5], [0, 0.5], [0.5, 1], [0.5, 0], [1, 1]];
  let ok = true, detail = '';
  cards.forEach((n, i) => {
    const uv = octEncode(n), back = octDecode(uv[0], uv[1]);
    if (!uv.every((v, a) => near(v, expectedUV[i][a])) || !back.every((v, a) => near(v, n[a]))) {
      ok = false; detail += ` ${n}->${fmt(uv)}->${fmt(back)}`;
    }
  });
  check('octahedral encode/decode of the six axis directions', ok, detail);

  const r = 0.489;
  check('gamma-5 encode/decode round trip', near(Math.pow(Math.pow(r, GAMMA_INV), GAMMA), r, 1e-12),
    `pow(pow(${r},0.2),5)=${Math.pow(Math.pow(r, GAMMA_INV), GAMMA)}`);

  // No interior texel is exactly axis-aligned for interior = 4, so the +Y direction
  // must come out of a blend, not a single ray.
  const dir = texelDirection(3, 3);
  check('texel directions are unit vectors', near(Math.hypot(...dir), 1, 1e-6), fmt(dir));
}

// ---------------------------------------------------------------- 4. probe trace + atlas

{
  const rays = RAY_DIRECTIONS.length;
  const positions = probePositions();
  const valid = positions.map(() => true);

  // Frame 1 (fresh, hysteresis 0): every texel holds pow(mean radiance, 1/5).
  const radiance = [];
  const distance = [];
  for (let i = 0; i < positions.length; i++) {
    const probeRays = probeRaysNoBias(i);
    radiance.push(blendProbe(probeRays, null, 0));
    distance.push(blendDistance(probeRays, null, 0));
  }

  // Ray composition: probes 1 and 3 hit the box top with their -Y ray and miss with the
  // other five; every other probe hits the ground with -Y and misses the rest.
  let ok = true, detail = '';
  positions.forEach((p, i) => {
    const probeRays = probeRaysNoBias(i);
    const hits = probeRays.filter(r => r.hit);
    const expectT = (i === 1) ? 0.1 : (i === 3) ? 1.1 : p[1];
    if (hits.length !== 1 || !near(hits[0].t, expectT)) {
      ok = false; detail += ` probe${i}:${hits.length} hits t=${hits[0] ? hits[0].t : '-'} want ${expectT}`;
    }
    probeRays.filter(r => !r.hit).forEach(r => {
      if (!r.radiance.every((v, c) => near(v, SKY[c]))) { ok = false; detail += ` probe${i} sky mismatch`; }
    });
  });
  check('each probe: exactly one hit (-Y) at the exact expected t, 5 sky rays', ok, detail);

  // A fully open upward texel: its direction can only see sky, so the decoded value is
  // the sky radiance exactly.
  const skyOnly = blendProbe(RAY_DIRECTIONS.map((d, ray) => ({
    ray, direction: d, hit: false, t: Infinity, radiance: [...SKY],
  })), null, 0);
  const decodedSky = skyOnly[5][3].map(c => Math.pow(c, GAMMA));
  check('all-sky atlas decodes back to the sky radiance', decodedSky.every((v, c) => near(v, SKY[c], 1e-4)),
    fmt(decodedSky));

  // Hysteresis: one step at 0.93 keeps 93% of the old value.
  const step = blendProbe(RAY_DIRECTIONS.map((d, ray) => ({
    ray, direction: d, hit: false, t: Infinity, radiance: [1, 1, 1],
  })), skyOnly, 0.93);
  const mixed = Math.pow(step[5][3][0], GAMMA);
  const expectMixed = Math.pow(Math.pow(1, GAMMA_INV) * 0.07
    + Math.pow(SKY[0], GAMMA_INV) * 0.93, GAMMA);
  check('hysteresis blend keeps the old value at the right weight',
    near(mixed, expectMixed, 1e-6), `${mixed} expected ${expectMixed}`);

  // Independent cosine-weighted average for one texel, recomputed by hand from the ray
  // list rather than through blendProbe's loop.
  {
    const probeRays = probeRaysNoBias(0);
    const dir = texelDirection(2, 4);
    let sum = [0, 0, 0], wsum = 0;
    for (const r of probeRays) {
      const w = Math.max(dir[0] * r.direction[0] + dir[1] * r.direction[1] + dir[2] * r.direction[2], 0);
      sum = sum.map((s, c) => s + r.radiance[c] * w); wsum += w;
    }
    const expected = sum.map(s => Math.pow(Math.max(s / wsum, 1e-6), GAMMA_INV));
    const got = radiance[0][4][2];
    check('atlas texel equals the independent cosine-weighted mean',
      got.every((v, c) => near(v, expected[c], 1e-9)), `${fmt(got)} expected ${fmt(expected)}`);
  }

  // Ring of probes around a point must interpolate: query exactly at probe 0 with the
  // +Y normal, where the trilinear weight concentrates on probe 0.
  {
    const p = probePositions()[0];
    const query = [p[0] + 0.5 * FIELD.spacing, p[1] + 0.5 * FIELD.spacing, p[2] + 0.5 * FIELD.spacing];
    const res = sampleCascade(radiance, distance, valid, query, [0, 1, 0]);
    check('cage returns a non-zero irradiance for an up-facing point', res.w > 0 && res.e.some(v => v > 0),
      `w=${res.w.toFixed(4)} e=${fmt(res.e)}`);
    const outside = sampleCascade(radiance, distance, valid, [50, 0.5, 50], [0, 1, 0]);
    check('a point outside the field returns weight 0', outside.w === 0, `w=${outside.w}`);
  }

  // Fragments: an up-facing ground point, and the same point inside the box's shadow.
  {
    const lit = shadeFragment(scene, radiance, distance, valid, [0, 0, -1], [0, 1, 0], [0.30, 0.29, 0.28]);
    console.log(`      ground(0,0,-1) lit:    color=${fmt(lit.color)} srgb=${fmt(lit.srgb.map(v => v * 255))}`);
    const under = shadeFragment(scene, radiance, distance, valid, [1, 0, -1], [0, 1, 0], [0.30, 0.29, 0.28]);
    console.log(`      ground under box:      color=${fmt(under.color)} srgb=${fmt(under.srgb.map(v => v * 255))}`);
    check('a lit ground point is brighter than an occluded one', lit.color[0] > under.color[0],
      `${lit.color[0].toFixed(4)} > ${under.color[0].toFixed(4)}`);
  }
}

/** Probe rays for one probe with bias 0, which is what the hand checks assume. */
function probeRaysNoBias(index) {
  const rays = [];
  const p = probePosition(index);
  for (let i = 0; i < RAY_DIRECTIONS.length; i++) {
    const d = RAY_DIRECTIONS[i];
    const hit = traceClosest(scene, p, d);
    rays.push(hit.hit
      ? { ray: i, direction: d, hit: true, t: hit.t, hitDistance: hit.t, radiance: shadeHit(scene, hit, d) }
      : { ray: i, direction: d, hit: false, t: Infinity, radiance: [...SKY] });
  }
  return rays;
}

// ---------------------------------------------------------------- 6. tie safety

// A ray that passes exactly through a shared triangle edge is a tie: an f64 CPU and an
// f32 GPU may resolve it differently. The reference scene is built so no probe ray does
// this, and the harness checks it rather than trusting the geometry to stay that way.
{
  let worst = 1, where = '';
  for (let i = 0; i < probePositions().length; i++) {
    for (const bias of [0, 0.225]) {
      for (const r of traceProbeRays(scene, i, bias)) {
        if (!r.hit) continue;
        const edge = Math.min(...LAST_BARY);
        if (edge < worst) { worst = edge; where = `probe ${i} t=${r.t.toFixed(3)}`; }
      }
    }
  }
  check('no probe ray hits a shared triangle edge (barycentric tie)', worst > 1e-3,
    `closest barycentric margin ${worst.toExponential(3)} (${where})`);
}

console.log(failures === 0 ? '\nall CPU reference checks passed' : `\n${failures} check(s) FAILED`);
process.exit(failures === 0 ? 0 : 1);
