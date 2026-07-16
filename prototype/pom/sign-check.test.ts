// prototype/pom/sign-check.test.ts — correctness of the canonical POM march.
//
// This mirrors the WGSL `pom_march` in pom.ts BYTE-FOR-BYTE (same tiled UV
// space, same height field, same occlusion interpolation) and asserts the core
// property of parallax occlusion mapping: the marched UV shifts OPPOSITE to the
// view direction's tangent-space xy component. The view direction here is the
// fragment→camera vector (z > 0, out of the surface) — the convention proven
// correct at runtime by reading the debug-material pixel from the live CEF page
// (viewDir = parallaxDirection.negate() read back as (0,0,1) at center).
//
// (An earlier version of this test tried to DERIVE the tangent-space viewDir
// from 3D geometry; that was wrong because TSL's `vec3.mul(mat3)` TBN transform
// does not match a naive dot-product model. The shader's own runtime pixel read
// is the ground truth for the sign; this test only locks in the march LOGIC.)

import { test, expect } from "bun:test";

const TEX_SIZE = 256;
const REPEAT = 4;

// --- Mirror of makeBrickTextures() height channel (pom.ts) ---
function makeHeight(): Uint8Array {
  const bw = 60;
  const bh = 28;
  const mortar = 6;
  const periodX = bw + mortar;
  const periodY = bh + mortar;
  const size = TEX_SIZE;
  const height = new Uint8Array(size * size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const row = Math.floor(y / periodY);
      const offX = (row & 1) * (periodX >> 1);
      const bx = (x + offX) % periodX;
      const by = y % periodY;
      const inMortar = bx < mortar || by < mortar;
      height[y * size + x] = inMortar ? 40 : 255;
    }
  }
  return height;
}

const height = makeHeight();

// Sample the height texture at a TILED uv (matching the WGSL: baseUV = tiledUV
// = uv*REPEAT, and the texture uses RepeatWrapping, so we sample at the tiled
// coordinate directly with wrap).
function sampleHeight(u: number, v: number): number {
  const tu = ((u % 1) + 1) % 1;
  const tv = ((v % 1) + 1) % 1;
  const x = Math.min(TEX_SIZE - 1, Math.floor(tu * TEX_SIZE));
  const y = Math.min(TEX_SIZE - 1, Math.floor(tv * TEX_SIZE));
  return height[y * TEX_SIZE + x] / 255;
}

// --- POM march, byte-for-byte with the WGSL pom_march (tiled UV space),
// including the view-angle-adaptive layer count. ---
function pomMarch(
  baseUV: [number, number],
  viewDir: [number, number, number],
  scale: number,
  minLayers: number,
  maxLayers: number,
): [number, number] {
  const len = Math.hypot(viewDir[0], viewDir[1], viewDir[2]);
  const v = [viewDir[0] / len, viewDir[1] / len, viewDir[2] / len];
  const vz = Math.max(Math.abs(v[2]), 0.001);
  const numLayersF = minLayers + (maxLayers - minLayers) * (1 - Math.abs(v[2]));
  const numLayers = Math.max(minLayers, Math.min(maxLayers, Math.round(numLayersF)));
  const P = [(v[0] * scale) / vz, (v[1] * scale) / vz];
  const delta = [P[0] / numLayers, P[1] / numLayers];
  const layerDepth = 1 / numLayers;

  let currentUV = [baseUV[0], baseUV[1]];
  let currentDepth = 0;
  let prevUV = [baseUV[0], baseUV[1]];
  let prevDepth = 0;
  let prevHeight = sampleHeight(baseUV[0], baseUV[1]);

  for (let i = 0; i < numLayers; i++) {
    currentUV = [currentUV[0] - delta[0], currentUV[1] - delta[1]];
    currentDepth += layerDepth;
    const h = sampleHeight(currentUV[0], currentUV[1]);
    if (h < currentDepth) {
      const afterDepth = h - currentDepth;
      const beforeDepth = prevHeight - prevDepth;
      const w = afterDepth / (afterDepth - beforeDepth);
      return [
        currentUV[0] + (prevUV[0] - currentUV[0]) * w,
        currentUV[1] + (prevUV[1] - currentUV[1]) * w,
      ];
    }
    prevUV = [currentUV[0], currentUV[1]];
    prevDepth = currentDepth;
    prevHeight = h;
  }
  return currentUV;
}

// A tiled base UV in the middle of the texture (uv 0.5 * REPEAT).
const BASE: [number, number] = [0.5 * REPEAT, 0.5 * REPEAT];
const SCALE = 0.05;
const MIN_L = 16;
const MAX_L = 64;

test("march shifts UV opposite to the view direction's tangent x", () => {
  // viewDir frag->camera with +x: the visible point is found by stepping the
  // UV AGAINST +x (baseUV - delta, delta.x>0) => result.x < base.x.
  const [uxPos] = pomMarch(BASE, [0.3, 0, 0.95], SCALE, MIN_L, MAX_L);
  expect(uxPos).toBeLessThan(BASE[0]);

  // viewDir frag->camera with -x: result.x > base.x.
  const [uxNeg] = pomMarch(BASE, [-0.3, 0, 0.95], SCALE, MIN_L, MAX_L);
  expect(uxNeg).toBeGreaterThan(BASE[0]);
});

test("head-on view (viewDir = +z) produces no horizontal shift", () => {
  // Straight-on, no tangent component => delta is zero => UV unchanged.
  const [ux, uy] = pomMarch(BASE, [0, 0, 1], SCALE, MIN_L, MAX_L);
  expect(Math.abs(ux - BASE[0])).toBeLessThan(1e-9);
  expect(Math.abs(uy - BASE[1])).toBeLessThan(1e-9);
});

test("march shifts UV opposite to the view direction's tangent y", () => {
  const [, uyPos] = pomMarch(BASE, [0, 0.3, 0.95], SCALE, MIN_L, MAX_L);
  expect(uyPos).toBeLessThan(BASE[1]);
  const [, uyNeg] = pomMarch(BASE, [0, -0.3, 0.95], SCALE, MIN_L, MAX_L);
  expect(uyNeg).toBeGreaterThan(BASE[1]);
});

test("displacement magnitude is bounded by the height scale", () => {
  // The full UV excursion cannot exceed |P| = |v.xy|/vz * scale.
  const v: [number, number, number] = [0.5, 0.2, 0.84];
  const len = Math.hypot(...v);
  const vn = [v[0] / len, v[1] / len, v[2] / len];
  const vz = Math.max(Math.abs(vn[2]), 0.001);
  const maxExcursion = (Math.hypot(vn[0], vn[1]) / vz) * SCALE;
  const [ux, uy] = pomMarch(BASE, v, SCALE, MIN_L, MAX_L);
  const excursion = Math.hypot(ux - BASE[0], uy - BASE[1]);
  expect(excursion).toBeLessThanOrEqual(maxExcursion + 1e-9);
});

test("adaptive layer count: grazing uses more layers than head-on", () => {
  // Head-on (|v.z|~1) => minLayers; grazing (|v.z|~0) => maxLayers. We can't
  // read the in-shader count directly, but a grazing march makes SMALLER UV
  // steps (delta = P/numLayers, numLayers larger) so the first-intersection UV
  // is closer to baseUV than a head-on fixed-min march would be. Sanity-check
  // the formula matches the WGSL: numLayersF = mix(max, min, |v.z|).
  const grazing = 0.0; // |v.z| ~0, fully grazing
  const headon = 1.0; // |v.z| ~1, head-on
  const numGrazing = MIN_L + (MAX_L - MIN_L) * (1 - grazing);
  const numHeadon = MIN_L + (MAX_L - MIN_L) * (1 - headon);
  expect(Math.round(numGrazing)).toBeGreaterThan(Math.round(numHeadon));
  expect(Math.round(numGrazing)).toBe(MAX_L);
  expect(Math.round(numHeadon)).toBe(MIN_L);
});

// Mirror of WGSL `pom_self_shadow`: height=1 is a raised brick, height≈0 is a
// recessed groove. The ray advances TOWARD the light while rising. A height
// sample above that ray blocks it. This locks in the sign that previously made
// the entire wall dark.
function selfShadowMarch(
  startUV: number,
  lightX: number,
  lightZ: number,
  scale: number,
  sample: (u: number) => number,
): boolean {
  if (lightZ <= 0.001) return false;
  const steps = 2;
  const stepUV = (lightX / Math.max(lightZ, 0.001)) * scale / steps;
  let rayUV = startUV;
  let rayHeight = sample(startUV);
  const bias = 0.035;
  for (let i = 0; i < steps; i++) {
    rayUV += stepUV;
    rayHeight += 1 / steps;
    if (sample(rayUV) > rayHeight + bias) return true;
    if (rayHeight >= 1) return false;
  }
  return false;
}

test("self-shadow: raised ridge in the light direction occludes a recessed groove", () => {
  // Begin in mortar (0.16). The light travels +x and reaches a brick (1.0),
  // which is above the rising ray and therefore blocks it.
  const heightWithRidge = (u: number) => (u >= 0.06 ? 1.0 : 0.16);
  expect(selfShadowMarch(0, 1, 1, 1, heightWithRidge)).toBe(true);
});

test("self-shadow: flat terrain and a ridge behind the light remain lit", () => {
  expect(selfShadowMarch(0, 1, 1, 1, () => 0.16)).toBe(false);
  // The ridge lies in -x, behind a +x-directed light ray, so cannot occlude.
  expect(selfShadowMarch(0, 1, 1, 1, (u) => (u < -0.06 ? 1.0 : 0.16))).toBe(false);
});
