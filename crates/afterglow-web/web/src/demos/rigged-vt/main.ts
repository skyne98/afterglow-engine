import * as THREE from "three/webgpu";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import {
  EngineRuntime,
  RegistrationStatus,
  RendererHost,
  type RenderFrame,
} from "../../engine/index.ts";
import {
  BigAssetSession,
  getVirtualTextureDimensions,
} from "../../engine/assets/index.ts";
import {
  AnimationSet,
  ModelCollectionStatus,
  ModelPrimitives,
  SkeletonDebugAdapter,
  computeDeformedBoundsInto,
  groundDeformedModel,
  normalizeModelPivot,
} from "../../engine/presentation/index.ts";
import {
  FeedbackRegistrationStatus,
  FORMAT_RGBA,
  VirtualGltfBinding,
  VirtualTextureFeedbackCoordinator,
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
} from "../../engine/diagnostics/index.ts";

const FEEDBACK_INTERVAL = 8;
const MODEL_LAYER = 1;
const MODEL_HEIGHT = 2.55;
const MODEL_CAPACITY = 32;

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x11141a);
const camera = new THREE.PerspectiveCamera(
  48,
  innerWidth / innerHeight,
  0.05,
  100,
);
camera.position.set(0, 1.45, 4.1);
camera.lookAt(0, 1.25, 0);
camera.layers.enable(MODEL_LAYER);

scene.add(new THREE.HemisphereLight(0xc8d8ff, 0x231d1a, 2.1));
const keyLight = new THREE.DirectionalLight(0xffead0, 3.5);
keyLight.position.set(3, 5, 4);
keyLight.layers.enable(MODEL_LAYER);
keyLight.castShadow = true;
keyLight.shadow.mapSize.set(2048, 2048);
keyLight.shadow.camera.left = -3.5;
keyLight.shadow.camera.right = 3.5;
keyLight.shadow.camera.top = 3.5;
keyLight.shadow.camera.bottom = -3.5;
keyLight.shadow.camera.near = 0.1;
keyLight.shadow.camera.far = 12;
keyLight.shadow.bias = -0.0004;
keyLight.shadow.normalBias = 0.025;
scene.add(keyLight);
const rimLight = new THREE.DirectionalLight(0x7799ff, 2.2);
rimLight.position.set(-4, 3, -3);
rimLight.layers.enable(MODEL_LAYER);
scene.add(rimLight);
const floor = new THREE.Mesh(
  new THREE.CircleGeometry(3.2, 64),
  new THREE.MeshStandardMaterial({
    color: 0x242830,
    roughness: 0.9,
    metalness: 0,
  }),
);
floor.rotation.x = -Math.PI / 2;
floor.receiveShadow = true;
scene.add(floor);
const grid = new THREE.GridHelper(6, 12, 0x536072, 0x303640);
grid.position.y = 0.002;
scene.add(grid);

const runtime = EngineRuntime.forScene({
  scene,
  entityCapacity: 1,
  memory: {
    frameScratchBytes: 32 * 1024,
    renderScratchBytes: 32 * 1024,
    structuralCommands: 16,
    workerCompletions: 32,
    assetRequests: 64,
    vtRequests: 512,
    telemetryRecords: 8192,
    telemetryMetricCells: 512,
  },
  diagnosticCapacity: 64,
  maxWorkerInputs: 1,
  maxRenderPasses: 2,
});
let feedbackCoordinator: VirtualTextureFeedbackCoordinator | null = null;
function resizeCamera(width: number, height: number): void {
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
  const ratio = Math.min(2, Math.max(0.1, devicePixelRatio));
  feedbackCoordinator?.resize(width * ratio, height * ratio);
}
const rendererHost = await RendererHost.create({
  scene,
  camera,
  diagnostics: runtime.diagnostics,
  parameters: { antialias: false, trackTimestamp: false },
  onResize: resizeCamera,
}).catch((error: unknown) => {
  runtime.dispose();
  throw error;
});
rendererHost.renderer.shadowMap.enabled = true;
rendererHost.renderer.shadowMap.type = THREE.PCFShadowMap;
const bootstrap = new BootstrapGuard(16);
bootstrap.defer(() => rendererHost.dispose());
bootstrap.defer(() => runtime.dispose());
try {
  const device = rendererHost.device;
  const format = device.features.has("texture-compression-bc")
    ? 0
    : device.features.has("texture-compression-astc")
      ? 1
      : FORMAT_RGBA;
  const workerCount = Math.max(
    2,
    Math.min(4, Math.floor((navigator.hardwareConcurrency || 4) / 2)),
  );
  const session = await BigAssetSession.open({
    containerPath: "rigged-vt.big",
    telemetry: runtime.telemetry,
    format,
    workerCount,
    transcodeQueueCapacity: 64,
    maxPendingPages: 16,
    maxPendingBytes: 2 * 1024 * 1024,
    maxHeaderBytes: 2 * 1024 * 1024,
  });
  bootstrap.defer(() => session.close());
  const assetStore = await session.createAssetStore(4, 4);
  async function waitForPackedModel(path: string) {
    const handle = assetStore.loadOptimizedGLTF(path, new GLTFLoader());
    while (handle.state === "loading") {
      assetStore.poll();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    if (handle.state !== "ready")
      throw new Error(
        `${path} failed GLTF parsing or runtime mesh optimization`,
      );
    return handle;
  }
  // The shared-ring mesh optimizer is intentionally one-in-flight. Bootstrap
  // models serially instead of racing calls through its SPSC transport.
  const firstHandle = await waitForPackedModel("model.glb");
  const secondHandle = await waitForPackedModel("model-2.glb");
  function requireReadyAsset<T>(asset: T | undefined): T {
    if (!asset) throw new Error("ready packed model has no asset");
    return asset;
  }
  const firstAsset = requireReadyAsset(firstHandle.asset);
  const secondAsset = requireReadyAsset(secondHandle.asset);
  const store = session.createVirtualTextureStore(device);
  feedbackCoordinator = new VirtualTextureFeedbackCoordinator(
    rendererHost.renderer,
    store,
    { renderables: 2, passes: 8, cadence: FEEDBACK_INTERVAL, scale: 0.125 },
  );
  feedbackCoordinator.resize(
    rendererHost.renderer.domElement.width,
    rendererHost.renderer.domElement.height,
  );

  const firstPivot = new THREE.Group();
  const secondPivot = new THREE.Group();
  firstPivot.add(firstAsset.scene);
  secondPivot.add(secondAsset.scene);
  scene.add(firstPivot, secondPivot);
  const firstPrimitives = new ModelPrimitives(MODEL_CAPACITY);
  const secondPrimitives = new ModelPrimitives(MODEL_CAPACITY);
  if (
    firstPrimitives.collect(firstAsset.scene) !==
      ModelCollectionStatus.Complete ||
    secondPrimitives.collect(secondAsset.scene) !==
      ModelCollectionStatus.Complete
  )
    throw new Error("model primitive capacity exceeded");
  if (firstPrimitives.count === 0 || secondPrimitives.count === 0)
    throw new Error("packed model has no render primitives");
  let firstSkinnedCount = 0,
    secondSkinnedCount = 0;
  function configurePrimitives(
    primitives: ModelPrimitives,
    requireSkinned: boolean,
  ): number {
    let skinned = 0;
    for (let index = 0; index < primitives.count; index++) {
      const mesh = primitives.items[index];
      if (!mesh) continue;
      mesh.layers.set(MODEL_LAYER);
      mesh.castShadow = true;
      mesh.receiveShadow = true;
      if (mesh instanceof THREE.SkinnedMesh) skinned++;
      else if (requireSkinned)
        throw new Error(`model contains non-skinned primitive ${mesh.name}`);
    }
    return skinned;
  }
  firstSkinnedCount = configurePrimitives(firstPrimitives, true);
  secondSkinnedCount = configurePrimitives(secondPrimitives, false);
  if (firstAsset.animations.length === 0 || secondAsset.animations.length === 0)
    throw new Error("packed model has no animation clips");
  normalizeModelPivot(
    firstPivot,
    MODEL_HEIGHT,
    new THREE.Box3(),
    new THREE.Vector3(),
    new THREE.Vector3(),
  );
  normalizeModelPivot(
    secondPivot,
    MODEL_HEIGHT,
    new THREE.Box3(),
    new THREE.Vector3(),
    new THREE.Vector3(),
  );
  const firstAnimations = new AnimationSet(
    firstAsset.scene,
    firstAsset.animations,
    firstAsset.animations.length,
  );
  bootstrap.defer(() => firstAnimations.dispose());
  const secondAnimations = new AnimationSet(
    secondAsset.scene,
    secondAsset.animations,
    secondAsset.animations.length,
  );
  bootstrap.defer(() => secondAnimations.dispose());
  const firstClipIndex = 0;
  let secondClipIndex = secondAsset.animations.findIndex(
    (clip) => clip.name === "Idle",
  );
  if (secondClipIndex < 0) secondClipIndex = 0;
  firstAnimations.play(firstClipIndex);
  secondAnimations.play(secondClipIndex);
  firstAnimations.setTime(0);
  secondAnimations.setTime(0);
  groundDeformedModel(
    firstPivot,
    firstPrimitives,
    new THREE.Box3(),
    new THREE.Vector3(),
  );
  groundDeformedModel(
    secondPivot,
    secondPrimitives,
    new THREE.Box3(),
    new THREE.Vector3(),
  );
  const firstSkeleton = new SkeletonDebugAdapter(scene, firstAsset.scene);
  bootstrap.defer(() => firstSkeleton.dispose());
  const secondSkeleton = new SkeletonDebugAdapter(scene, secondAsset.scene);
  bootstrap.defer(() => secondSkeleton.dispose());

  function resolveImage(modelPath: string, imageIndex: number) {
    const path = `${modelPath}#image-${imageIndex}`;
    let entry = store.getEntry(path);
    if (entry) return entry;
    const dimensions = getVirtualTextureDimensions(session.header, path);
    store.loadTexture(path, { ...dimensions, mipTail: true });
    entry = store.getEntry(path);
    return entry;
  }
  const firstBinding = VirtualGltfBinding.create(firstAsset, store, {
    primitiveCapacity: MODEL_CAPACITY,
    feedbackScene: scene,
    feedbackRoot: firstPivot,
    exclusiveRoots: [
      secondPivot,
      floor,
      grid,
      firstSkeleton.helper,
      secondSkeleton.helper,
    ],
    feedbackCamera: camera,
    feedbackPixelScale: feedbackCoordinator.pixelScale,
    resolveImage: (index) => resolveImage("model.glb", index),
  });
  bootstrap.defer(() => firstBinding.dispose());
  const secondBinding = VirtualGltfBinding.create(secondAsset, store, {
    primitiveCapacity: MODEL_CAPACITY,
    feedbackScene: scene,
    feedbackRoot: secondPivot,
    exclusiveRoots: [
      firstPivot,
      floor,
      grid,
      firstSkeleton.helper,
      secondSkeleton.helper,
    ],
    feedbackCamera: camera,
    feedbackPixelScale: feedbackCoordinator.pixelScale,
    resolveImage: (index) => resolveImage("model-2.glb", index),
  });
  bootstrap.defer(() => secondBinding.dispose());
  if (
    feedbackCoordinator.register(firstBinding) !==
      FeedbackRegistrationStatus.Registered ||
    feedbackCoordinator.register(secondBinding) !==
      FeedbackRegistrationStatus.Registered
  )
    throw new Error("feedback coordinator capacity exceeded");
  if (
    runtime.registerWorker(feedbackCoordinator) !==
      RegistrationStatus.Registered ||
    runtime.registerRenderPass(rendererHost) !==
      RegistrationStatus.Registered ||
    runtime.registerRenderPass(feedbackCoordinator) !==
      RegistrationStatus.Registered
  )
    throw new Error("runtime registration capacity exceeded");

  const firstBaseImage = firstAsset.materialTextures.find(
    (layout) => layout.baseColorImage !== null,
  )?.baseColorImage;
  const secondBaseImage = secondAsset.materialTextures.find(
    (layout) => layout.baseColorImage !== null,
  )?.baseColorImage;
  if (
    firstBaseImage === undefined ||
    firstBaseImage === null ||
    secondBaseImage === undefined ||
    secondBaseImage === null
  )
    throw new Error("packed model has no virtual base color");
  const firstDimensions = getVirtualTextureDimensions(
    session.header,
    `model.glb#image-${firstBaseImage}`,
  );
  const secondDimensions = getVirtualTextureDimensions(
    session.header,
    `model-2.glb#image-${secondBaseImage}`,
  );
  const firstBounds = new THREE.Box3(),
    secondBounds = new THREE.Box3();
  const firstVertex = new THREE.Vector3(),
    secondVertex = new THREE.Vector3();
  function measureFirstBounds(): THREE.Box3 {
    firstPivot.updateMatrixWorld(true);
    return computeDeformedBoundsInto(firstPrimitives, firstBounds, firstVertex);
  }
  function measureSecondBounds(): THREE.Box3 {
    secondPivot.updateMatrixWorld(true);
    return computeDeformedBoundsInto(
      secondPrimitives,
      secondBounds,
      secondVertex,
    );
  }
  const firstGroundedMinY = measureFirstBounds().min.y;
  const secondGroundedMinY = measureSecondBounds().min.y;

  let activeModel = 0;
  let animationEnabled = true;
  let feedbackEnabled = true;
  let skeletonRequested = false;
  let orbitAngle = 0;
  let orbitVelocity = 0;
  let cameraDistance = 4.1;
  let zoomVelocity = 0;
  let smoothedDt = 1 / 60;
  const input = new BoundedKeyboardInput();
  const frameSteps = new FrameStepHarness(32);
  bootstrap.defer(() => input.dispose());
  const errorCapture = new BrowserErrorCapture(runtime.diagnostics);
  bootstrap.defer(() => errorCapture.dispose());
  const hud = new TextHud(document.getElementById("hud"));

  /** @alloc-effect none */
  function setActiveModel(modelNumber: number): void {
    activeModel = modelNumber === 2 ? 1 : 0;
    firstPivot.visible = activeModel === 0;
    secondPivot.visible = activeModel === 1;
    firstSkeleton.setVisible(activeModel === 0 && skeletonRequested);
    secondSkeleton.setVisible(activeModel === 1 && skeletonRequested);
    orbitAngle = 0;
    orbitVelocity = 0;
    cameraDistance = 4.1;
    zoomVelocity = 0;
  }
  /** @alloc-effect none */
  function setAnimationEnabled(enabled: boolean): void {
    animationEnabled = enabled;
    firstAnimations.setEnabled(enabled);
    secondAnimations.setEnabled(enabled);
  }
  /** @alloc-effect none */
  function setSkeletonVisible(visible: boolean): void {
    skeletonRequested = visible;
    firstSkeleton.setVisible(activeModel === 0 && visible);
    secondSkeleton.setVisible(activeModel === 1 && visible);
  }
  /** @alloc-effect none */
  function setFeedbackEnabled(enabled: boolean): void {
    feedbackEnabled = enabled;
    firstBinding.setFeedbackEnabled(enabled);
    secondBinding.setFeedbackEnabled(enabled);
  }
  /** @alloc-effect none */
  function resetView(): void {
    orbitAngle = 0;
    orbitVelocity = 0;
    cameraDistance = 4.1;
    zoomVelocity = 0;
  }

  /** @alloc-effect diagnostic */
  function updateHud(): void {
    const stats = store.getStats();
    const activeAsset = activeModel === 0 ? firstAsset : secondAsset;
    const activeClip =
      activeModel === 0
        ? firstAsset.animations[firstClipIndex]
        : secondAsset.animations[secondClipIndex];
    const activeDimensions =
      activeModel === 0 ? firstDimensions : secondDimensions;
    const activeMeshes =
      activeModel === 0 ? firstPrimitives.count : secondPrimitives.count;
    const activeSkinned =
      activeModel === 0 ? firstSkinnedCount : secondSkinnedCount;
    const optimization = activeAsset.meshOptimization[0];
    hud.setText(
      `afterglow — Model VT · model ${activeModel + 1}/2\n` +
        `Meshes: ${activeMeshes} · ${activeSkinned} skinned\n` +
        `Animation: ${activeClip?.name ?? "none"} · ${(activeClip?.duration ?? 0).toFixed(2)} s · ${animationEnabled ? "playing" : "paused"}\n` +
        `Pipeline: image-free GLB · runtime meshopt ACMR ${(optimization?.originalAcmr ?? 0).toFixed(2)}→${(optimization?.optimizedAcmr ?? 0).toFixed(2)}\n` +
        `Base color: ${activeDimensions.width}×${activeDimensions.height} · atlas: ${store.atlasWidth}² · ${format === 0 ? "BC7" : format === 1 ? "ASTC" : "RGBA"}\n` +
        `Resident: ${stats.atlasSlotsUsed}/${stats.atlasSlotsTotal} · pending: ${stats.pendingPages}\n` +
        `Feedback passes: ${activeModel === 0 ? firstBinding.feedbackPassCount : secondBinding.feedbackPassCount} · FPS: ${(1 / smoothedDt).toFixed(0)} · errors: ${runtime.diagnostics.count}`,
    );
  }

  /** @alloc-effect none */
  function updateFrame(frame: Readonly<RenderFrame>): void {
    const dt = Math.min(0.05, frame.deltaSeconds);
    smoothedDt = smoothedDt * 0.95 + dt * 0.05;
    feedbackCoordinator?.recordFrameTime(dt * 1000);
    if (animationEnabled) {
      firstAnimations.update(dt);
      secondAnimations.update(dt);
    }
    if (input.consumePressed(DemoInputAction.ModelOne)) setActiveModel(1);
    if (input.consumePressed(DemoInputAction.ModelTwo)) setActiveModel(2);
    if (input.consumePressed(DemoInputAction.ToggleAnimation))
      setAnimationEnabled(!animationEnabled);
    if (input.consumePressed(DemoInputAction.ToggleSkeleton))
      setSkeletonVisible(!skeletonRequested);
    if (input.consumePressed(DemoInputAction.ToggleFeedback))
      setFeedbackEnabled(!feedbackEnabled);
    if (input.consumePressed(DemoInputAction.ResetView)) resetView();
    const rotateInput = input.programmatic
      ? 0
      : (input.isDown(DemoInputAction.OrbitRight) ? 1 : 0) -
        (input.isDown(DemoInputAction.OrbitLeft) ? 1 : 0);
    const zoomInput = input.programmatic
      ? 0
      : (input.isDown(DemoInputAction.ZoomOut) ? 1 : 0) -
        (input.isDown(DemoInputAction.ZoomIn) ? 1 : 0);
    orbitVelocity += rotateInput * 7.5 * dt;
    zoomVelocity += zoomInput * 8 * dt;
    const damping = Math.exp(-7 * dt);
    orbitVelocity *= damping;
    zoomVelocity *= damping;
    orbitAngle += orbitVelocity * dt;
    cameraDistance = Math.max(
      1.35,
      Math.min(8, cameraDistance + zoomVelocity * dt),
    );
    if (
      (cameraDistance === 1.35 && zoomVelocity < 0) ||
      (cameraDistance === 8 && zoomVelocity > 0)
    )
      zoomVelocity = 0;
    camera.position.set(
      Math.sin(orbitAngle) * cameraDistance,
      1.45,
      Math.cos(orbitAngle) * cameraDistance,
    );
    camera.lookAt(0, 1.25, 0);
    frameSteps.poll(frame.frameId);
    if (frame.frameId % 15 === 0) updateHud(); // @alloc-allowed reason=DiagnosticHud issue=DME-034 expires=2026-10-01
  }

  runtime.enterWarmup();
  firstPivot.visible = true;
  secondPivot.visible = true;
  firstSkeleton.setVisible(true);
  secondSkeleton.setVisible(true);
  runtime.adapter.warmAllDescriptors();
  await runtime.warm();
  rendererHost.renderer.render(scene, camera);
  firstSkeleton.setVisible(false);
  secondSkeleton.setVisible(false);
  await rendererHost.renderer.compileAsync(scene, camera);
  rendererHost.renderer.render(scene, camera);
  setActiveModel(1);
  rendererHost.renderer.render(scene, camera);
  setActiveModel(2);
  rendererHost.renderer.render(scene, camera);
  setActiveModel(1);
  rendererHost.attachVirtualTextureStore(store);
  runtime.sealGameplay();
  const shutdown = new PageShutdown(() => {
    runtime.stop();
    firstBinding.dispose();
    secondBinding.dispose();
    firstAnimations.dispose();
    secondAnimations.dispose();
    firstSkeleton.dispose();
    secondSkeleton.dispose();
    input.dispose();
    errorCapture.dispose();
    void session.close();
    runtime.dispose();
  });
  bootstrap.defer(() => shutdown.dispose());
  runtime.start({ update: updateFrame });

  function activeBoneCount(): number {
    const primitives = activeModel === 0 ? firstPrimitives : secondPrimitives;
    for (let index = 0; index < primitives.count; index++) {
      const mesh = primitives.items[index];
      if (mesh instanceof THREE.SkinnedMesh) return mesh.skeleton.bones.length;
    }
    return 0;
  }
  function setAnimationTime(seconds: number): void {
    if (activeModel === 0) {
      const duration = firstAsset.animations[firstClipIndex]?.duration ?? 1;
      firstAnimations.setTime(Math.max(0, seconds) % duration);
    } else {
      const duration = secondAsset.animations[secondClipIndex]?.duration ?? 1;
      secondAnimations.setTime(Math.max(0, seconds) % duration);
    }
  }
  function status() {
    const activeAsset = activeModel === 0 ? firstAsset : secondAsset;
    const activePrimitives =
      activeModel === 0 ? firstPrimitives : secondPrimitives;
    const activeDimensions =
      activeModel === 0 ? firstDimensions : secondDimensions;
    const activeClip =
      activeModel === 0
        ? firstAsset.animations[firstClipIndex]
        : secondAsset.animations[secondClipIndex];
    const optimization = activeAsset.meshOptimization[0];
    return {
      activeModel: activeModel + 1,
      meshes: activePrimitives.count,
      skinnedMeshes: activeModel === 0 ? firstSkinnedCount : secondSkinnedCount,
      bones: activeBoneCount(),
      clip: activeClip?.name ?? "",
      clipDuration: activeClip?.duration ?? 0,
      animationEnabled,
      feedbackEnabled,
      skeletonVisible:
        activeModel === 0
          ? firstSkeleton.helper.visible
          : secondSkeleton.helper.visible,
      groundedMinY: activeModel === 0 ? firstGroundedMinY : secondGroundedMinY,
      orbitAngle,
      cameraDistance,
      sourceWidth: activeDimensions.width,
      sourceHeight: activeDimensions.height,
      material: "virtual-gltf-metallic-roughness",
      packedAsset: activeModel === 0 ? "model.glb" : "model-2.glb",
      meshOptimized:
        activeAsset.meshOptimization.length === activePrimitives.count,
      originalAcmr: optimization?.originalAcmr ?? 0,
      optimizedAcmr: optimization?.optimizedAcmr ?? 0,
      preservedAttributes: optimization?.preservedAttributes ?? [],
      sameMeshFeedback: true,
      feedbackChannels:
        activeModel === 0
          ? firstBinding.feedbackPassCount
          : secondBinding.feedbackPassCount,
      shadows:
        rendererHost.renderer.shadowMap.enabled &&
        keyLight.castShadow &&
        floor.receiveShadow,
      shadowMapSize: keyLight.shadow.mapSize.x,
      rendererSealed: rendererHost.sealMonitor.isSealed,
      pipelineViolations: rendererHost.sealMonitor.violations,
    };
  }

  publishDevHarness("__afterglowRiggedVT", {
    setProgrammatic(enabled: boolean) {
      input.programmatic = enabled;
    },
    setAnimationEnabled,
    setAnimationTime,
    measureBounds() {
      const bounds =
        activeModel === 0 ? measureFirstBounds() : measureSecondBounds();
      return { minY: bounds.min.y, maxY: bounds.max.y };
    },
    setActiveModel,
    setFeedbackEnabled,
    setSkeletonVisible,
    setView(angle: number, distance: number) {
      orbitAngle = angle;
      orbitVelocity = 0;
      cameraDistance = Math.max(1.35, Math.min(8, distance));
      zoomVelocity = 0;
    },
    step(count = 1) {
      return frameSteps.wait(runtime.frame.frameId, count);
    },
    telemetry: () => store.getStats(),
    debugSnapshot: () => store.getDebugSnapshot(),
    feedbackMips: () => [],
    errorCount: () => runtime.diagnostics.count,
    errors: () => errorCapture.snapshot(),
    status,
  });
  bootstrap.release();
} catch (error) {
  try {
    await bootstrap.rollback();
  } catch (cleanupError) {
    if (error instanceof Error && error.cause === undefined)
      error.cause = cleanupError;
  }
  throw error;
}
