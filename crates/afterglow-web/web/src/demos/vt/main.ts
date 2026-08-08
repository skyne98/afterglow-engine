import * as THREE from "three/webgpu";
import {
  EngineRuntime,
  RegistrationStatus,
  type RenderFrame,
} from "../../engine/index.ts";
import {
  FeedbackRegistrationStatus,
  VirtualMaterialBinding,
  VirtualTextureFeedbackCoordinator,
  VirtualTextureSystem,
  generateTerrainPage,
} from "../../engine/virtual-texturing/index.ts";
import {
  BoundedKeyboardInput,
  DemoInputAction,
} from "../../engine/input/index.ts";
import { TextHud } from "../../engine/diagnostics/index.ts";

const VIRTUAL_SIZE = 262_144;
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0a0c10);
scene.add(new THREE.AmbientLight(0x8090a0, 1.5));
const camera = new THREE.OrthographicCamera(-7, 7, 5, -5, 0.1, 30);
camera.position.z = 14;
let coordinator: VirtualTextureFeedbackCoordinator | null = null;
const runtime = await EngineRuntime.forScene({
  scene,
  camera,
  entityCapacity: 1,
  memory: {
    frameScratchBytes: 16 * 1024,
    renderScratchBytes: 16 * 1024,
    structuralCommands: 8,
    workerCompletions: 8,
    assetRequests: 8,
    vtRequests: 512,
    telemetryRecords: 8192,
    telemetryMetricCells: 512,
  },
  diagnosticCapacity: 64,
  maxWorkerInputs: 1,
  maxRenderPasses: 2,
  maxOwnedResources: 3,
  renderer: {
    parameters: { antialias: true },
    onResize: resizeCamera,
  },
});
function resizeCamera(width: number, height: number): void {
  const aspect = width / height;
  camera.left = -5 * aspect;
  camera.right = 5 * aspect;
  camera.top = 5;
  camera.bottom = -5;
  camera.updateProjectionMatrix();
}
const rendererHost = runtime.rendererHost;
try {
  const textures = new VirtualTextureSystem({
    maxTextures: 1,
    maxMutablePageRefreshesPerPoll: 1,
    device: rendererHost.device,
    pools: [{
      format: "rgba8unorm-srgb",
      capacities: { maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024 },
    }],
  });
  if (runtime.ownDisposable(textures) !== RegistrationStatus.Registered)
    throw new Error("procedural VT owner capacity exceeded");
  const texture = textures.createTexture({
    width: VIRTUAL_SIZE,
    height: VIRTUAL_SIZE,
    format: "rgba8unorm-srgb",
    addressMode: "clamp",
    label: "procedural terrain",
  }, async (_path, request) =>
    generateTerrainPage(request.mip, request.x, request.y, VIRTUAL_SIZE));
  if (texture === 0) throw new Error("procedural VT registration failed");
  coordinator = runtime.createVirtualTextureFeedback(textures, {
    renderables: 1, passes: 1, cadenceMs: 55,
    predictionHorizonMs: 100, scale: 0.125,
  });
  const quad = new THREE.Mesh(
    new THREE.PlaneGeometry(12, 10),
    new THREE.MeshStandardMaterial({ roughness: 0.9, metalness: 0 }),
  );
  scene.add(quad);
  const binding = new VirtualMaterialBinding({
    scene,
    camera,
    root: quad,
    mesh: quad,
    textures,
    set: { albedo: texture },
    feedbackPixelScale: coordinator.pixelScale,
    material: { roughnessFactor: 0.9, metalnessFactor: 0 },
  });
  if (runtime.ownDisposable(binding) !== RegistrationStatus.Registered)
    throw new Error("procedural VT binding owner capacity exceeded");
  if (coordinator.register(binding) !== FeedbackRegistrationStatus.Registered)
    throw new Error("procedural VT feedback capacity exceeded");

  let camX = 0.5,
    camY = 0.5,
    zoom = 4;
  const input = new BoundedKeyboardInput();
  if (runtime.ownDisposable(input) !== RegistrationStatus.Registered)
    throw new Error("procedural VT input owner capacity exceeded");
  const hud = new TextHud(document.getElementById("info"));
  const uv = quad.geometry.getAttribute("uv");
  /** @alloc-effect none */
  function updateUv(): void {
    const half = 0.5 / zoom;
    uv.setXY(0, camX - half, camY + half);
    uv.setXY(1, camX + half, camY + half);
    uv.setXY(2, camX - half, camY - half);
    uv.setXY(3, camX + half, camY - half);
    uv.needsUpdate = true;
  }
  /** @alloc-effect diagnostic */
  function updateHud(): void {
    const stats = textures.getStats();
    hud.setText(
      `afterglow — Engine Virtual Texture\n` +
        `262,144² terrain · 256 GiB logical RGBA\n` +
        `VirtualTextureSystem · ${textures.atlasWidth}² shared atlas\n` +
        `UV ${camX.toFixed(3)}, ${camY.toFixed(3)} · zoom ${zoom.toFixed(1)}×\n` +
        `Resident ${stats.atlasSlotsUsed}/${stats.atlasSlotsTotal} · pending ${stats.pendingPages}\n` +
        `Errors ${runtime.diagnostics.count}\n\nWASD pan · wheel zoom · O overview · P pixel`,
    );
  }
  /** @alloc-effect none */
  function updateFrame(frame: Readonly<RenderFrame>): void {
    const dt = Math.min(0.05, frame.deltaSeconds);
    coordinator?.recordFrameTime(dt * 1000);
    if (!input.programmatic) {
      const speed = (1.4 / zoom) * dt;
      camX = Math.max(
        0,
        Math.min(
          1,
          camX +
            ((input.isDown(DemoInputAction.OrbitRight) ? 1 : 0) -
              (input.isDown(DemoInputAction.OrbitLeft) ? 1 : 0)) *
              speed,
        ),
      );
      camY = Math.max(
        0,
        Math.min(
          1,
          camY +
            ((input.isDown(DemoInputAction.ZoomIn) ? 1 : 0) -
              (input.isDown(DemoInputAction.ZoomOut) ? 1 : 0)) *
              speed,
        ),
      );
      const wheel = input.consumeWheelDelta();
      if (wheel !== 0)
        zoom = Math.max(
          0.5,
          Math.min(VIRTUAL_SIZE, zoom * Math.exp(wheel * 0.001)),
        );
      if (input.consumePressed(DemoInputAction.Overview)) zoom = 0.5;
      if (input.consumePressed(DemoInputAction.PixelView)) zoom = VIRTUAL_SIZE;
    }
    updateUv();
    if (frame.frameId % 15 === 0) updateHud(); // @alloc-allowed reason=DiagnosticHud issue=DME-034 expires=2026-10-01
  }

  runtime.enterWarmup();
  runtime.adapter.warmAllDescriptors();
  await runtime.warm();
  updateUv();
  rendererHost.renderer.render(scene, camera);
  runtime.sealGameplay();
  runtime.start({ update: updateFrame });

  console.log("afterglow-engine: canonical engine-backed VT demo started");
} catch (error) {
  try { await runtime.close(); }
  catch (cleanupError) {
    if (error instanceof Error && error.cause === undefined) error.cause = cleanupError;
  }
  throw error;
}

export { runtime as demoRuntime };
