import * as THREE from "three/webgpu";
import {
  EngineAssets,
  createAssetRangeSource,
  loadResidentTexture,
  readBigHeader,
} from "../../engine/assets/index.ts";
import {
  EngineRuntime,
  GpuProfiler,
  Profiling,
  ProfilingRes,
  RegistrationStatus,
  type RenderFrame,
  type ProfilingFrame,
  type ProfilingHost,
} from "../../engine/index.ts";
import {
  BoundedKeyboardInput,
  DemoInputAction,
  RelativePointerInput,
} from "../../engine/input/index.ts";
import {
  FeedbackRegistrationStatus,
  FORMAT_RGBA,
  SLOT_SIZE,
  VirtualPomSceneBinding,
  VirtualTextureFeedbackCoordinator,
  VirtualTextureTuning,
  validatePomShaderWarmup,
  type VirtualTextureInfo,
} from "../../engine/virtual-texturing/index.ts";
import { TextHud } from "../../engine/diagnostics/index.ts";

const VT_QUALITY_BIAS = 0,
  FEEDBACK_CADENCE_MS = 55;
const POM_MIN_LAYERS = 8,
  POM_MAX_LAYERS = 32,
  POM_HEIGHT_SCALE = 0.05,
  POM_MAX_OFFSET_RATIO = 2,
  POM_MAX_DISTANCE = 0,
  POM_SHADOW_STEPS = 8,
  POM_SHADOW_BIAS = 0.01,
  POM_SHADOW_STRENGTH = 0.82;
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x101318);
scene.fog = new THREE.Fog(0x101318, 7, 28);
const camera = new THREE.PerspectiveCamera(
  70,
  innerWidth / innerHeight,
  0.05,
  60,
);
camera.rotation.order = "YXZ";
let coordinator: VirtualTextureFeedbackCoordinator | null = null;
const runtime = await EngineRuntime.forScene({
  scene,
  camera,
  entityCapacity: 1,
  memory: {
    frameScratchBytes: 16384,
    renderScratchBytes: 16384,
    structuralCommands: 8,
    workerCompletions: 8,
    assetRequests: 8,
    vtRequests: 512,
    telemetryRecords: 65536,
    telemetryMetricCells: 512,
  },
  diagnosticCapacity: 128,
  maxWorkerInputs: 1,
  maxRenderPasses: 2,
  maxOwnedResources: 4,
  renderer: {
    parameters: { antialias: false, trackTimestamp: false },
    onResize: resizeCamera,
  },
});
function resizeCamera(width: number, height: number): void {
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}
const host = runtime.rendererHost;
try {
  const source = createAssetRangeSource("", runtime.telemetry);
  const device = host.device,
    format = device.features.has("texture-compression-bc")
      ? 0
      : device.features.has("texture-compression-astc")
        ? 1
        : FORMAT_RGBA;
  const engineAssets = await EngineAssets.open({
    containerPath: "dungeon.big",
    telemetry: runtime.telemetry,
    format,
    transcodeQueueCapacity: 16,
    urgentBatchDeadlineMs: 1,
    focusBatchDeadlineMs: 16,
    peripheralBatchDeadlineMs: 64,
    maxPendingPages: 16,
    maxPendingBytes: 2 * 1024 * 1024,
    maxHeaderBytes: 2 * 1024 * 1024,
    source,
  });
  if (runtime.ownCloseable(engineAssets) !== RegistrationStatus.Registered)
    throw new Error("Dungeon asset owner capacity exceeded");
  const storageFormat = format === 0
    ? "bc7-rgba-unorm"
    : format === 1
      ? "astc-4x4-unorm"
      : "rgba8unorm";
  const store = engineAssets.createVirtualTextureSystem({
    maxTextures: 16,
    maxMutablePageRefreshesPerPoll: 2,
    device,
    pools: [{
      format: storageFormat,
      capacities: { maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024 },
      tuning: new VirtualTextureTuning({ atlasMaxDimension: 53 * SLOT_SIZE }),
    }],
  });
  coordinator = runtime.createVirtualTextureFeedback(store, {
    renderables: 1,
    passes: 1,
    cadenceMs: FEEDBACK_CADENCE_MS,
    predictionHorizonMs: 100,
    scale: 0.125,
  });
  const activeCoordinator = coordinator;
  // Resident blue-noise dither tile for the POM ray-start jitter. Loaded before
  // the binding so it can be baked into the material options at construction.
  const residentThree = THREE as unknown as Parameters<
    typeof loadResidentTexture
  >[0]; // @unsafe-cast reason=ResidentThreeConstructorVariance issue=DME-030 expires=2026-10-01
  const blueNoiseHeader = await readBigHeader(
    source,
    "blue-noise.big",
    64 * 1024,
  );
  const blueNoiseSource = {
    read: async (offset: number, length: number): Promise<Uint8Array> =>
      source.read("blue-noise.big", offset, length),
  };
  const blueNoise = (
    await loadResidentTexture(residentThree, blueNoiseSource, blueNoiseHeader, "blue-noise")
  ).texture as unknown as THREE.Texture;
  const pomBinding = new VirtualPomSceneBinding({
    camera,
    textures: store,
    feedbackPixelScale: coordinator.pixelScale,
    capacity: 12,
    material: {
      minLayers: POM_MIN_LAYERS,
      maxLayers: POM_MAX_LAYERS,
      heightScale: POM_HEIGHT_SCALE,
      maxOffsetRatio: POM_MAX_OFFSET_RATIO,
      maxDistance: POM_MAX_DISTANCE,
      shadowSteps: POM_SHADOW_STEPS,
      shadowBias: POM_SHADOW_BIAS,
      shadowStrength: POM_SHADOW_STRENGTH,
      qualityBias: VT_QUALITY_BIAS,
      addressMode: 1,
      side: THREE.DoubleSide,
      blueNoiseTexture: blueNoise,
      blueNoiseTile: 64,
    },
  });
  if (runtime.ownDisposable(pomBinding) !== RegistrationStatus.Registered)
    throw new Error("Dungeon POM owner capacity exceeded");
  if (
    coordinator.register(pomBinding) !== FeedbackRegistrationStatus.Registered
  )
    throw new Error("Dungeon feedback capacity exceeded");
  scene.add(new THREE.HemisphereLight(0xb9c8e8, 0x241b15, 1.6));
  const lamp = new THREE.PointLight(0xffc985, 30, 18, 2);
  lamp.position.set(0, 3.2, 0);
  scene.add(lamp);
  const floor = new THREE.Mesh(
    new THREE.PlaneGeometry(16, 16),
    new THREE.MeshStandardMaterial({ color: 0x292722, roughness: 1 }),
  );
  floor.rotation.x = -Math.PI / 2;
  scene.add(floor);
  const ceiling = floor.clone();
  ceiling.position.y = 4;
  ceiling.rotation.x = Math.PI / 2;
  ceiling.material = new THREE.MeshStandardMaterial({
    color: 0x18191b,
    roughness: 1,
  });
  scene.add(ceiling);
  const materialNames = ["Rock064", "Ground103", "PavingStones150"];
  // Resident (non-VT) 8-bit R8 height field, loaded from a v6 `.big` container
  // via the unified resident loader. Height stays out of VT so the POM march
  // pays one direct mip-0 fetch per step; normals/albedo/masks remain
  // VT-streamed. Blue-noise dither tile is loaded above (before the binding).
  const heightSource = {
    read: async (offset: number, length: number): Promise<Uint8Array> =>
      source.read("dungeon-height.big", offset, length),
  };
  const heightHeader = await readBigHeader(
    source,
    "dungeon-height.big",
    2 * 1024 * 1024,
  );
  const heights = await Promise.all(
    materialNames.map((name) =>
      loadResidentTexture(
        residentThree,
        heightSource,
        heightHeader,
        `${name}_Height`,
      ),
    ),
  );
  const registerTexture = (path: string) => {
    const handle = engineAssets.registerVirtualTexture(
      path, storageFormat, "repeat", true,
    );
    if (handle === 0) throw new Error(`virtual texture capacity exceeded: ${path}`);
    return handle;
  };
  const sets = materialNames.map((name) => {
    const set = {
      albedo: registerTexture(`${name}_Color.png`),
      normal: registerTexture(`${name}_NormalGL.png`),
      masks: registerTexture(`${name}_Masks.png`),
    };
    return set;
  });
  const segments: Array<readonly [number, number, number, number]> = [
    [-8, -8, 8, -8],
    [8, -8, 8, 8],
    [8, 8, -8, 8],
    [-8, 8, -8, -8],
    [-3, -8, -3, 1],
    [-3, 1, 2, 1],
    [2, 1, 2, 8],
    [3, -8, 3, -1],
    [-2, -1, 3, -1],
    [-2, -1, -2, 5],
    [-2, 5, 4, 5],
    [4, 5, 4, 8],
  ];
  type Wall = { x1: number; z1: number; x2: number; z2: number };
  const walls: Array<Wall | null> = new Array(12).fill(null);
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i],
      set = sets[i % sets.length],
      heightResult = heights[i % heights.length];
    if (!segment || !set || !heightResult)
      throw new Error("Dungeon material layout incomplete");
    const height = heightResult.texture as unknown as THREE.Texture;
    const [x1, z1, x2, z2] = segment,
      dx = x2 - x1,
      dz = z2 - z1,
      len = Math.hypot(dx, dz),
      geometry = new THREE.PlaneGeometry(len, 4);
    geometry.setAttribute(
      "tangent",
      new THREE.BufferAttribute(
        new Float32Array([1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1]),
        4,
      ),
    );
    const wallUv = geometry.getAttribute("uv");
    for (let u = 0; u < wallUv.count; u++)
      wallUv.setX(u, (wallUv.getX(u) * len) / 4);
    const placeholder = new THREE.MeshStandardMaterial(),
      mesh = new THREE.Mesh(geometry, placeholder);
    mesh.position.set((x1 + x2) / 2, 2, (z1 + z2) / 2);
    mesh.rotation.y = Math.atan2(-dz, dx);
    scene.add(mesh);
    pomBinding.add(mesh, set, height);
    placeholder.dispose();
    walls[i] = { x1, z1, x2, z2 };
  } // @unsafe-cast reason=HeightTextureStructuralType issue=DME-030 expires=2026-10-01
  pomBinding.seal();
  const PLAYER_RADIUS = 0.28,
    pose = { x: -5.5, z: -5.5, yaw: 0, pitch: 0 };
  let pomEnabled = true,
    smoothedDt = 1 / 60;
  const input = new BoundedKeyboardInput();
  if (runtime.ownDisposable(input) !== RegistrationStatus.Registered)
    throw new Error("Dungeon input owner capacity exceeded");
  const relativePointer = new RelativePointerInput(
    host.renderer.domElement,
    (x, y) => {
      pose.yaw -= x * 0.002;
      pose.pitch = Math.max(-1.45, Math.min(1.45, pose.pitch - y * 0.002));
    },
  );
  if (runtime.ownDisposable(relativePointer) !== RegistrationStatus.Registered)
    throw new Error("Dungeon pointer owner capacity exceeded");
  const hud = new TextHud(document.getElementById("hud"));
  const timing = {
    vtCpuUs: 0,
    renderSubmitUs: 0,
    feedbackSubmitUs: 0,
    frameCpuUs: 0,
    gpuTimingValid: false,
    resolvedFrameId: -1,
    gpuSceneMs: 0,
    gpuOutputMs: 0,
    gpuFeedbackMs: 0,
    gpuTotalMs: 0,
    gpuTimestampSupported: host.timestampSupported,
  };
  // Central profiling ECS resource: gathers renderer.info counts + Three GPU
  // pass timings into a bounded ring. Set on the world so any system can read it.
  const profiling = new Profiling(
    { renderer: host.renderer, deltaSource: () => smoothedDt * 1000 } as unknown as ProfilingHost,
    { capacity: 240 },
  );
  ProfilingRes.set(runtime.adapter.world, profiling);
  /** @alloc-effect none */ function pointDistance(
    x: number,
    z: number,
    w: Wall,
  ): number {
    const dx = w.x2 - w.x1,
      dz = w.z2 - w.z1,
      l2 = dx * dx + dz * dz,
      t = Math.max(0, Math.min(1, ((x - w.x1) * dx + (z - w.z1) * dz) / l2));
    return Math.hypot(x - (w.x1 + t * dx), z - (w.z1 + t * dz));
  }
  /** @alloc-effect none */ function valid(x: number, z: number): boolean {
    if (!(x > -7.7 && x < 7.7 && z > -7.7 && z < 7.7)) return false;
    for (let i = 4; i < walls.length; i++) {
      const wall = walls[i];
      if (wall && pointDistance(x, z, wall) <= PLAYER_RADIUS) return false;
    }
    return true;
  }
  /** @alloc-effect none */ function setPose(
    x: number,
    z: number,
    yaw = pose.yaw,
    pitch = pose.pitch,
  ): void {
    if (valid(x, z)) {
      pose.x = x;
      pose.z = z;
    }
    pose.yaw = yaw;
    pose.pitch = Math.max(-1.45, Math.min(1.45, pitch));
  }
  /** @alloc-effect none */ function move(
    forward: number,
    strafe: number,
  ): void {
    const sin = Math.sin(pose.yaw),
      cos = Math.cos(pose.yaw),
      dx = -sin * forward + cos * strafe,
      dz = -cos * forward - sin * strafe;
    if (valid(pose.x + dx, pose.z)) pose.x += dx;
    if (valid(pose.x, pose.z + dz)) pose.z += dz;
  }
  /** @alloc-effect none */ function setPomEnabled(enabled: boolean): void {
    pomEnabled = enabled;
    pomBinding.setPomEnabled(enabled);
  }
  /** @alloc-effect diagnostic */ function updateHud(): void {
    const d = store.getStats(),
      status = relativePointer.getStatus();
    hud.setText(
      `afterglow — Engine Dungeon\n3 × 8K PBR sets · 12 walls · ${Math.round(1 / smoothedDt)} FPS\nPosition ${pose.x.toFixed(2)}, ${pose.z.toFixed(2)} · ${status.eventType}\nPOM ${pomEnabled ? "8–32 layers · 8-step self-shadow" : "off"}\nResident ${d.atlasSlotsUsed}/${d.atlasSlotsTotal} · pending ${d.pendingPages}\nErrors ${runtime.diagnostics.count}`,
    );
  }
  /** @alloc-effect none */ function updateFrame(
    frame: Readonly<RenderFrame>,
  ): void {
    const frameStarted = performance.now(),
      dt = Math.min(0.05, frame.deltaSeconds);
    smoothedDt = smoothedDt * 0.95 + dt * 0.05;
    coordinator?.recordFrameTime(dt * 1000);
    {
      let f =
          (input.isDown(DemoInputAction.ZoomIn) ? 1 : 0) -
          (input.isDown(DemoInputAction.ZoomOut) ? 1 : 0),
        s =
          (input.isDown(DemoInputAction.OrbitRight) ? 1 : 0) -
          (input.isDown(DemoInputAction.OrbitLeft) ? 1 : 0),
        speed = input.isDown(DemoInputAction.Sprint) ? 5.5 : 2.8;
      if (f || s) {
        const n = Math.hypot(f, s);
        move((f / n) * speed * dt, (s / n) * speed * dt);
      }
      if (input.consumePressed(DemoInputAction.ResetView))
        setPose(-5.5, -5.5, 0, 0);
      if (input.consumePressed(DemoInputAction.PixelView))
        setPomEnabled(!pomEnabled);
      if (input.consumePressed(DemoInputAction.ModelOne))
        setPose(-5.5, -5.5, 0, 0);
      if (input.consumePressed(DemoInputAction.ModelTwo))
        setPose(5.5, -5.5, Math.PI, 0);
      if (input.consumePressed(DemoInputAction.PoseThree))
        setPose(5.5, 6.5, -Math.PI / 2, 0);
    }
    camera.position.set(pose.x, 1.7, pose.z);
    camera.rotation.set(pose.pitch, pose.yaw, 0);
    camera.updateMatrixWorld();
    lamp.position.set(pose.x, 3.1, pose.z);
    timing.vtCpuUs = coordinator?.vtCpuUs ?? 0;
    timing.renderSubmitUs = host.renderSubmitUs;
    timing.feedbackSubmitUs = coordinator?.feedbackSubmitUs ?? 0;
    timing.frameCpuUs = (performance.now() - frameStarted) * 1000;
    // Gather profiling (renderer.info + GPU pass timings) off the hot path.
    // Fire-and-forget: info snapshot is immediate; GPU readback resolves later.
    if (frame.frameId % 15 === 0) void profiling.gather(frame.frameId); // @alloc-allowed reason=DiagnosticGpuReadback issue=DME-034 expires=2026-10-01
    if (frame.frameId % 15 === 0) updateHud(); // @alloc-allowed reason=DiagnosticHud issue=DME-034 expires=2026-10-01
  } // @alloc-allowed reason=DiagnosticHud issue=DME-034 expires=2026-10-01
  runtime.enterWarmup();
  profiling.setEnabled(true);
  await validatePomShaderWarmup(host, async () => {
    runtime.adapter.warmAllDescriptors();
    pomBinding.setPomEnabled(false);
    await host.warm();
    await activeCoordinator.warm();
    pomBinding.setPomEnabled(true);
    await host.warm();
    await activeCoordinator.warm();
    await runtime.warm();
  });
  host.renderer.render(scene, camera);
  // R8unorm height is universally filterable; no float32-filterable feature
  // gate or post-warm-up format assertion is required (unlike the former
  // r32float-from-r16 path).
  runtime.sealGameplay();
  runtime.start({ update: updateFrame });
  console.log("afterglow-engine: canonical Dungeon started");
} catch (error) {
  try { await runtime.close(); }
  catch (cleanup) {
    if (error instanceof Error && error.cause === undefined) error.cause = cleanup;
  }
  throw error;
}

export { runtime as demoRuntime };
