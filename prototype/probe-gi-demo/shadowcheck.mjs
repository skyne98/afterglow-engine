// Narrowing the "no direct light" bug: cast the same shadow rays the fragment
// shader casts, but on the CPU, for known points of the simple scene.
import { buildSimpleScene, buildBVH, bvhHit } from './bvh.js';

const scene = buildSimpleScene();
const bvh = buildBVH(scene.tris, scene.triCount);
const light = [3, 4, 3];

// sample: [label, position, expected normal]
const samples = [
  ['ground @ 3m toward light', [0, 0, 3], [0, 1, 0]],
  ['ground @ origin-ish     ', [0, 0, 1.5], [0, 1, 0]],
  ['ground far side         ', [0, 0, -3], [0, 1, 0]],
  ['cube +X face            ', [1, 1, 0], [1, 0, 0]],
  ['cube +Z face            ', [0, 1, 1], [0, 0, 1]],
  ['cube top face           ', [0, 2, 0], [0, 1, 0]],
  ['cube -X face (away)     ', [-1, 1, 0], [-1, 0, 0]],
];

for (const [label, p, n] of samples) {
  const toLight = [light[0] - p[0], light[1] - p[1], light[2] - p[2]];
  const dist = Math.hypot(...toLight);
  const d = toLight.map(v => v / dist);
  const ndl = n[0] * d[0] + n[1] * d[1] + n[2] * d[2];
  const o = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
  const hit = bvhHit(bvh.nodes, bvh.nodeU32, bvh.rights, bvh.order, scene.tris, o, d, dist - 0.05);
  const blocked = Number.isFinite(hit);
  console.log(
    `${label}  ndl=${ndl.toFixed(2)}  dist=${dist.toFixed(2)}  ` +
    `shadow=${blocked ? 'BLOCKED at t=' + hit.toFixed(2) : 'clear'}`
  );
}
