import * as THREE from "three/webgpu";
import {
  EngineRuntime,
  RegistrationStatus,
  type RenderFrame,
} from "../../engine/index.ts";
import {
  ModelSystem,
  loadCookedModel,
  projectedCoverage,
} from "../../engine/presentation/index.ts";
import {
  BoundedKeyboardInput,
  DemoInputAction,
} from "../../engine/input/index.ts";
import { TextHud } from "../../engine/diagnostics/index.ts";

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0a0c10);
const camera = new THREE.PerspectiveCamera(
  50,
  innerWidth / innerHeight,
  0.1,
  100,
);
camera.position.set(0, 1.4, 8);
scene.add(new THREE.HemisphereLight(0xaac4e8, 0x241b15, 2));
const key = new THREE.DirectionalLight(0xffffff, 3);
key.position.set(4, 7, 5);
scene.add(key);
const runtime = await EngineRuntime.forScene({
  scene,
  camera,
  entityCapacity: 1,
  memory: {
    frameScratchBytes: 8 * 1024,
    renderScratchBytes: 8 * 1024,
    structuralCommands: 4,
    workerCompletions: 4,
    assetRequests: 4,
    vtRequests: 4,
    telemetryRecords: 4096,
    telemetryMetricCells: 256,
  },
  diagnosticCapacity: 32,
  maxWorkerInputs: 0,
  maxRenderPasses: 1,
  maxOwnedResources: 6,
  renderer: {
    parameters: { antialias: true },
    onResize: resizeCamera,
  },
});
function resizeCamera(width: number, height: number): void {
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}
const rendererHost = runtime.rendererHost;
const loadingElement = document.getElementById("loading");
const startupHud = new TextHud(loadingElement);
try {
  const asset = await loadCookedModel({
    containerPath: "lod-demo.big",
    assetName: "Avocado",
    maxHeaderBytes: 64 * 1024,
  });
  if (runtime.ownDisposable(asset) !== RegistrationStatus.Registered)
    throw new Error("LOD asset owner capacity exceeded");
  const root = new THREE.Group();
  root.rotation.y = Math.PI;
  scene.add(root);
  const solidMaterial = new THREE.MeshStandardMaterial({
    color: 0x78b84b,
    roughness: 0.72,
    metalness: 0,
  });
  const wireMaterial = new THREE.MeshBasicMaterial({
    color: 0xc8f0a0,
    wireframe: true,
  });
  if (runtime.ownDisposable(solidMaterial) !== RegistrationStatus.Registered ||
      runtime.ownDisposable(wireMaterial) !== RegistrationStatus.Registered)
    throw new Error("LOD material owner capacity exceeded");
  const modelSystem = await ModelSystem.open({
    maxModels: 1,
    maxPendingOptimizations: 1,
    maxResidentCpuBytes: 1024 * 1024,
    completionsPerPoll: 1,
    ratios: [1, 0.5, 0.25, 0.1],
    targetError: 0.02,
    geometryArena: { buckets: [{
      slots: 4,
      maxVertices: 512,
      maxIndices: 2048,
      maxGroups: 1,
      indexKind: "u32",
      attributes: [
        { name: "position", itemSize: 3, kind: "f32" },
        { name: "uv", itemSize: 2, kind: "f32" },
        { name: "normal", itemSize: 3, kind: "f32" },
      ],
      morphAttributes: [],
      label: "avocado-lod",
    }] },
  }, runtime.telemetry);
  if (runtime.ownCloseable(modelSystem) !== RegistrationStatus.Registered)
    throw new Error("LOD model owner capacity exceeded");
  const handle = modelSystem.adoptCookedModel(asset);
  if (handle === 0) throw new Error("cooked model exceeds the fixed model system");
  const sourceMesh = new THREE.Mesh(new THREE.BufferGeometry(), solidMaterial);
  const lod = modelSystem.createBinding(
    handle, sourceMesh, new Float32Array([0.35, 0.18, 0.08]), 0.1,
  );
  if (!lod) throw new Error("cooked model binding could not resolve its handle");
  sourceMesh.geometry.dispose();
  const meshes = lod.meshes;
  const firstLevel = meshes[0];
  if (!firstLevel) throw new Error("cooked model has no published LOD levels");
  for (const mesh of meshes) root.add(mesh);
  const bounds = firstLevel.geometry.boundingSphere;
  if (!bounds) throw new Error("static LOD source has no bounds");
  const modelScale = 1.5 / bounds.radius;
  for (const mesh of meshes)
    mesh.position.copy(bounds.center).multiplyScalar(-1);
  root.scale.setScalar(modelScale);
  if (runtime.ownDisposable(lod) !== RegistrationStatus.Registered)
    throw new Error("LOD binding owner capacity exceeded");
  const input = new BoundedKeyboardInput();
  if (runtime.ownDisposable(input) !== RegistrationStatus.Registered)
    throw new Error("LOD input owner capacity exceeded");
  const infoElement = document.getElementById("info");
  const hud = new TextHud(infoElement);
  if (loadingElement) loadingElement.style.display = "none";
  if (infoElement) infoElement.style.display = "block";
  let distance = 8;
  // Begin at a deterministic, fully visible presentation distance before
  // traversing the complete LOD chain.
  let elapsed = Math.asin((distance - 24) / 21) / 0.45;
  let wireframe = false;
  const verticalFov = THREE.MathUtils.degToRad(camera.fov);

  /** @alloc-effect diagnostic */
  function updateHud(): void {
    const level = lod.level();
    const triangles = asset.levels[level]?.triangleCount ?? 0;
    hud.setText(
      `afterglow-engine — Offline Static LOD\n` +
        `CC0 Avocado · level ${level} · ${triangles} triangles\n` +
        `Distance ${distance.toFixed(2)} · hysteresis 10%\n` +
        `LOD chain ${asset.levels.map((entry) => entry.triangleCount).join(" → ")}\n` +
        `Errors ${runtime.diagnostics.count}\n\nW wireframe`,
    );
  }
  /** @alloc-effect none */
  function update(frame: Readonly<RenderFrame>): void {
    elapsed += frame.deltaSeconds;
    distance = 24 + Math.sin(elapsed * 0.45) * 21;
    camera.position.z = distance;
    camera.lookAt(0, 0, 0);
    root.rotation.y = Math.PI + elapsed * 0.25;
    lod.select(projectedCoverage(1.5, distance, verticalFov));
    if (input.consumePressed(DemoInputAction.ZoomIn)) {
      wireframe = !wireframe;
      for (let index = 0; index < meshes.length; index++) {
        const mesh = meshes[index];
        if (mesh) mesh.material = wireframe ? wireMaterial : solidMaterial;
      }
    }
    if (frame.frameId % 15 === 0) updateHud(); // @alloc-allowed reason=DiagnosticHud issue=DME-033 expires=2026-10-01
  }

  runtime.enterWarmup();
  runtime.adapter.warmAllDescriptors();
  await runtime.warm();
  rendererHost.renderer.render(scene, camera);
  await rendererHost.renderer.compileAsync(scene, camera);
  for (const mesh of meshes) mesh.material = wireMaterial;
  rendererHost.renderer.render(scene, camera);
  await rendererHost.renderer.compileAsync(scene, camera);
  for (const mesh of meshes) mesh.material = solidMaterial;
  runtime.sealGameplay();
  runtime.start({ update: update });
  console.log("afterglow-engine: canonical offline static LOD demo started");
} catch (error) {
  startupHud.setText(`LOD bootstrap failed: ${String(error)}`);
  try { await runtime.close(); }
  catch (cleanupError) {
    if (error instanceof Error && error.cause === undefined) error.cause = cleanupError;
  }
  throw error;
}

export { runtime as demoRuntime };
