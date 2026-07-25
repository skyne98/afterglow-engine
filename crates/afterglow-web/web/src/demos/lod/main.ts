import * as THREE from "three/webgpu";
import {
  EngineRuntime,
  RegistrationStatus,
  RendererHost,
  type RenderFrame,
} from "../../engine/index.ts";
import {
  LodSet,
  loadStaticMesh,
  projectedCoverage,
} from "../../engine/presentation/index.ts";
import {
  BoundedKeyboardInput,
  DemoInputAction,
} from "../../engine/input/index.ts";
import {
  BootstrapGuard,
  BrowserErrorCapture,
  FrameStepHarness,
  PageShutdown,
  TextHud,
  publishDevHarness,
} from "../../engine/diagnostics/index.ts";

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
const runtime = EngineRuntime.forScene({
  scene,
  entityCapacity: 1,
  memory: {
    frameScratchBytes: 8 * 1024,
    renderScratchBytes: 8 * 1024,
    structuralCommands: 4,
    workerCompletions: 4,
    assetRequests: 4,
    vtRequests: 4,
  },
  diagnosticCapacity: 32,
  maxWorkerInputs: 0,
  maxRenderPasses: 1,
});
function resizeCamera(width: number, height: number): void {
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}
const rendererHost = await RendererHost.create({
  scene,
  camera,
  diagnostics: runtime.diagnostics,
  parameters: { antialias: true },
  onResize: resizeCamera,
}).catch((error: unknown) => {
  runtime.dispose();
  throw error;
});
const loadingElement = document.getElementById("loading");
const startupHud = new TextHud(loadingElement);
const bootstrap = new BootstrapGuard(10);
bootstrap.defer(() => rendererHost.dispose());
bootstrap.defer(() => runtime.dispose());
try {
  const asset = await loadStaticMesh({
    containerPath: "lod-demo.big",
    assetName: "Avocado",
    maxHeaderBytes: 64 * 1024,
  });
  bootstrap.defer(() => asset.dispose());
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
  bootstrap.defer(() => solidMaterial.dispose());
  bootstrap.defer(() => wireMaterial.dispose());
  const meshes: THREE.Mesh[] = [];
  for (const level of asset.levels) {
    const mesh = new THREE.Mesh(level.geometry, solidMaterial);
    root.add(mesh);
    meshes.push(mesh);
  }
  const bounds = asset.levels[0]?.geometry.boundingSphere;
  if (!bounds) throw new Error("static LOD source has no bounds");
  const modelScale = 1.5 / bounds.radius;
  for (const mesh of meshes)
    mesh.position.copy(bounds.center).multiplyScalar(-1);
  root.scale.setScalar(modelScale);
  const lod = new LodSet(meshes, [0.35, 0.18, 0.08], 0.1, 4);
  const input = new BoundedKeyboardInput();
  bootstrap.defer(() => input.dispose());
  const errors = new BrowserErrorCapture(runtime.diagnostics);
  bootstrap.defer(() => errors.dispose());
  const frameSteps = new FrameStepHarness(24);
  const infoElement = document.getElementById("info");
  const hud = new TextHud(infoElement);
  if (loadingElement) loadingElement.style.display = "none";
  if (infoElement) infoElement.style.display = "block";
  let programmatic = false;
  let distance = 8;
  let elapsed = 0;
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
    if (!programmatic) distance = 24 + Math.sin(elapsed * 0.45) * 21;
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
    frameSteps.poll(frame.frameId);
    if (frame.frameId % 15 === 0) updateHud(); // @alloc-allowed reason=DiagnosticHud issue=DME-033 expires=2026-10-01
  }

  if (
    runtime.registerRenderPass(rendererHost) !== RegistrationStatus.Registered
  )
    throw new Error("LOD render-pass capacity exceeded");
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
  const shutdown = new PageShutdown(() => {
    runtime.stop();
    input.dispose();
    errors.dispose();
    asset.dispose();
    runtime.dispose();
  });
  bootstrap.defer(() => shutdown.dispose());
  runtime.start({ update: update });
  function step(count = 1): Promise<void> {
    return frameSteps.wait(runtime.frame.frameId, Math.max(1, count | 0));
  }
  publishDevHarness("__afterglowLod", {
    snapshot: () => ({
      level: lod.level(),
      distance,
      triangles: asset.levels[lod.level()]?.triangleCount ?? 0,
      visible: meshes.reduce(
        (count, mesh) => count + (mesh.visible ? 1 : 0),
        0,
      ),
      errors: errors.snapshot(),
    }),
    setDistance(value: number) {
      programmatic = true;
      distance = Math.max(2, Math.min(80, value));
    },
    step,
    async run() {
      programmatic = true;
      const distances = [5, 12, 24, 50, 24, 12, 5];
      const levels: number[] = [];
      for (const value of distances) {
        distance = value;
        await step(3);
        levels.push(lod.level());
        if (
          meshes.reduce((count, mesh) => count + (mesh.visible ? 1 : 0), 0) !==
          1
        )
          throw new Error("LOD visibility invariant failed");
      }
      if (runtime.diagnostics.count !== 0)
        throw new Error("LOD runtime diagnostics are not empty");
      return {
        ok: true,
        distances,
        levels,
        triangles: asset.levels.map((entry) => entry.triangleCount),
      };
    },
  });
  bootstrap.release();
  console.log("afterglow-engine: canonical offline static LOD demo started");
} catch (error) {
  startupHud.setText(`LOD bootstrap failed: ${String(error)}`);
  try {
    await bootstrap.rollback();
  } catch (cleanupError) {
    if (error instanceof Error && error.cause === undefined)
      error.cause = cleanupError;
  }
  throw error;
}
