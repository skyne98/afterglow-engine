import { createWebGPUOnlyRenderer, showWebGPUFailure } from './engine/webgpu-only.ts';

const THREE = window.THREE;
const { createWorld, addEntity, addComponent, query } = {
  createWorld: window.bitecsCreateWorld,
  addEntity: window.bitecsAddEntity,
  addComponent: window.bitecsAddComponent,
  query: window.bitecsQuery,
};

// --- Three.js WebGPU setup ---

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0a0c10);

const camera = new THREE.PerspectiveCamera(75, innerWidth / innerHeight, 0.1, 5000);
camera.position.set(0, 200, 600);
camera.lookAt(0, 0, 0);

const renderer = await createWebGPUOnlyRenderer({ antialias: true }).catch(error => {
  showWebGPUFailure(error);
  throw error;
});
renderer.setSize(innerWidth, innerHeight);
document.body.appendChild(renderer.domElement);

addEventListener('resize', () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});

// --- Lights ---

const dirLight = new THREE.DirectionalLight(0xffffff, 1.5);
dirLight.position.set(100, 200, 100);
scene.add(dirLight);
scene.add(new THREE.AmbientLight(0x404060, 0.5));

// --- bitECS world ---

const world = createWorld();

const Position = { x: [], y: [], z: [] };
const Rotation = { x: [], y: [], z: [] };
const Qw = [];
const Scale = { x: [], y: [], z: [] };
const Dirty = [];

// --- InstancedMesh ---

const ENTITY_COUNT = 5000;
const cubeGeom = new THREE.BoxGeometry(4, 4, 4);
const cubeMat = new THREE.MeshStandardMaterial({ metalness: 0.1, roughness: 0.8 });

const mesh = new THREE.InstancedMesh(cubeGeom, cubeMat, ENTITY_COUNT);
mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
mesh.matrixAutoUpdate = false;
mesh.frustumCulled = false;
scene.add(mesh);

const matrixData = mesh.instanceMatrix.array;

// --- Create entities ---

const entities = [];

for (let i = 0; i < ENTITY_COUNT; i++) {
  const eid = addEntity(world);
  addComponent(world, eid, Position);
  addComponent(world, eid, Rotation);
  addComponent(world, eid, Scale);

  const r = 200 + Math.random() * 300;
  const theta = Math.random() * Math.PI * 2;
  const phi = Math.acos(2 * Math.random() - 1);
  Position.x[eid] = r * Math.sin(phi) * Math.cos(theta);
  Position.y[eid] = r * Math.sin(phi) * Math.sin(theta);
  Position.z[eid] = r * Math.cos(phi);

  const q = new THREE.Quaternion().setFromEuler(
    new THREE.Euler(Math.random() * Math.PI, Math.random() * Math.PI, Math.random() * Math.PI)
  );
  Rotation.x[eid] = q.x;
  Rotation.y[eid] = q.y;
  Rotation.z[eid] = q.z;
  Qw[eid] = q.w;

  Scale.x[eid] = Scale.y[eid] = Scale.z[eid] = 0.5 + Math.random() * 1.5;
  Dirty[eid] = 1;
  entities.push(eid);
}

console.log('afterglow-engine: ' + ENTITY_COUNT + ' entities created');
document.getElementById('info').textContent = 'afterglow-engine — ' + ENTITY_COUNT + ' entities — bitECS + Three.js';

// --- Batched raw matrix compose ---

function composeMatrices() {
  for (let i = 0; i < ENTITY_COUNT; i++) {
    if (!Dirty[i]) continue;
    const eid = i;
    const qx = Rotation.x[eid], qy = Rotation.y[eid], qz = Rotation.z[eid], qw = Qw[eid];
    const x2 = qx + qx, y2 = qy + qy, z2 = qz + qz;
    const xx = qx * x2, xy = qx * y2, xz = qx * z2;
    const yy = qy * y2, yz = qy * z2, zz = qz * z2;
    const wx = qw * x2, wy = qw * y2, wz = qw * z2;
    const sx = Scale.x[eid], sy = Scale.y[eid], sz = Scale.z[eid];
    const off = eid * 16;
    matrixData[off]      = (1 - (yy + zz)) * sx;
    matrixData[off + 1]  = (xy + wz) * sx;
    matrixData[off + 2]  = (xz - wy) * sx;
    matrixData[off + 3]  = 0;
    matrixData[off + 4]  = (xy - wz) * sy;
    matrixData[off + 5]  = (1 - (xx + zz)) * sy;
    matrixData[off + 6]  = (yz + wx) * sy;
    matrixData[off + 7]  = 0;
    matrixData[off + 8]  = (xz + wy) * sz;
    matrixData[off + 9]  = (yz - wx) * sz;
    matrixData[off + 10] = (1 - (xx + yy)) * sz;
    matrixData[off + 11] = 0;
    matrixData[off + 12] = Position.x[eid];
    matrixData[off + 13] = Position.y[eid];
    matrixData[off + 14] = Position.z[eid];
    matrixData[off + 15] = 1;
    Dirty[eid] = 0;
  }
  mesh.instanceMatrix.needsUpdate = true;
}

// --- Built-in frame benchmark (rAF timing) ---
//
// Press 'B' to start a 300-frame benchmark, or load with ?bench=300 to auto-run.
// Measures p50/p90/p99/max frame times and counts frames below 55 FPS.
// Uses the same rAF-timestamp method as Chrome DevTools' FPS counter.

const benchEl = document.getElementById('bench');
let bench = null;
let benchPrev = -1;

function startBench(frames = 300) {
  bench = { frames, intervals: [], prev: -1, start: -1 };
  benchEl.style.display = 'block';
  benchEl.textContent = 'benchmarking... 0/' + frames;
}

function tickBench(ts) {
  if (!bench) return;
  if (bench.start < 0) bench.start = ts;
  if (bench.prev >= 0) {
    bench.intervals.push(ts - bench.prev);
    benchEl.textContent = 'benchmarking... ' + bench.intervals.length + '/' + bench.frames;
  }
  bench.prev = ts;
  if (bench.intervals.length >= bench.frames) {
    const s = [...bench.intervals].sort((a, b) => a - b);
    const n = s.length;
    const avg = s.reduce((q, v) => q + v, 0) / n;
    const p50 = s[n * 0.5 | 0];
    const p90 = s[n * 0.9 | 0];
    const p99 = s[n * 0.99 | 0];
    const max = s[n - 1];
    const b55 = bench.intervals.filter(v => v > 1000 / 55).length;
    const pass = (1000 / p99) >= 55;
    benchEl.textContent =
      (pass ? '✅' : '❌') + ' p99=' + (1000 / p99).toFixed(1) + 'fps  max=' + (1000 / max).toFixed(1) + 'fps' +
      '  drops=' + b55 + '/' + n + '  avg=' + (1000 / avg).toFixed(1) + 'fps' +
      '  p50=' + p50.toFixed(2) + 'ms  p90=' + p90.toFixed(2) + 'ms';
    console.log('[bench] ' + benchEl.textContent);
    bench = null;
  }
}

// 'B' key starts a benchmark.
addEventListener('keydown', e => {
  if (e.key === 'b' || e.key === 'B') startBench();
});

// ?bench=300 auto-starts after the first frame.
const benchParam = new URLSearchParams(location.search).get('bench');
let benchAutoStarted = false;

const startTime = performance.now();
let lastTime = startTime;
let fpsCount = 0;
let fpsTime = startTime;
let engineFrameId = 0;

function animate(ts) {
  if (benchAutoStarted === false && benchParam) {
    benchAutoStarted = true;
    startBench(parseInt(benchParam, 10) || 300);
  }
  tickBench(ts);

  // --- Animation loop ---
  const now = performance.now();
  const elapsed = (now - startTime) / 1000;
  lastTime = now;

  // Move ~10% of entities (simulate physics)
  for (let i = 0; i < ENTITY_COUNT; i += 10) {
    const eid = entities[i];
    const px = Position.x[eid];
    const pz = Position.z[eid];
    const angle = 0.002 + Math.sin(elapsed + i * 0.01) * 0.001;
    Position.x[eid] = px * Math.cos(angle) - pz * Math.sin(angle);
    Position.z[eid] = px * Math.sin(angle) + pz * Math.cos(angle);
    Position.y[eid] += Math.sin(elapsed * 2 + i * 0.1) * 0.5;
    Dirty[eid] = 1;
  }

  // Rotate camera
  camera.position.x = Math.cos(elapsed * 0.15) * 700;
  camera.position.z = Math.sin(elapsed * 0.15) * 700;
  camera.position.y = 200 + Math.sin(elapsed * 0.3) * 100;
  camera.lookAt(0, 0, 0);

  // Compose matrices (batched raw math)
  composeMatrices();

  // Render
  renderer.renderAsync(scene, camera);

  // FPS
  fpsCount++;
  engineFrameId++;
  if (now - fpsTime > 1000) {
    const fps = fpsCount;
    fpsCount = 0;
    fpsTime = now;
    console.log('FPS: ' + fps + '  frame: ' + engineFrameId + '  entities: ' + ENTITY_COUNT);
    document.getElementById('info').textContent =
      'afterglow-engine — ' + ENTITY_COUNT + ' entities — ' + fps + ' FPS — bitECS + Three.js';
  }

  requestAnimationFrame(animate);
}

composeMatrices();
requestAnimationFrame(animate);
console.log('afterglow-engine: render loop started');
