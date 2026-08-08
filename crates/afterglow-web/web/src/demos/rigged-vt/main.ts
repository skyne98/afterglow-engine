import * as THREE from "three/webgpu";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import {
  EngineRuntime,
  RegistrationStatus,
  type RenderFrame,
} from "../../engine/index.ts";
import { EngineAssets } from "../../engine/assets/index.ts";
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
  type VirtualTextureHandle,
  type VirtualTextureInfo,
} from "../../engine/virtual-texturing/index.ts";
import {
  BoundedKeyboardInput,
  DemoInputAction,
} from "../../engine/input/index.ts";
import { TextHud } from "../../engine/diagnostics/index.ts";

const FEEDBACK_CADENCE_MS = 55;
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

let feedbackCoordinator: VirtualTextureFeedbackCoordinator | null = null;
const runtime = await EngineRuntime.forScene({
  scene,
  camera,
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
  maxOwnedResources: 8,
  renderer: {
    parameters: { antialias: false, trackTimestamp: false },
    onResize: resizeCamera,
  },
});
function resizeCamera(width: number, height: number): void {
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}
const rendererHost = runtime.rendererHost;
rendererHost.renderer.shadowMap.enabled = true;
rendererHost.renderer.shadowMap.type = THREE.PCFShadowMap;
try {
  const device = rendererHost.device;
  const format = device.features.has("texture-compression-bc")
    ? 0
    : device.features.has("texture-compression-astc")
      ? 1
      : FORMAT_RGBA;
  const engineAssets = await EngineAssets.open({
    containerPath: "rigged-vt.big",
    telemetry: runtime.telemetry,
    format,
    transcodeQueueCapacity: 16,
    urgentBatchDeadlineMs: 1,
    focusBatchDeadlineMs: 16,
    peripheralBatchDeadlineMs: 64,
    maxPendingPages: 16,
    maxPendingBytes: 2 * 1024 * 1024,
    maxHeaderBytes: 2 * 1024 * 1024,
  });
  if (runtime.ownCloseable(engineAssets) !== RegistrationStatus.Registered)
    throw new Error("rigged VT asset owner capacity exceeded");
  const assetStore = await engineAssets.createAssetStore(4, 4);
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
  const storageFormat = format === 0
    ? "bc7-rgba-unorm"
    : format === 1
      ? "astc-4x4-unorm"
      : "rgba8unorm";
  const store = engineAssets.createVirtualTextureSystem({
    maxTextures: 64,
    maxMutablePageRefreshesPerPoll: 2,
    device,
    pools: [{
      format: storageFormat,
      capacities: { maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024 },
    }],
  });
  feedbackCoordinator = runtime.createVirtualTextureFeedback(store, {
    renderables: 2, passes: 8, cadenceMs: FEEDBACK_CADENCE_MS,
    predictionHorizonMs: 100, scale: 0.125,
  });

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
  if (runtime.ownDisposable(firstAnimations) !== RegistrationStatus.Registered)
    throw new Error("first animation owner capacity exceeded");
  const secondAnimations = new AnimationSet(
    secondAsset.scene,
    secondAsset.animations,
    secondAsset.animations.length,
  );
  if (runtime.ownDisposable(secondAnimations) !== RegistrationStatus.Registered)
    throw new Error("second animation owner capacity exceeded");
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
  if (runtime.ownDisposable(firstSkeleton) !== RegistrationStatus.Registered)
    throw new Error("first skeleton owner capacity exceeded");
  const secondSkeleton = new SkeletonDebugAdapter(scene, secondAsset.scene);
  if (runtime.ownDisposable(secondSkeleton) !== RegistrationStatus.Registered)
    throw new Error("second skeleton owner capacity exceeded");

  const imageEntries = new Map<string, VirtualTextureHandle>();
  function resolveImage(modelPath: string, imageIndex: number) {
    const path = `${modelPath}#image-${imageIndex}`;
    const existing = imageEntries.get(path);
    if (existing) return existing;
    const handle = engineAssets.registerVirtualTexture(
      path, storageFormat, "repeat", true,
    );
    if (handle === 0) throw new Error(`virtual texture capacity exceeded: ${path}`);
    imageEntries.set(path, handle);
    return handle;
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
  if (runtime.ownDisposable(firstBinding) !== RegistrationStatus.Registered)
    throw new Error("first virtual model binding owner capacity exceeded");
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
  if (runtime.ownDisposable(secondBinding) !== RegistrationStatus.Registered)
    throw new Error("second virtual model binding owner capacity exceeded");
  if (
    feedbackCoordinator.register(firstBinding) !==
      FeedbackRegistrationStatus.Registered ||
    feedbackCoordinator.register(secondBinding) !==
      FeedbackRegistrationStatus.Registered
  )
    throw new Error("feedback coordinator capacity exceeded");
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
  const firstBaseTexture = resolveImage("model.glb", firstBaseImage);
  const secondBaseTexture = resolveImage("model-2.glb", secondBaseImage);
  const firstInfo: VirtualTextureInfo = {
    texture: 0, textureId: 0, sourceKey: '', width: 0, height: 0,
    pageGridX: 0, pageGridY: 0,
  };
  const secondInfo: VirtualTextureInfo = { ...firstInfo };
  if (!store.readTextureInfo(firstBaseTexture, firstInfo) ||
      !store.readTextureInfo(secondBaseTexture, secondInfo))
    throw new Error("packed model base texture handle is stale");
  const firstDimensions = { width: firstInfo.width, height: firstInfo.height };
  const secondDimensions = { width: secondInfo.width, height: secondInfo.height };
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
  if (runtime.ownDisposable(input) !== RegistrationStatus.Registered)
    throw new Error("rigged VT input owner capacity exceeded");
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
  runtime.sealGameplay();
  runtime.start({ update: updateFrame });

} catch (error) {
  try { await runtime.close(); }
  catch (cleanupError) {
    if (error instanceof Error && error.cause === undefined) error.cause = cleanupError;
  }
  throw error;
}

export { runtime as demoRuntime };
