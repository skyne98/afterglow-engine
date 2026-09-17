// GPU stages for the probe-GI reference, dumped as text for comparison with gi-cpu.js.
//
// Deliberately dumb: brute-force tracing over 12 triangles (no BVH, no acceleration
// structure) so a mismatch is a math/plumbing mismatch, never a traversal-structure
// bug. Stage by stage, in the order the real pipeline runs:
//
//   stage=probePos   probe centres the GPU actually reads
//   stage=rays       per-ray { radiance.rgb, t }   (-1 = miss)
//   stage=atlas      per-texel gamma-5 encoded radiance (fresh, hysteresis 0)
//
// Run through the shell:  prototype/gi-ref/run-gpu.sh <logname>
// The final line is `GIREF done` so a runner can stop the shell immediately.

import { buildScene, LIGHT, SKY, SKY_INTENSITY, triAlbedo } from './scene.js';
import { FIELD, RAY_DIRECTIONS, SELF_SHADOW_BIAS, ATLAS, GAMMA_INV, PI } from './gi-cpu.js';

const logLine = (tag, payload) => console.log(`GIREF ${tag} ${JSON.stringify(payload)}`);

if (!navigator.gpu) throw new Error('no navigator.gpu: run this through afterglow-shell');
const adapter = await navigator.gpu.requestAdapter();
if (!adapter) throw new Error('no WebGPU adapter');
const device = await adapter.requestDevice();
device.onuncapturederror = e => console.log(`GIREF error ${e.error?.message ?? e.error}`);

const scene = buildScene();
const probePositions = [];
for (let index = 0; index < FIELD.counts[0] * FIELD.counts[1] * FIELD.counts[2]; index++) {
  const cx = FIELD.counts[0], cy = FIELD.counts[1];
  const cell = [index % cx, Math.floor(index / cx) % cy, Math.floor(index / (cx * cy))];
  probePositions.push(cell.map((c, axis) => FIELD.origin[axis] + (c + 0.5) * FIELD.spacing));
}
const rayCount = probePositions.length * RAY_DIRECTIONS.length;
const texelCount = probePositions.length * ATLAS.tile * ATLAS.tile;

// Triangle storage: 3 vec4 per triangle, matching index*3 + corner.
const triData = new Float32Array(scene.triCount * 12);
for (let i = 0; i < scene.triCount; i++) {
  for (let v = 0; v < 3; v++) {
    for (let a = 0; a < 3; a++) triData[i * 12 + v * 4 + a] = scene.tris[i * 9 + v * 3 + a];
  }
}
const probeData = new Float32Array(probePositions.length * 4);
probePositions.forEach((p, i) => probeData.set([p[0], p[1], p[2], 1], i * 4));

const sim = new Float32Array(16);
sim.set([LIGHT.position[0], LIGHT.position[1], LIGHT.position[2], LIGHT.intensity], 0);
sim.set([SKY[0] * SKY_INTENSITY, SKY[1] * SKY_INTENSITY, SKY[2] * SKY_INTENSITY, 0], 4);
sim.set([FIELD.spacing, SELF_SHADOW_BIAS, LIGHT.falloffK, scene.triCount], 8);
sim[12] = ATLAS.tile; sim[13] = ATLAS.interior; sim[14] = FIELD.counts[0]; sim[15] = FIELD.counts[1];

const upload = (data, usage) => {
  const buffer = device.createBuffer({
    size: Math.max(16, data.byteLength),
    usage: usage | GPUBufferUsage.COPY_DST,
  });
  device.queue.writeBuffer(buffer, 0, data);
  return buffer;
};
const triBuf = upload(triData, GPUBufferUsage.STORAGE);
const probeBuf = upload(probeData, GPUBufferUsage.STORAGE);
const simBuf = upload(sim, GPUBufferUsage.UNIFORM);
const rayBuf = device.createBuffer({
  size: rayCount * 16, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC,
});
const atlasBuf = device.createBuffer({
  size: texelCount * 16, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC,
});

const WGSL = /* wgsl */ `
struct Sim {
  light: vec4f,     // xyz position, w intensity
  sky: vec4f,       // rgb radiance
  misc: vec4f,      // x spacing, y self-shadow bias, z falloff k, w triCount
  atlas: vec4f,     // x tile size, y interior, z counts.x, w counts.y
};
@group(0) @binding(0) var<storage, read> tris: array<vec4f>;
@group(0) @binding(1) var<storage, read> probePos: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> rayOut: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> atlasOut: array<vec4f>;
@group(0) @binding(4) var<uniform> S: Sim;

const PI: f32 = 3.141592653589793;
const RAYS_PER_PROBE: u32 = 6u;
const RAYS: array<vec3f, 6> = array<vec3f, 6>(
  vec3f(1.0, 0.0, 0.0), vec3f(-1.0, 0.0, 0.0),
  vec3f(0.0, 1.0, 0.0), vec3f(0.0, -1.0, 0.0),
  vec3f(0.0, 0.0, 1.0), vec3f(0.0, 0.0, -1.0),
);

fn hitTri(index: u32, o: vec3f, d: vec3f, tMax: f32) -> f32 {
  let a = tris[index * 3u].xyz;
  let b = tris[index * 3u + 1u].xyz;
  let c = tris[index * 3u + 2u].xyz;
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
  if (t <= 1e-3 || t > tMax) { return -1.0; }
  return t;
}

fn triNormalOf(index: u32) -> vec3f {
  let a = tris[index * 3u].xyz;
  let b = tris[index * 3u + 1u].xyz;
  let c = tris[index * 3u + 2u].xyz;
  return normalize(cross(b - a, c - a));
}

fn triAlbedoOf(index: u32) -> vec3f {
  let n = triNormalOf(index);
  let horizontal = step(0.7, abs(n.y));
  return mix(vec3f(0.40, 0.38, 0.36), vec3f(0.30, 0.29, 0.28), horizontal);
}

fn trace(o: vec3f, d: vec3f, counts: i32) -> vec2f {
  var bestT = 1e30;
  var bestTri = 0xffffffffu;
  for (var i = 0; i < counts; i = i + 1) {
    let t = hitTri(u32(i), o, d, bestT);
    if (t > 0.0) { bestT = t; bestTri = u32(i); }
  }
  return vec2f(select(-1.0, bestT, bestTri != 0xffffffffu), bitcast<f32>(bestTri));
}

fn occluded(o: vec3f, d: vec3f, tMax: f32, counts: i32) -> bool {
  for (var i = 0; i < counts; i = i + 1) {
    if (hitTri(u32(i), o, d, tMax) > 0.0) { return true; }
  }
  return false;
}

/** albedo/PI * irradiance, zero on a backface hit (the DDGI probe-ray rule). */
fn shadeHit(index: u32, point: vec3f, rayDir: vec3f, counts: i32) -> vec3f {
  let n = triNormalOf(index);
  if (dot(n, rayDir) > 0.0) { return vec3f(0.0); }
  let albedo = triAlbedoOf(index);
  let toLight = S.light.xyz - point;
  let dist = length(toLight);
  let l = toLight / max(dist, 1e-4);
  let ndotl = dot(n, l);
  if (ndotl <= 0.0) { return vec3f(0.0); }
  if (occluded(point + n * 0.02, l, dist - 0.05, counts)) { return vec3f(0.0); }
  let e = S.light.w / (1.0 + S.misc.z * dist * dist) * ndotl;
  return albedo * e / PI;
}

@compute @workgroup_size(32)
fn probeTrace(@builtin(global_invocation_id) gid: vec3u) {
  let ray = gid.x;
  let perProbe = RAYS_PER_PROBE;
  if (ray >= u32(arrayLength(&rayOut))) { return; }
  let probe = ray / perProbe;
  let d = RAYS[ray % perProbe];
  let p = probePos[probe].xyz;
  let o = p + d * (S.misc.y * S.misc.x);
  let counts = i32(S.misc.w);
  let hit = trace(o, d, counts);
  if (hit.x < 0.0) {
    rayOut[ray] = vec4f(S.sky.rgb, -1.0);
    return;
  }
  let point = o + d * hit.x;
  let tri = bitcast<u32>(hit.y);
  rayOut[ray] = vec4f(shadeHit(tri, point, d, counts), hit.x);
}

fn octDecode(uv: vec2f) -> vec3f {
  let f = uv * 2.0 - vec2f(1.0, 1.0);
  var n = vec3f(f.x, f.y, 1.0 - abs(f.x) - abs(f.y));
  let t = clamp(-n.z, 0.0, 1.0);
  n.x = n.x + select(t, -t, n.x >= 0.0);
  n.y = n.y + select(t, -t, n.y >= 0.0);
  return normalize(n);
}

@compute @workgroup_size(64)
fn blendAtlas(@builtin(global_invocation_id) gid: vec3u) {
  let texel = gid.x;
  let tile = u32(S.atlas.x);
  let interior = u32(S.atlas.y);
  let perProbe = RAYS_PER_PROBE;
  if (texel >= u32(arrayLength(&atlasOut))) { return; }
  let probe = texel / (tile * tile);
  let local = texel % (tile * tile);
  let px = local % tile;
  let py = local / tile;
  let dir = octDecode(vec2f((f32(px) - 0.5) / f32(interior), (f32(py) - 0.5) / f32(interior)));
  var sum = vec3f(0.0);
  var wsum = 0.0;
  for (var i = 0u; i < perProbe; i = i + 1u) {
    let w = max(dot(dir, RAYS[i]), 0.0);
    sum = sum + rayOut[probe * perProbe + i].rgb * w;
    wsum = wsum + w;
  }
  var e = vec3f(0.0);
  if (wsum > 1e-5) { e = sum / wsum; }
  atlasOut[texel] = vec4f(pow(max(e, vec3f(1e-6)), vec3f(0.2)), 0.0);
}
`;

const module = device.createShaderModule({ code: WGSL });
// One explicit layout for both entry points: layout 'auto' would give each pipeline the
// subset of bindings its own entry point uses, so a single bind group could not serve
// both (probeTrace does not touch the atlas output).
const bindGroupLayout = device.createBindGroupLayout({
  entries: [
    { binding: 0, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 1, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 2, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'storage' } },
    { binding: 3, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'storage' } },
    { binding: 4, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'uniform' } },
  ],
});
const pipelineLayout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] });
const tracePipeline = device.createComputePipeline({
  layout: pipelineLayout, compute: { module, entryPoint: 'probeTrace' },
});
const blendPipeline = device.createComputePipeline({
  layout: pipelineLayout, compute: { module, entryPoint: 'blendAtlas' },
});
const bindGroup = device.createBindGroup({
  layout: bindGroupLayout,
  entries: [
    { binding: 0, resource: { buffer: triBuf } },
    { binding: 1, resource: { buffer: probeBuf } },
    { binding: 2, resource: { buffer: rayBuf } },
    { binding: 3, resource: { buffer: atlasBuf } },
    { binding: 4, resource: { buffer: simBuf } },
  ],
});

const encoder = device.createCommandEncoder();
const pass = encoder.beginComputePass();
pass.setPipeline(tracePipeline);
pass.setBindGroup(0, bindGroup);
pass.dispatchWorkgroups(Math.ceil(rayCount / 32));
pass.setPipeline(blendPipeline);
pass.setBindGroup(0, bindGroup);
pass.dispatchWorkgroups(Math.ceil(texelCount / 64));
pass.end();

const readRay = device.createBuffer({
  size: rayCount * 16, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
});
const readAtlas = device.createBuffer({
  size: texelCount * 16, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
});
encoder.copyBufferToBuffer(rayBuf, 0, readRay, 0, rayCount * 16);
encoder.copyBufferToBuffer(atlasBuf, 0, readAtlas, 0, texelCount * 16);
device.queue.submit([encoder.finish()]);

await readRay.mapAsync(GPUMapMode.READ);
const gpuRays = new Float32Array(readRay.getMappedRange().slice(0));
readRay.unmap();
await readAtlas.mapAsync(GPUMapMode.READ);
const gpuAtlas = new Float32Array(readAtlas.getMappedRange().slice(0));
readAtlas.unmap();

logLine('probePos', probePositions);
logLine('rays', Array.from({ length: rayCount }, (_, i) =>
  [+gpuRays[i * 4].toFixed(7), +gpuRays[i * 4 + 1].toFixed(7), +gpuRays[i * 4 + 2].toFixed(7), gpuRays[i * 4 + 3]]));
logLine('atlas', Array.from({ length: texelCount }, (_, i) =>
  [+gpuAtlas[i * 4].toFixed(7), +gpuAtlas[i * 4 + 1].toFixed(7), +gpuAtlas[i * 4 + 2].toFixed(7)]));
console.log('GIREF done');
