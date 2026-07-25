import * as THREE from "three/webgpu";
import {
  EngineRuntime,
  RegistrationStatus,
  RendererHost,
  type RenderFrame,
} from "../../engine/index.ts";
import {
  FeedbackRegistrationStatus,
  SLOT_SIZE,
  VirtualMaterialBinding,
  VirtualTextureFeedbackCoordinator,
  createProceduralVirtualTextureStore,
  generateTerrainPage,
} from "../../engine/virtual-texturing/index.ts";
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
  testRawFeedback,
  testUploadLocations,
} from "../../engine/diagnostics/index.ts";

const VIRTUAL_SIZE = 262_144;
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0a0c10);
scene.add(new THREE.AmbientLight(0x8090a0, 1.5));
const camera = new THREE.OrthographicCamera(-7, 7, 5, -5, 0.1, 30);
camera.position.z = 14;
const runtime = EngineRuntime.forScene({
  scene,
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
});
let coordinator: VirtualTextureFeedbackCoordinator | null = null;
function resizeCamera(width: number, height: number): void {
  const aspect = width / height;
  camera.left = -5 * aspect;
  camera.right = 5 * aspect;
  camera.top = 5;
  camera.bottom = -5;
  camera.updateProjectionMatrix();
  const ratio = Math.min(2, Math.max(0.1, devicePixelRatio));
  coordinator?.resize(width * ratio, height * ratio);
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
const bootstrap = new BootstrapGuard(12);
bootstrap.defer(() => rendererHost.dispose());
bootstrap.defer(() => runtime.dispose());
try {
  const path = "procedural://terrain";
  const store = createProceduralVirtualTextureStore(
    async (_path, request) =>
      generateTerrainPage(request.mip, request.x, request.y, VIRTUAL_SIZE),
    rendererHost.device,
    { maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024 },
  );
  bootstrap.defer(() => store.dispose());
  store.loadTexture(path, { width: VIRTUAL_SIZE, height: VIRTUAL_SIZE });
  const entry = store.getEntry(path);
  if (!entry) throw new Error("procedural VT registration failed");
  coordinator = new VirtualTextureFeedbackCoordinator(
    rendererHost.renderer,
    store,
    { renderables: 1, passes: 1, cadenceMs: 55, scale: 0.125 },
  );
  coordinator.resize(
    rendererHost.renderer.domElement.width,
    rendererHost.renderer.domElement.height,
  );
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
    store,
    set: { albedo: entry },
    feedbackPixelScale: coordinator.pixelScale,
    material: { roughnessFactor: 0.9, metalnessFactor: 0 },
  });
  bootstrap.defer(() => binding.dispose());
  if (
    coordinator.register(binding) !== FeedbackRegistrationStatus.Registered ||
    runtime.registerWorker(coordinator) !== RegistrationStatus.Registered ||
    runtime.registerRenderPass(rendererHost) !==
      RegistrationStatus.Registered ||
    runtime.registerRenderPass(coordinator) !== RegistrationStatus.Registered
  )
    throw new Error("procedural VT runtime capacity exceeded");

  let camX = 0.5,
    camY = 0.5,
    zoom = 4;
  const input = new BoundedKeyboardInput();
  bootstrap.defer(() => input.dispose());
  const frameSteps = new FrameStepHarness(32);
  const errors = new BrowserErrorCapture(runtime.diagnostics);
  bootstrap.defer(() => errors.dispose());
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
    const stats = store.getStats();
    hud.setText(
      `afterglow — Engine Virtual Texture\n` +
        `262,144² terrain · 256 GiB logical RGBA\n` +
        `VirtualTextureStore · ${store.atlasWidth}² shared atlas\n` +
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
    frameSteps.poll(frame.frameId);
    if (frame.frameId % 15 === 0) updateHud(); // @alloc-allowed reason=DiagnosticHud issue=DME-034 expires=2026-10-01
  }

  runtime.enterWarmup();
  runtime.adapter.warmAllDescriptors();
  await runtime.warm();
  updateUv();
  rendererHost.renderer.render(scene, camera);
  rendererHost.attachVirtualTextureStore(store);
  runtime.sealGameplay();
  const shutdown = new PageShutdown(() => {
    runtime.stop();
    binding.dispose();
    input.dispose();
    errors.dispose();
    store.dispose();
    runtime.dispose();
  });
  bootstrap.defer(() => shutdown.dispose());
  runtime.start({ update: updateFrame });

  function step(count = 1): Promise<void> {
    return frameSteps.wait(runtime.frame.frameId, Math.max(1, count | 0));
  }
  publishDevHarness("__afterglowVtGpuTest", {
    snapshot: () => ({
      gpuReady: Boolean(store.gpuAtlasTexture),
      resident: store.getDebugSnapshot().atlasSlotsUsed,
      loaded: store.getDebugSnapshot().atlasSlotsUsed,
      errors: errors.snapshot(),
    }),
    setCamera(x: number, y: number, nextZoom: number) {
      input.programmatic = true;
      input.clear();
      camX = Math.max(0, Math.min(1, x));
      camY = Math.max(0, Math.min(1, y));
      zoom = Math.max(0.5, Math.min(VIRTUAL_SIZE, nextZoom));
    },
    async run() {
      input.programmatic = true;
      for (let index = 0; index < 90; index++) await step(1);
      if (!store.gpuAtlasTexture) throw new Error("engine atlas not attached");
      const feedbackRuns = [];
      for (const direction of ["east", "west", "rotated"])
        feedbackRuns.push(
          await testRawFeedback(rendererHost.device, direction),
        );
      const residencyRuns = [];
      const scenarios = [
        {
          name: "eastbound",
          points: [
            { x: 0.08, y: 0.25, z: 8 },
            { x: 0.5, y: 0.25, z: 16 },
            { x: 0.92, y: 0.25, z: 32 },
          ],
        },
        {
          name: "westbound",
          points: [
            { x: 0.92, y: 0.75, z: 32 },
            { x: 0.5, y: 0.75, z: 8 },
            { x: 0.08, y: 0.75, z: 2 },
          ],
        },
        {
          name: "diagonal-lod",
          points: [
            { x: 0.08, y: 0.08, z: 0.5 },
            { x: 0.5, y: 0.5, z: 64 },
            { x: 0.92, y: 0.92, z: VIRTUAL_SIZE },
          ],
        },
      ];
      for (const scenario of scenarios) {
        const before = store.getDebugSnapshot().atlasSlotsUsed;
        const checkpoints = [];
        for (const point of scenario.points) {
          camX = point.x;
          camY = point.y;
          zoom = point.z;
          await step(35);
          checkpoints.push({
            ...point,
            pages: store.getDebugSnapshot().atlasSlotsUsed,
          });
        }
        residencyRuns.push({
          name: scenario.name,
          before,
          after: store.getDebugSnapshot().atlasSlotsUsed,
          checkpoints,
        });
      }
      const uploads = await testUploadLocations(
        rendererHost.device,
        store.atlasWidth,
        store.atlasHeight,
        SLOT_SIZE,
      );
      await rendererHost.device.queue.onSubmittedWorkDone();
      if (runtime.diagnostics.count !== 0)
        throw new Error("VT GPU diagnostics are not empty");
      return {
        ok: true,
        feedbackRuns,
        uploads,
        residencyRuns,
        resident: store.getDebugSnapshot().atlasSlotsUsed,
        virtualSize: VIRTUAL_SIZE,
        atlas: [store.atlasWidth, store.atlasHeight],
      };
    },
  });
  bootstrap.release();
  console.log("afterglow-engine: canonical engine-backed VT demo started");
} catch (error) {
  try {
    await bootstrap.rollback();
  } catch (cleanupError) {
    if (error instanceof Error && error.cause === undefined)
      error.cause = cleanupError;
  }
  throw error;
}
