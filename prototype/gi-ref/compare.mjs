// Diff the GPU stages (dumped by gpu.js in the shell) against the CPU reference.
//
//   node prototype/gi-ref/compare.mjs prototype/gi-ref/gpu.log
//
// Reports a per-stage maximum absolute error. The CPU side is the same code that
// test-cpu.mjs checks by hand, so a passing diff means the GPU agrees with a
// hand-verified reference, not merely with itself.

import { readFileSync } from 'node:fs';
import { buildScene, SKY } from './scene.js';
import { FIELD, RAY_DIRECTIONS, ATLAS, SELF_SHADOW_BIAS, traceProbeRays, blendProbe } from './gi-cpu.js';

const logPath = process.argv[2] ?? 'prototype/gi-ref/gpu.log';
const log = readFileSync(logPath, 'utf8');

const stages = {};
for (const line of log.split('\n')) {
  if (line.startsWith('GIREF error ')) { console.log(`shell/GPU error: ${line.slice(12)}`); continue; }
  const match = /^GIREF (\w+)(?: (.*))?$/.exec(line);
  if (!match) continue;
  if (match[1] === 'done') { stages.done = true; continue; }
  if (match[2] === undefined) continue;
  try {
    stages[match[1]] = JSON.parse(match[2]);
  } catch {
    console.log(`unparsable ${match[1]} stage: ${match[2].slice(0, 200)}`);
  }
}
if (!stages.done) {
  console.log('the run did not reach GIREF done; last lines:');
  console.log(log.split('\n').slice(-6).join('\n'));
  process.exit(1);
}

let failures = 0;
const report = (stage, maxError, tolerance, detail) => {
  const ok = maxError <= tolerance;
  if (!ok) failures++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${stage.padEnd(28)} max |cpu - gpu| = ${maxError.toExponential(3)}`
    + ` (tol ${tolerance.toExponential(0)})${detail ? '  ' + detail : ''}`);
};

const scene = buildScene();

// --- stage: probe positions (the GPU reads a host-built buffer; compare to the CPU grid)
{
  const cpu = probePositionsCpu();
  const gpu = stages.probePos;
  let maxError = 0;
  if (!gpu || gpu.length !== cpu.length) {
    maxError = Infinity;
  } else {
    gpu.forEach((p, i) => p.forEach((v, a) => { maxError = Math.max(maxError, Math.abs(v - cpu[i][a])); }));
  }
  report('probe positions', maxError, 1e-6, `${cpu.length} probes`);
}

// --- stage: probe rays
{
  const cpu = [];
  for (let i = 0; i < probePositionsCpu().length; i++) {
    for (const r of traceProbeRays(scene, i, SELF_SHADOW_BIAS)) {
      cpu.push([r.radiance[0], r.radiance[1], r.radiance[2], r.hit ? r.t : -1]);
    }
  }
  const gpu = stages.rays;
  let maxError = 0, hits = 0;
  if (!gpu || gpu.length !== cpu.length) {
    maxError = Infinity;
  } else {
    cpu.forEach((c, i) => {
      if (c[3] > 0) hits++;
      for (let k = 0; k < 4; k++) maxError = Math.max(maxError, Math.abs(c[k] - gpu[i][k]));
    });
  }
  report('probe rays (radiance + t)', maxError, 1e-5, `${cpu.length} rays, ${hits} hits`);
  if (gpu && gpu.length === cpu.length) {
    for (let i = 0; i < cpu.length; i++) {
      if (Math.abs(cpu[i][3] - gpu[i][3]) > 1e-5) {
        console.log(`      ray ${i} (probe ${Math.floor(i / 6)}, dir ${RAY_DIRECTIONS[i % 6]}):`
          + ` cpu t=${cpu[i][3]} gpu t=${gpu[i][3]}`);
      }
    }
  }
}

// --- stage: atlas blend (fresh frame, hysteresis 0)
{
  const cpu = [];
  for (let i = 0; i < probePositionsCpu().length; i++) {
    const tile = blendProbe(traceProbeRays(scene, i, SELF_SHADOW_BIAS), null, 0);
    for (let py = 0; py < ATLAS.tile; py++) {
      for (let px = 0; px < ATLAS.tile; px++) cpu.push(tile[py][px]);
    }
  }
  const gpu = stages.atlas;
  let maxError = 0;
  if (!gpu || gpu.length !== cpu.length) {
    maxError = Infinity;
  } else {
    cpu.forEach((c, i) => c.forEach((v, k) => { maxError = Math.max(maxError, Math.abs(v - gpu[i][k])); }));
  }
  report('atlas blend (gamma-5 encoded)', maxError, 1e-4, `${cpu.length} texels`);
  if (gpu && gpu.length === cpu.length) {
    let worst = 0, worstIndex = 0;
    cpu.forEach((c, i) => c.forEach((v, k) => {
      const d = Math.abs(v - gpu[i][k]);
      if (d > worst) { worst = d; worstIndex = i; }
    }));
    const probe = Math.floor(worstIndex / (ATLAS.tile * ATLAS.tile));
    const local = worstIndex % (ATLAS.tile * ATLAS.tile);
    console.log(`      worst texel: probe ${probe} px=${local % ATLAS.tile} py=${Math.floor(local / ATLAS.tile)}`
      + ` cpu=${cpu[worstIndex].map(v => v.toFixed(6))} gpu=${gpu[worstIndex].map(v => v.toFixed(6))}`);
  }
}

function probePositionsCpu() {
  const out = [];
  const total = FIELD.counts[0] * FIELD.counts[1] * FIELD.counts[2];
  for (let index = 0; index < total; index++) {
    const cx = FIELD.counts[0], cy = FIELD.counts[1];
    const cell = [index % cx, Math.floor(index / cx) % cy, Math.floor(index / (cx * cy))];
    out.push(cell.map((c, axis) => FIELD.origin[axis] + (c + 0.5) * FIELD.spacing));
  }
  return out;
}

console.log(failures === 0 ? '\nGPU matches the CPU reference on every stage'
  : `\n${failures} stage(s) differ`);
process.exit(failures === 0 ? 0 : 1);
