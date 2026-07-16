// crates/afterglow-web/www/engine/webgpu-only.ts
function disableWebGLFallback(renderer) {
  renderer._getFallback = null;
}
function assertWebGPUBackend(renderer) {
  if (renderer.backend?.isWebGPUBackend !== true || renderer.backend.device == null) {
    throw new Error("Afterglow requires a live WebGPU backend; WebGL fallback is forbidden.");
  }
}
function showWebGPUFailure(error) {
  const message = error instanceof Error ? error.message : String(error);
  const panel = document.createElement("pre");
  panel.id = "afterglow-webgpu-failure";
  panel.textContent = `Afterglow requires hardware WebGPU.

${message}`;
  panel.style.cssText = "box-sizing:border-box;margin:0;min-height:100vh;padding:24px;background:#11151c;color:#ff9a9a;font:16px/1.5 ui-monospace,monospace;white-space:pre-wrap";
  document.body.replaceChildren(panel);
  console.error("Afterglow WebGPU startup failed:", error);
}
async function createWebGPUOnlyRenderer(parameters = {}) {
  const gpu = navigator.gpu;
  if (!gpu)
    throw new Error("navigator.gpu is unavailable. WebGL fallback is disabled.");
  const adapter = await gpu.requestAdapter();
  if (!adapter)
    throw new Error("Unable to acquire a hardware WebGPU adapter. WebGL fallback is disabled.");
  const renderer = new window.THREE.WebGPURenderer({ ...parameters });
  disableWebGLFallback(renderer);
  try {
    await renderer.init();
    assertWebGPUBackend(renderer);
  } catch (error) {
    renderer.dispose?.();
    throw error;
  }
  const onDeviceLost = renderer.onDeviceLost.bind(renderer);
  renderer.onDeviceLost = (info) => {
    onDeviceLost(info);
    showWebGPUFailure(new Error(`WebGPU device lost (${info.reason ?? "unknown"}): ${info.message ?? "no detail"}`));
  };
  return renderer;
}

// crates/afterglow-web/www/engine-demo.ts
var THREE = window.THREE;
var { createWorld, addEntity, addComponent, query } = {
  createWorld: window.bitecsCreateWorld,
  addEntity: window.bitecsAddEntity,
  addComponent: window.bitecsAddComponent,
  query: window.bitecsQuery
};
var scene = new THREE.Scene;
scene.background = new THREE.Color(658448);
var camera = new THREE.PerspectiveCamera(75, innerWidth / innerHeight, 0.1, 5000);
camera.position.set(0, 200, 600);
camera.lookAt(0, 0, 0);
var renderer = await createWebGPUOnlyRenderer({ antialias: true }).catch((error) => {
  showWebGPUFailure(error);
  throw error;
});
renderer.setSize(innerWidth, innerHeight);
document.body.appendChild(renderer.domElement);
addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});
var dirLight = new THREE.DirectionalLight(16777215, 1.5);
dirLight.position.set(100, 200, 100);
scene.add(dirLight);
scene.add(new THREE.AmbientLight(4210784, 0.5));
var world = createWorld();
var Position = { x: [], y: [], z: [] };
var Rotation = { x: [], y: [], z: [] };
var Qw = [];
var Scale = { x: [], y: [], z: [] };
var Dirty = [];
var ENTITY_COUNT = 5000;
var cubeGeom = new THREE.BoxGeometry(4, 4, 4);
var cubeMat = new THREE.MeshStandardMaterial({ metalness: 0.1, roughness: 0.8 });
var mesh = new THREE.InstancedMesh(cubeGeom, cubeMat, ENTITY_COUNT);
mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
mesh.matrixAutoUpdate = false;
mesh.frustumCulled = false;
scene.add(mesh);
var matrixData = mesh.instanceMatrix.array;
var entities = [];
for (let i = 0;i < ENTITY_COUNT; i++) {
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
  const q = new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.random() * Math.PI, Math.random() * Math.PI, Math.random() * Math.PI));
  Rotation.x[eid] = q.x;
  Rotation.y[eid] = q.y;
  Rotation.z[eid] = q.z;
  Qw[eid] = q.w;
  Scale.x[eid] = Scale.y[eid] = Scale.z[eid] = 0.5 + Math.random() * 1.5;
  Dirty[eid] = 1;
  entities.push(eid);
}
console.log("afterglow-engine: " + ENTITY_COUNT + " entities created");
document.getElementById("info").textContent = "afterglow-engine — " + ENTITY_COUNT + " entities — bitECS + Three.js";
function composeMatrices() {
  for (let i = 0;i < ENTITY_COUNT; i++) {
    if (!Dirty[i])
      continue;
    const eid = i;
    const qx = Rotation.x[eid], qy = Rotation.y[eid], qz = Rotation.z[eid], qw = Qw[eid];
    const x2 = qx + qx, y2 = qy + qy, z2 = qz + qz;
    const xx = qx * x2, xy = qx * y2, xz = qx * z2;
    const yy = qy * y2, yz = qy * z2, zz = qz * z2;
    const wx = qw * x2, wy = qw * y2, wz = qw * z2;
    const sx = Scale.x[eid], sy = Scale.y[eid], sz = Scale.z[eid];
    const off = eid * 16;
    matrixData[off] = (1 - (yy + zz)) * sx;
    matrixData[off + 1] = (xy + wz) * sx;
    matrixData[off + 2] = (xz - wy) * sx;
    matrixData[off + 3] = 0;
    matrixData[off + 4] = (xy - wz) * sy;
    matrixData[off + 5] = (1 - (xx + zz)) * sy;
    matrixData[off + 6] = (yz + wx) * sy;
    matrixData[off + 7] = 0;
    matrixData[off + 8] = (xz + wy) * sz;
    matrixData[off + 9] = (yz - wx) * sz;
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
var benchEl = document.getElementById("bench");
var bench = null;
function startBench(frames = 300) {
  bench = { frames, intervals: [], prev: -1, start: -1 };
  benchEl.style.display = "block";
  benchEl.textContent = "benchmarking... 0/" + frames;
}
function tickBench(ts) {
  if (!bench)
    return;
  if (bench.start < 0)
    bench.start = ts;
  if (bench.prev >= 0) {
    bench.intervals.push(ts - bench.prev);
    benchEl.textContent = "benchmarking... " + bench.intervals.length + "/" + bench.frames;
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
    const b55 = bench.intervals.filter((v) => v > 1000 / 55).length;
    const pass = 1000 / p99 >= 55;
    benchEl.textContent = (pass ? "✅" : "❌") + " p99=" + (1000 / p99).toFixed(1) + "fps  max=" + (1000 / max).toFixed(1) + "fps" + "  drops=" + b55 + "/" + n + "  avg=" + (1000 / avg).toFixed(1) + "fps" + "  p50=" + p50.toFixed(2) + "ms  p90=" + p90.toFixed(2) + "ms";
    console.log("[bench] " + benchEl.textContent);
    bench = null;
  }
}
addEventListener("keydown", (e) => {
  if (e.key === "b" || e.key === "B")
    startBench();
});
var benchParam = new URLSearchParams(location.search).get("bench");
var benchAutoStarted = false;
var startTime = performance.now();
var lastTime = startTime;
var fpsCount = 0;
var fpsTime = startTime;
var engineFrameId = 0;
function animate(ts) {
  if (benchAutoStarted === false && benchParam) {
    benchAutoStarted = true;
    startBench(parseInt(benchParam, 10) || 300);
  }
  tickBench(ts);
  const now = performance.now();
  const elapsed = (now - startTime) / 1000;
  lastTime = now;
  for (let i = 0;i < ENTITY_COUNT; i += 10) {
    const eid = entities[i];
    const px = Position.x[eid];
    const pz = Position.z[eid];
    const angle = 0.002 + Math.sin(elapsed + i * 0.01) * 0.001;
    Position.x[eid] = px * Math.cos(angle) - pz * Math.sin(angle);
    Position.z[eid] = px * Math.sin(angle) + pz * Math.cos(angle);
    Position.y[eid] += Math.sin(elapsed * 2 + i * 0.1) * 0.5;
    Dirty[eid] = 1;
  }
  camera.position.x = Math.cos(elapsed * 0.15) * 700;
  camera.position.z = Math.sin(elapsed * 0.15) * 700;
  camera.position.y = 200 + Math.sin(elapsed * 0.3) * 100;
  camera.lookAt(0, 0, 0);
  composeMatrices();
  renderer.renderAsync(scene, camera);
  fpsCount++;
  engineFrameId++;
  if (now - fpsTime > 1000) {
    const fps = fpsCount;
    fpsCount = 0;
    fpsTime = now;
    console.log("FPS: " + fps + "  frame: " + engineFrameId + "  entities: " + ENTITY_COUNT);
    document.getElementById("info").textContent = "afterglow-engine — " + ENTITY_COUNT + " entities — " + fps + " FPS — bitECS + Three.js";
  }
  requestAnimationFrame(animate);
}
composeMatrices();
requestAnimationFrame(animate);
console.log("afterglow-engine: render loop started");
