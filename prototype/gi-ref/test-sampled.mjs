// Checks for the formulas sampled from "The Lantern Vault". Each expectation is derived
// from the documented property, not from the code under test. Run:
//   node prototype/gi-ref/test-sampled.mjs

import {
  OCT, TEXELS, octWrap, octEncode, fetch, cageWeight, chebyshevVisibility, integrateTexel,
} from './sampled.js';

let failures = 0;
const check = (name, ok, detail = '') => {
  if (!ok) failures++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? '  ' + detail : ''}`);
};
const near = (a, b, eps = 1e-6) => Math.abs(a - b) <= eps;

// --- seam mirroring: every out-of-range fetch lands on the texel the formula names
{
  // (x, y) -> mirrored texel, worked out by hand from ddOctWrap's four cases.
  const cases = [
    [[-1, 3], 0 + 8 * 4],   // x < 0  -> (-x-1, 7-y) = (0, 4)
    [[8, 3], 7 + 8 * 4],    // x > 7  -> (15-x, 7-y) = (7, 4)
    [[3, -1], 4 + 8 * 0],   // y < 0  -> (7-x, -y-1) = (4, 0)
    [[3, 8], 4 + 8 * 7],    // y > 7  -> (7-x, 15-y) = (4, 7)
    [[0, 0], 0],
    [[7, 7], 63],
  ];
  let ok = true, detail = '';
  for (const [input, expected] of cases) {
    const got = octWrap(input[0], input[1]);
    if (got !== expected) { ok = false; detail += ` (${input})->${got} want ${expected}`; }
  }
  check('octWrap mirrors every seam case to the right texel', ok, detail);
}

// --- the seam is continuous: a direction and its mirror across the seam must read the
//     same value, which is the whole point of mirroring instead of a gutter
{
  // Cache texels hold their index, so any seam discontinuity shows up as a jump.
  const cache = new Float32Array(TEXELS * 4);
  for (let i = 0; i < TEXELS; i++) cache[i * 4] = i;
  const maxJump = (dirs) => {
    let worst = 0;
    for (let i = 1; i < dirs.length; i++) {
      const a = fetch(cache, 0, dirs[i - 1]).slice(0, 3);
      const b = fetch(cache, 0, dirs[i]).slice(0, 3);
      worst = Math.max(worst, Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]));
    }
    return worst;
  };
  // Walk a great circle through the +Z pole (where the octahedral seam sits).
  const ring = [];
  for (let i = 0; i <= 720; i++) {
    const t = i / 720 * Math.PI * 2;
    // sweep a full rotation about Y, crossing both seams at z = 0
    ring.push([Math.sin(t), 0.35, Math.cos(t)]);
  }
  const jump = maxJump(ring);
  // Without mirroring, the wrap would clamp and jump by ~7 texel indices (the diagonal
  // neighbours differ by more). With mirroring, neighbouring fetches stay adjacent.
  check('fetch is seam-continuous across a full rotation of the +Z pole', jump < 6.0,
    `largest texel-index jump between neighbouring directions = ${jump.toFixed(2)}`);
}

// --- convolution normalization: a constant radiance field integrates to PI * L
{
  const rays = [
    { direction: [0, 0, 1], radiance: [2, 3, 4], distance: 5 },
    { direction: [1, 0, 0], radiance: [2, 3, 4], distance: 5 },
    { direction: [0, 0, -1], radiance: [2, 3, 4], distance: 5 },
  ];
  const axis = [0, 0, 1];
  const result = integrateTexel(rays, axis, 0.92, null);
  const expected = [2, 3, 4].map(c => c * Math.PI);
  check('constant radiance integrates to PI * L for an axis-aligned texel',
    result.raw.every((v, i) => near(v, expected[i], 1e-6)), `${result.raw} expected ${expected}`);
  check('distance moments use the pow(cos,16) filter', near(result.moments[0], 5, 1e-6),
    `mean distance ${result.moments[0]}`);
}

// --- the surface then turns irradiance back into reflected radiance: albedo * E / PI,
//     which for a uniform environment L gives albedo * L exactly
{
  const E = [2, 3, 4].map(c => c * Math.PI);
  const albedo = [0.4, 0.38, 0.36];
  const reflected = albedo.map((a, i) => a * E[i] / Math.PI);
  const expected = albedo.map((a, i) => a * [2, 3, 4][i]);
  check('albedo * E / PI recovers albedo * L under a uniform environment',
    reflected.every((v, i) => near(v, expected[i], 1e-9)), `${reflected} expected ${expected}`);
}

// --- the wrapped normal weight never reaches zero, unlike max(dot(n, dir), 0)
{
  const side = cageWeight(1, [1, 0, 0], [0, 1, 0], null);       // probe exactly at 90 deg
  const aligned = cageWeight(1, [0, 1, 0], [0, 1, 0], null);    // probe along the normal
  const behind = cageWeight(1, [0, -1, 0], [0, 1, 0], null);    // probe behind the normal
  // wrap = 0 -> weight = 0.04, which is below 0.2 so the sharpening applies: 0.04^3/0.04
  check('a probe 90 deg off the normal still contributes', near(side, 0.25 + 0.04, 1e-9),
    `weight ${side}`);
  check('a probe along the normal gets the full weight', near(aligned, 1.04, 1e-9), `weight ${aligned}`);
  check('a probe behind the normal is not zeroed outright', near(behind, 0.04 ** 3 / 0.04, 1e-12),
    `weight ${behind}`);
  // Sharpening: below 0.2 the weight becomes weight^3 / 0.04, which for 0.1 is 0.025.
  const sharpened = cageWeight(1, [0, -1, 0], [0, 1, 0], null);
  check('the sub-0.2 sharpening matches w^3/0.04', near(sharpened, 0.04 ** 3 / 0.04, 1e-12),
    `${sharpened}`);
}

// --- Chebyshev: cubed suppression is monotonically stronger, and exact at the extremes
{
  const vis = chebyshevVisibility([1, 1.2], 1.5);
  check('Chebyshev visibility is in [0,1]', vis >= 0 && vis <= 1, `${vis}`);
  check('cubing never increases visibility', vis ** 3 <= vis + 1e-12, `${vis} -> ${vis ** 3}`);
  const close = chebyshevVisibility([1, 1.2], 1.01);
  check('a closer query sees more than a distant one', close > vis, `${close} > ${vis}`);
  check('zero delta gives full visibility', near(chebyshevVisibility([1, 1.2], 1.0), 1, 1e-9));
}

// --- hysteresis: uninitialized history takes the fresh value, and a large change
//     shortens the history toward 0.70
{
  const rays = [{ direction: [0, 0, 1], radiance: [1, 1, 1], distance: 2 }];
  const first = integrateTexel(rays, [0, 0, 1], 0.92, null);
  check('an uninitialized texel takes the fresh value', near(first.color[0], Math.PI, 1e-6),
    `${first.color[0]}`);
  const warm = integrateTexel(rays, [0, 0, 1], 0.92,
    { color: [Math.PI, Math.PI, Math.PI], moments: [2, 4] });
  // Same value => no change => full hysteresis => identical.
  check('unchanged radiance keeps the history', near(warm.color[0], Math.PI, 1e-6), `${warm.color[0]}`);
  const shifted = integrateTexel([{ direction: [0, 0, 1], radiance: [9, 9, 9], distance: 2 }],
    [0, 0, 1], 0.92, { color: [Math.PI, Math.PI, Math.PI], moments: [2, 4] });
  const fresh = 9 * Math.PI;
  const fullHysteresis = Math.PI * 0.92 + fresh * 0.08;
  // Shortening the history means the NEW value dominates: for a brightening change the
  // result moves further up (closer to fresh) than plain hysteresis would.
  check('a large radiance change moves the result closer to the new value',
    shifted.color[0] > fullHysteresis + 1e-6, `${shifted.color[0]} > ${fullHysteresis}`);
  const shortenedHysteresis = Math.PI * 0.70 + fresh * 0.30;
  check('and lands on the 0.70 shortened value', near(shifted.color[0], shortenedHysteresis, 1e-6),
    `${shifted.color[0]} expected ${shortenedHysteresis}`);
}

console.log(failures === 0 ? '\nall sampled-formula checks passed' : `\n${failures} check(s) FAILED`);
process.exit(failures === 0 ? 0 : 1);
