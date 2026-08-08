import * as THREE from "three/webgpu";
import {
  EngineRuntime,
  RenderTier,
  benchFromUrl,
  formatBenchResults,
  type EngineFrameClient,
  type RenderFrame,
} from "../../engine/index.ts";

const ENTITY_COUNT = 5_000;
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0a0c10);

const camera = new THREE.PerspectiveCamera(
  75,
  innerWidth / innerHeight,
  0.1,
  5_000,
);
camera.position.set(0, 200, 600);
camera.lookAt(0, 0, 0);

scene.add(new THREE.DirectionalLight(0xffffff, 1.5));
const key = scene.children[0];
if (key instanceof THREE.DirectionalLight) key.position.set(100, 200, 100);
scene.add(new THREE.AmbientLight(0x404060, 0.5));

const runtime = await EngineRuntime.forScene({
  scene,
  camera,
  entityCapacity: ENTITY_COUNT,
  memory: {
    frameScratchBytes: 64 * 1024,
    renderScratchBytes: 64 * 1024,
    structuralCommands: 512,
    workerCompletions: 0,
    assetRequests: 0,
    vtRequests: 0,
    telemetryRecords: 4096,
    telemetryMetricCells: 256,
  },
  diagnosticCapacity: 64,
  maxWorkerInputs: 0,
  maxRenderPasses: 1,
  maxOwnedResources: 0,
  renderer: {
    parameters: { antialias: true },
    onResize: resizeCamera,
  },
});
const adapter = runtime.adapter;

const geometry = new THREE.BoxGeometry(4, 4, 4);
const descriptor = adapter.registry.register({
  tier: RenderTier.Instanced,
  geometry,
  createMaterial: () =>
    new THREE.MeshStandardMaterial({ metalness: 0.1, roughness: 0.8 }),
  shardCapacity: ENTITY_COUNT,
  maxShards: 1,
  boundsPolicy: "dynamic-disable-three-culling",
});

const entities = new Uint32Array(ENTITY_COUNT);
let randomState = 0x6d2b79f5;
function random(): number {
  randomState ^= randomState << 13;
  randomState ^= randomState >>> 17;
  randomState ^= randomState << 5;
  return (randomState >>> 0) / 0x1_0000_0000;
}
const euler = new THREE.Euler();
const quaternion = new THREE.Quaternion();
for (let index = 0; index < ENTITY_COUNT; index++) {
  const entity = adapter.createEntity();
  entities[index] = entity;
  adapter.addTransform(entity);
  adapter.addRenderRef(entity, descriptor);
  adapter.markStructural(entity);

  const radius = 200 + random() * 300;
  const theta = random() * Math.PI * 2;
  const phi = Math.acos(2 * random() - 1);
  adapter.transform.positionX[entity] =
    radius * Math.sin(phi) * Math.cos(theta);
  adapter.transform.positionY[entity] =
    radius * Math.sin(phi) * Math.sin(theta);
  adapter.transform.positionZ[entity] = radius * Math.cos(phi);
  euler.set(
    random() * Math.PI,
    random() * Math.PI,
    random() * Math.PI,
  );
  quaternion.setFromEuler(euler);
  adapter.transform.rotationX[entity] = quaternion.x;
  adapter.transform.rotationY[entity] = quaternion.y;
  adapter.transform.rotationZ[entity] = quaternion.z;
  adapter.transform.rotationW[entity] = quaternion.w;
  const scale = 0.5 + random() * 1.5;
  adapter.transform.scaleX[entity] = scale;
  adapter.transform.scaleY[entity] = scale;
  adapter.transform.scaleZ[entity] = scale;
  adapter.markTransformDirty(entity);
}

function resizeCamera(width: number, height: number): void {
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}

const benchmark = benchFromUrl({
  onDone(result) {
    console.log(`[bench] ${formatBenchResults(result)}`);
  },
});
/** @alloc-effect none */
function updateFrame(frame: Readonly<RenderFrame>): void {
  const elapsed = frame.elapsedSeconds;
  for (let index = 0; index < ENTITY_COUNT; index += 10) {
    const entity = entities[index] ?? 0;
    const x = adapter.transform.positionX[entity] ?? 0;
    const z = adapter.transform.positionZ[entity] ?? 0;
    const angle = 0.002 + Math.sin(elapsed + index * 0.01) * 0.001;
    adapter.transform.positionX[entity] =
      x * Math.cos(angle) - z * Math.sin(angle);
    adapter.transform.positionZ[entity] =
      x * Math.sin(angle) + z * Math.cos(angle);
    adapter.transform.positionY[entity] =
      (adapter.transform.positionY[entity] ?? 0) +
      Math.sin(elapsed * 2 + index * 0.1) * 0.5;
    adapter.markTransformDirty(entity);
  }

  camera.position.x = Math.cos(elapsed * 0.15) * 700;
  camera.position.z = Math.sin(elapsed * 0.15) * 700;
  camera.position.y = 200 + Math.sin(elapsed * 0.3) * 100;
  camera.lookAt(0, 0, 0);

  if (benchmark) {
    benchmark.tick(elapsed * 1000);
    if (benchmark.hasPendingResults) benchmark.finish(); // @alloc-allowed reason=DiagnosticCapture issue=DME-013 expires=2026-10-01
  }
}

const frameClient: EngineFrameClient = { update: updateFrame };
runtime.enterWarmup();
adapter.warmAllDescriptors();
await runtime.warm();
runtime.sealGameplay();
runtime.start(frameClient);

export { runtime as demoRuntime };
