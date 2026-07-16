import { MeshoptClient } from './meshopt.client.ts';
import { TextureClient } from './texture.client.ts';
import { Rpc } from './rpc.ts';
import { BigContainerAssetLoader, createFetchRangeLoader, createPageDataProvider, getVirtualTextureDimensions, parseBigHeader } from './engine/big-parser.ts';
import { createWebGPUOnlyRenderer, showWebGPUFailure } from './engine/webgpu-only.ts';
import { RendererSeal, warmRendererVariants } from './engine/renderer-seal.ts';

const THREE = window.THREE;
const VT = window.AfterglowVT;
const GLTFLoader = window.AfterglowLoaders?.GLTFLoader;
const AssetStore = window.AfterglowAssets?.AssetStore;
if (!VT || !GLTFLoader || !AssetStore) throw new Error('Afterglow VT/asset/loader bundle is unavailable');

const FEEDBACK_INTERVAL = 8;
const MODEL_LAYER = 1;
const errors: string[] = [];
let frame = 0;
let last = performance.now();
let smoothedDt = 1 / 60;
let lastResult = { totalRequests: 0 };
const feedbackResults: (Map<number, any> | null)[] = [null, null, null, null];
const mergedFeedback = new Map<number, any>();
let animationEnabled = true;
let feedbackEnabled = true;
let programmatic = false;
let orbitAngle = 0;
let orbitVelocity = 0;
let cameraDistance = 4.1;
let zoomVelocity = 0;
const keys = new Set<string>();
const waiters: { target: number; resolve: () => void }[] = [];

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x11141a);
const camera = new THREE.PerspectiveCamera(48, innerWidth / innerHeight, 0.05, 100);
camera.position.set(0, 1.45, 4.1);
camera.lookAt(0, 1.25, 0);
camera.layers.enable(MODEL_LAYER);
const renderer = await createWebGPUOnlyRenderer({ antialias: false, trackTimestamp: false })
  .catch(error => { showWebGPUFailure(error); throw error; });
renderer.setPixelRatio(devicePixelRatio);
renderer.setSize(innerWidth, innerHeight);
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFShadowMap;
document.body.append(renderer.domElement);
const rendererSeal = new RendererSeal(renderer.backend);
renderer.backend.device.addEventListener('uncapturederror', event => errors.push(String(event.error?.message ?? event.error)));
addEventListener('error', event => errors.push(String(event.error?.stack ?? event.message)));
addEventListener('unhandledrejection', event => errors.push(String(event.reason?.stack ?? event.reason)));

scene.add(new THREE.HemisphereLight(0xc8d8ff, 0x231d1a, 2.1));
const key = new THREE.DirectionalLight(0xffead0, 3.5);
key.position.set(3, 5, 4);
key.layers.enable(MODEL_LAYER);
key.castShadow = true;
key.shadow.mapSize.set(2048, 2048);
key.shadow.camera.left = -3.5;
key.shadow.camera.right = 3.5;
key.shadow.camera.top = 3.5;
key.shadow.camera.bottom = -3.5;
key.shadow.camera.near = 0.1;
key.shadow.camera.far = 12;
key.shadow.bias = -0.0004;
key.shadow.normalBias = 0.025;
scene.add(key);
const rim = new THREE.DirectionalLight(0x7799ff, 2.2);
rim.position.set(-4, 3, -3);
rim.layers.enable(MODEL_LAYER);
scene.add(rim);
const floor = new THREE.Mesh(
  new THREE.CircleGeometry(3.2, 64),
  new THREE.MeshStandardMaterial({ color: 0x242830, roughness: 0.9, metalness: 0 }),
);
floor.rotation.x = -Math.PI / 2;
floor.receiveShadow = true;
scene.add(floor);
const grid = new THREE.GridHelper(6, 12, 0x536072, 0x303640);
grid.position.y = 0.002;
scene.add(grid);

const rangeLoader = createFetchRangeLoader();
const workerCount = Math.max(2, Math.min(4, Math.floor((navigator.hardwareConcurrency || 4) / 2)));
const textureRpcs = await Promise.all(Array.from({ length: workerCount }, () => Rpc.create({
  mainWasmUrl: 'afterglow_web.wasm', workerJsUrl: 'worker.js', workerWasmUrl: 'texture.wasm', timeoutMs: 10000,
})));
const textureWorkers = textureRpcs.map(rpc => new TextureClient(rpc));
const meshopt = await MeshoptClient.spawn('meshopt.wasm');
addEventListener('beforeunload', () => { for (const rpc of textureRpcs) rpc.terminate(); }, { once: true });
const prefix = await rangeLoader.read('rigged-vt.big', 0, 16);
const dataOffset = Number(new DataView(prefix.buffer, prefix.byteOffset + 8, 8).getBigUint64(0, true));
const headerBytes = await rangeLoader.read('rigged-vt.big', 0, dataOffset);
const { header } = parseBigHeader(headerBytes);
const format = renderer.backend.device.features.has('texture-compression-bc') ? 0
  : renderer.backend.device.features.has('texture-compression-astc') ? 1 : VT.FORMAT_RGBA;
const containerLoader = {
  load: (path: string) => rangeLoader.load(path),
  size: (path: string) => rangeLoader.size(path),
  read: (_path: string, offset: number, length: number) => rangeLoader.read('rigged-vt.big', offset, length),
};
const pageProvider = createPageDataProvider(containerLoader, header, textureWorkers, format);
const packedAssets = new BigContainerAssetLoader(rangeLoader, 'rigged-vt.big', header);
const assetStore = new AssetStore(packedAssets, meshopt);
const modelHandle = assetStore.loadOptimizedGLTF('model.glb', new GLTFLoader());
const secondModelHandle = assetStore.loadOptimizedGLTF('model-2.glb', new GLTFLoader());
while (modelHandle.state === 'loading' || secondModelHandle.state === 'loading') {
  assetStore.poll();
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
}
if (modelHandle.state !== 'ready' || secondModelHandle.state !== 'ready')
  throw new Error('packed models failed GLTF parsing or runtime mesh optimization');
const store = new VT.VirtualTextureStore(
  { read: (path: string, offset: number, length: number) => rangeLoader.read(path, offset, length), poll() {} },
  pageProvider, format, renderer.backend.device, new VT.VirtualTextureTuning(),
);
const paths = {
  albedo: 'model.glb#image-0',
  normal: 'model.glb#image-2',
  masks: 'model.glb#image-1',
};
const dimensions = getVirtualTextureDimensions(header, paths.albedo);
const materialSet = store.loadMaterialSet(paths, { ...dimensions, mipTail: true });
function loadIndependentTexture(path: string) {
  const existing = store.getEntry(path);
  if (existing) return existing;
  const size = getVirtualTextureDimensions(header, path);
  store.loadTexture(path, { ...size, mipTail: true });
  return store.getEntry(path);
}
const secondDimensions = getVirtualTextureDimensions(header, 'model-2.glb#image-0');
const feedbackPasses = Array.from({ length: 4 }, () => new VT.VirtualTextureFeedbackPass(0.125));
const feedbackPass = feedbackPasses[0];
for (const pass of feedbackPasses)
  pass.resize(renderer.domElement.width, renderer.domElement.height);

const gltf = modelHandle.asset;
const model = gltf.scene;
const skinnedMeshes: any[] = [];
let sourceMaterial: any = null;
model.traverse((object: any) => {
  if (!object.isMesh) return;
  object.layers.set(MODEL_LAYER);
  object.castShadow = true;
  object.receiveShadow = true;
  if (!object.isSkinnedMesh) throw new Error(`rigged VT demo found a non-skinned render mesh: ${object.name}`);
  if (Array.isArray(object.material)) throw new Error('rigged VT demo requires one material per primitive');
  sourceMaterial ??= object.material;
  skinnedMeshes.push(object);
});
if (skinnedMeshes.length === 0) throw new Error('model contains no SkinnedMesh');
if (gltf.animations.length === 0) throw new Error('model contains no animation clips');
const deformedBounds = new THREE.Box3();
const deformedVertex = new THREE.Vector3();
function measureDeformedBounds() {
  deformedBounds.makeEmpty();
  modelPivot.updateMatrixWorld(true);
  for (const mesh of skinnedMeshes) {
    const count = mesh.geometry.getAttribute('position').count;
    for (let index = 0; index < count; index++) {
      mesh.getVertexPosition(index, deformedVertex);
      deformedVertex.applyMatrix4(mesh.matrixWorld);
      deformedBounds.expandByPoint(deformedVertex);
    }
  }
  return deformedBounds;
}

// Keep normalization and grounding on an engine-owned parent that animation
// tracks cannot overwrite.
const modelPivot = new THREE.Group();
modelPivot.add(model);
const box = new THREE.Box3().setFromObject(model);
const size = box.getSize(new THREE.Vector3());
modelPivot.scale.setScalar(2.55 / size.y);
box.setFromObject(modelPivot);
const center = box.getCenter(new THREE.Vector3());
modelPivot.position.set(-center.x, -box.min.y, -center.z);
scene.add(modelPivot);

const pair = VT.createVirtualGltfMaterialPair(THREE, store, materialSet, feedbackPass.pixelScale, {
  addressMode: VT.VirtualTextureAddressMode.Repeat,
  qualityBias: 0,
  baseColorFactor: [sourceMaterial.color.r, sourceMaterial.color.g, sourceMaterial.color.b, sourceMaterial.opacity],
  roughnessFactor: sourceMaterial.roughness,
  metalnessFactor: sourceMaterial.metalness,
  normalScale: [1, -1],
  side: sourceMaterial.side,
});
const visibleMaterials = skinnedMeshes.map(mesh => mesh.material);
for (let index = 0; index < skinnedMeshes.length; index++) {
  visibleMaterials[index] = pair.material;
  skinnedMeshes[index].material = pair.material;
}
// GLTFLoader must parse the unmodified packed model, including its original
// material references. Once the VT replacement owns sampling, release those
// resident browser images so the demo proves one texture path rather than
// silently keeping a duplicate resident material alive.
const importedTextures = new Set<any>();
for (const property of ['map', 'normalMap', 'roughnessMap', 'metalnessMap', 'aoMap', 'emissiveMap']) {
  const imported = sourceMaterial[property];
  if (imported) importedTextures.add(imported);
}
for (const imported of importedTextures) {
  imported.dispose();
  imported.source?.data?.close?.();
}
sourceMaterial.dispose();
function useFeedbackMaterial(enabled: boolean): void {
  for (let index = 0; index < skinnedMeshes.length; index++)
    skinnedMeshes[index].material = enabled ? pair.feedbackMaterial : visibleMaterials[index];
}

const mixer = new THREE.AnimationMixer(model);
const action = mixer.clipAction(gltf.animations[0]);
action.play();
// The bind pose and first animation pose need not share the same root height.
// Evaluate the actual first frame, then ground its skinned bounds exactly.
mixer.setTime(0);
modelPivot.updateMatrixWorld(true);
let animatedBounds = measureDeformedBounds();
modelPivot.position.y -= animatedBounds.min.y;
modelPivot.updateMatrixWorld(true);
animatedBounds = measureDeformedBounds();
const groundedMinY = animatedBounds.min.y;
const skeleton = new THREE.SkeletonHelper(model);
skeleton.visible = false;
scene.add(skeleton);

// The latest downloaded GLB follows the exact same packed-model/runtime-
// meshopt path. Its differently sized base-color and normal VTs sample and
// feedback independently rather than relying on aligned material pages.
const secondGltf = secondModelHandle.asset;
const secondModel = secondGltf.scene;
const secondMeshes: any[] = [];
secondModel.traverse((object: any) => {
  if (!object.isMesh) return;
  object.layers.set(MODEL_LAYER);
  object.castShadow = true;
  object.receiveShadow = true;
  if (Array.isArray(object.material)) throw new Error('model 2 requires one material per primitive');
  secondMeshes.push(object);
});
if (secondMeshes.length === 0) throw new Error('model 2 contains no render meshes');
const secondSkinnedMeshes = secondMeshes.filter(mesh => mesh.isSkinnedMesh);
const secondPivot = new THREE.Group();
secondPivot.add(secondModel);
const secondBox = new THREE.Box3().setFromObject(secondModel);
const secondSize = secondBox.getSize(new THREE.Vector3());
secondPivot.scale.setScalar(2.55 / secondSize.y);
secondBox.setFromObject(secondPivot);
const secondCenter = secondBox.getCenter(new THREE.Vector3());
secondPivot.position.set(-secondCenter.x, -secondBox.min.y, -secondCenter.z);
scene.add(secondPivot);
const secondLayouts = new Map(secondGltf.materialTextures.map(layout => [layout.name, layout]));
const secondRecords: any[] = [];
const secondImportedTextures = new Set<any>();
const replacedSourceMaterials = new Set<any>();
let secondMaxFeedbackChannels = 1;
for (const mesh of secondMeshes) {
  const source = mesh.material;
  const layout = secondLayouts.get(source.name);
  if (!layout || layout.baseColorImage === null) {
    secondRecords.push({ mesh, pair: null, visibleMaterial: source, wasVisible: mesh.visible });
    continue;
  }
  const entry = (image: number | null) => image === null ? undefined :
    loadIndependentTexture(`model-2.glb#image-${image}`);
  const set = {
    albedo: entry(layout.baseColorImage),
    normal: entry(layout.normalImage),
    masks: entry(layout.metallicRoughnessImage),
    emissive: entry(layout.emissiveImage),
  };
  const pair = VT.createVirtualGltfMaterialPair(THREE, store, set, feedbackPass.pixelScale, {
    addressMode: VT.VirtualTextureAddressMode.Repeat,
    qualityBias: 0,
    baseColorFactor: [source.color.r, source.color.g, source.color.b, source.opacity],
    roughnessFactor: source.roughness,
    metalnessFactor: source.metalness,
    normalScale: [1, -1],
    emissiveFactor: [source.emissive.r, source.emissive.g, source.emissive.b],
    transparent: source.transparent,
    depthWrite: source.depthWrite,
    side: source.side,
  });
  secondMaxFeedbackChannels = Math.max(secondMaxFeedbackChannels, pair.feedbackMaterials.length);
  mesh.material = pair.material;
  secondRecords.push({ mesh, pair, visibleMaterial: pair.material, wasVisible: mesh.visible });
  replacedSourceMaterials.add(source);
  for (const property of ['map', 'normalMap', 'roughnessMap', 'metalnessMap', 'aoMap', 'emissiveMap']) {
    const imported = source[property];
    if (imported) secondImportedTextures.add(imported);
  }
}
for (const imported of secondImportedTextures) { imported.dispose(); imported.source?.data?.close?.(); }
for (const material of replacedSourceMaterials) material.dispose();
const secondClip = secondGltf.animations.find(clip => clip.name === 'Idle') ?? secondGltf.animations[0];
const secondMixer = new THREE.AnimationMixer(secondModel);
const secondAction = secondMixer.clipAction(secondClip);
secondAction.play();
secondMixer.setTime(0);
const secondSkeleton = new THREE.SkeletonHelper(secondModel);
secondSkeleton.visible = false;
scene.add(secondSkeleton);
const secondVertex = new THREE.Vector3(), secondBounds = new THREE.Box3();
function measureSecondBounds() {
  secondBounds.makeEmpty(); secondPivot.updateMatrixWorld(true);
  for (const mesh of secondMeshes) for (let index = 0; index < mesh.geometry.getAttribute('position').count; index++) {
    mesh.getVertexPosition(index, secondVertex); secondVertex.applyMatrix4(mesh.matrixWorld); secondBounds.expandByPoint(secondVertex);
  }
  return secondBounds;
}
let measuredSecond = measureSecondBounds();
secondPivot.position.y -= measuredSecond.min.y;
secondPivot.updateMatrixWorld(true);
measuredSecond = measureSecondBounds();
const secondGroundedMinY = measuredSecond.min.y;
let activeModel = 0;
let skeletonRequested = false;
secondPivot.visible = false;
function setActiveModel(modelNumber: number): void {
  activeModel = modelNumber === 2 ? 1 : 0;
  modelPivot.visible = activeModel === 0;
  secondPivot.visible = activeModel === 1;
  skeleton.visible = activeModel === 0 && skeletonRequested;
  secondSkeleton.visible = activeModel === 1 && skeletonRequested;
  for (let index = 0; index < feedbackResults.length; index++) feedbackResults[index] = null;
  orbitAngle = 0; orbitVelocity = 0; cameraDistance = 4.1; zoomVelocity = 0;
}
function useSecondFeedbackMaterial(index: number, enabled: boolean): void {
  for (const record of secondRecords) {
    if (!record.pair) { record.mesh.visible = enabled ? false : record.wasVisible; continue; }
    const feedbackIndex = Math.min(index, record.pair.feedbackMaterials.length - 1);
    record.mesh.material = enabled ? record.pair.feedbackMaterials[feedbackIndex] : record.visibleMaterial;
  }
}

// Compile visible and integer feedback variants against the exact same
// SkinnedMesh. No bind-pose proxy can drift from the animated feedback shape.
await warmRendererVariants(renderer, [{ scene, camera }]);
skeleton.visible = true;
await warmRendererVariants(renderer, [{ scene, camera }]);
skeleton.visible = false;
modelPivot.visible = false; secondPivot.visible = true;
await warmRendererVariants(renderer, [{ scene, camera }]);
renderer.render(scene, camera);
secondSkeleton.visible = true;
await warmRendererVariants(renderer, [{ scene, camera }]);
renderer.render(scene, camera);
secondSkeleton.visible = false;
const previousTarget = renderer.getRenderTarget();
const previousMask = camera.layers.mask;
const previousShadows = renderer.shadowMap.enabled;
renderer.shadowMap.enabled = false;
camera.layers.set(MODEL_LAYER);
modelPivot.visible = true; secondPivot.visible = false;
useFeedbackMaterial(true);
renderer.setRenderTarget(feedbackPass.target);
await warmRendererVariants(renderer, [{ scene, camera }]);
renderer.render(scene, camera);
useFeedbackMaterial(false);
modelPivot.visible = false; secondPivot.visible = true;
for (let index = 0; index < secondMaxFeedbackChannels; index++) {
  useSecondFeedbackMaterial(index, true);
  renderer.setRenderTarget(feedbackPasses[index].target);
  await warmRendererVariants(renderer, [{ scene, camera }]);
  renderer.render(scene, camera);
  useSecondFeedbackMaterial(index, false);
}
renderer.setRenderTarget(previousTarget);
camera.layers.mask = previousMask;
renderer.shadowMap.enabled = previousShadows;
modelPivot.visible = true; secondPivot.visible = false;
renderer.render(scene, camera);
store.attachRenderer(renderer);
rendererSeal.seal();

function submitFeedback(): void {
  const mask = camera.layers.mask;
  const shadows = renderer.shadowMap.enabled;
  renderer.shadowMap.enabled = false;
  camera.layers.set(MODEL_LAYER);
  if (activeModel === 0) {
    useFeedbackMaterial(true);
    feedbackPass.submit(renderer, scene, camera, store);
    useFeedbackMaterial(false);
  } else {
    for (let index = 0; index < secondMaxFeedbackChannels; index++) {
      useSecondFeedbackMaterial(index, true);
      feedbackPasses[index].submit(renderer, scene, camera, store);
      useSecondFeedbackMaterial(index, false);
    }
  }
  camera.layers.mask = mask;
  renderer.shadowMap.enabled = shadows;
}
function setAnimationEnabled(enabled: boolean): void {
  animationEnabled = Boolean(enabled);
  action.paused = !animationEnabled;
  secondAction.paused = !animationEnabled;
}
function setSkeletonVisible(visible: boolean): void {
  skeletonRequested = Boolean(visible);
  skeleton.visible = activeModel === 0 && skeletonRequested;
  secondSkeleton.visible = activeModel === 1 && skeletonRequested;
}

const hud = document.getElementById('hud')!;
renderer.setAnimationLoop(now => {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  smoothedDt = smoothedDt * 0.95 + dt * 0.05;
  store.recordFrameTime(dt * 1000);
  if (animationEnabled) { mixer.update(dt); secondMixer.update(dt); }
  const rotateInput = programmatic ? 0 : (keys.has('d') ? 1 : 0) - (keys.has('a') ? 1 : 0);
  const zoomInput = programmatic ? 0 : (keys.has('s') ? 1 : 0) - (keys.has('w') ? 1 : 0);
  orbitVelocity += rotateInput * 7.5 * dt;
  zoomVelocity += zoomInput * 8 * dt;
  const damping = Math.exp(-7 * dt);
  orbitVelocity *= damping;
  zoomVelocity *= damping;
  orbitAngle += orbitVelocity * dt;
  cameraDistance = Math.max(1.35, Math.min(8, cameraDistance + zoomVelocity * dt));
  if ((cameraDistance === 1.35 && zoomVelocity < 0) || (cameraDistance === 8 && zoomVelocity > 0)) zoomVelocity = 0;
  camera.position.set(Math.sin(orbitAngle) * cameraDistance, 1.45, Math.cos(orbitAngle) * cameraDistance);
  camera.lookAt(0, 1.25, 0);
  const expectedFeedback = activeModel === 0 ? 1 : secondMaxFeedbackChannels;
  for (let index = 0; index < feedbackPasses.length; index++) {
    const completed = feedbackPasses[index].consume();
    if (completed && index < expectedFeedback) feedbackResults[index] = completed;
  }
  let feedbackBatchReady = true;
  for (let index = 0; index < expectedFeedback; index++)
    if (feedbackResults[index] === null) feedbackBatchReady = false;
  if (feedbackBatchReady) {
    mergedFeedback.clear();
    for (let index = 0; index < expectedFeedback; index++) {
      for (const [key, request] of feedbackResults[index]!) mergedFeedback.set(key, request);
      feedbackResults[index] = null;
    }
    lastResult = store.processFeedback(mergedFeedback);
  }
  store.poll();
  renderer.render(scene, camera);
  if (feedbackEnabled && frame % FEEDBACK_INTERVAL === 0) submitFeedback();
  frame++;
  for (let index = waiters.length - 1; index >= 0; index--) {
    if (frame < waiters[index].target) continue;
    waiters[index].resolve();
    waiters.splice(index, 1);
  }
  if (frame % 15 === 0) {
    const stats = store.getStats();
    const activeGltf = activeModel === 0 ? gltf : secondGltf;
    const activeDimensions = activeModel === 0 ? dimensions : secondDimensions;
    const activeMeshes = activeModel === 0 ? skinnedMeshes : secondMeshes;
    const activeMips = activeModel === 0 ? feedbackPass.getLatestMips() :
      feedbackPasses.slice(0, secondMaxFeedbackChannels).flatMap(pass => pass.getLatestMips());
    hud.innerHTML = `<b>afterglow — Model VT</b> · model ${activeModel + 1}/2<br>` +
      `Meshes: ${activeMeshes.length} · ${activeModel === 0 ? skinnedMeshes.length : secondSkinnedMeshes.length} skinned<br>` +
      `Animation: ${activeModel === 0 ? gltf.animations[0].name : secondClip.name} · ${(activeModel === 0 ? gltf.animations[0].duration : secondClip.duration).toFixed(2)} s · ${animationEnabled ? 'playing' : 'paused'}<br>` +
      `Pipeline: GLB from .big · runtime meshopt ACMR ${activeGltf.meshOptimization[0].originalAcmr.toFixed(2)}→${activeGltf.meshOptimization[0].optimizedAcmr.toFixed(2)}<br>` +
      `Material: extracted glTF channels through VT<br>` +
      `Base color: ${activeDimensions.width}×${activeDimensions.height} · atlas: ${store.atlasWidth}² · ${format === 0 ? 'BC7' : format === 1 ? 'ASTC' : 'RGBA'}<br>` +
      `Resident: ${stats.atlasSlotsUsed}/${stats.atlasSlotsTotal} · pending: ${stats.pendingPages} · requests: ${lastResult.totalRequests}<br>` +
      `Feedback mips: [${activeMips.join(',')}] · FPS: ${(1 / smoothedDt).toFixed(0)} · errors: ${errors.length}`;
  }
});

addEventListener('resize', () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
  for (const pass of feedbackPasses) pass.resize(renderer.domElement.width, renderer.domElement.height);
});
addEventListener('keydown', event => {
  if (programmatic) return;
  const keyName = event.key.toLowerCase();
  keys.add(keyName);
  if (event.repeat) return;
  if (keyName === '1') setActiveModel(1);
  else if (keyName === '2') setActiveModel(2);
  else if (event.code === 'Space') setAnimationEnabled(!animationEnabled);
  else if (keyName === 'b') setSkeletonVisible(!skeletonRequested);
  else if (keyName === 'f') feedbackEnabled = !feedbackEnabled;
  else if (keyName === 'r') { keys.clear(); orbitAngle = 0; orbitVelocity = 0; cameraDistance = 4.1; zoomVelocity = 0; }
});
addEventListener('keyup', event => keys.delete(event.key.toLowerCase()));
addEventListener('blur', () => keys.clear());

window.__afterglowRiggedVT = {
  setProgrammatic(enabled: boolean) { programmatic = Boolean(enabled); },
  setAnimationEnabled,
  setAnimationTime(seconds: number) {
    if (activeModel === 0) mixer.setTime(Math.max(0, seconds) % gltf.animations[0].duration);
    else secondMixer.setTime(Math.max(0, seconds) % secondClip.duration);
  },
  measureBounds() {
    const bounds = activeModel === 0 ? measureDeformedBounds() : measureSecondBounds();
    return { minY: bounds.min.y, maxY: bounds.max.y };
  },
  setActiveModel,
  setFeedbackEnabled(enabled: boolean) { feedbackEnabled = Boolean(enabled); },
  setSkeletonVisible,
  setView(angle: number, distance: number) {
    orbitAngle = angle; orbitVelocity = 0;
    cameraDistance = Math.max(1.35, Math.min(8, distance)); zoomVelocity = 0;
  },
  step(count = 1) { return new Promise<void>(resolve => waiters.push({ target: frame + count, resolve })); },
  telemetry: () => store.getStats(),
  debugSnapshot: () => store.getDebugSnapshot(),
  feedbackMips: () => feedbackPass.getLatestMips(),
  errorCount: () => errors.length,
  errors: () => errors.slice(),
  status: () => {
    const activeGltf = activeModel === 0 ? gltf : secondGltf;
    const activeMeshes = activeModel === 0 ? skinnedMeshes : secondMeshes;
    const activeDimensions = activeModel === 0 ? dimensions : secondDimensions;
    const optimization = activeGltf.meshOptimization[0];
    return {
      activeModel: activeModel + 1,
      meshes: activeMeshes.length,
      skinnedMeshes: activeModel === 0 ? skinnedMeshes.length : secondSkinnedMeshes.length,
      bones: activeModel === 0 ? skinnedMeshes[0].skeleton.bones.length : (secondSkinnedMeshes[0]?.skeleton.bones.length ?? 0),
      clip: activeModel === 0 ? gltf.animations[0].name : secondClip.name,
      clipDuration: activeModel === 0 ? gltf.animations[0].duration : secondClip.duration,
      animationEnabled, feedbackEnabled, skeletonVisible: skeleton.visible,
      groundedMinY: activeModel === 0 ? groundedMinY : secondGroundedMinY,
      orbitAngle, cameraDistance,
      sourceWidth: activeDimensions.width, sourceHeight: activeDimensions.height,
      material: 'virtual-gltf-metallic-roughness',
      packedAsset: activeModel === 0 ? 'model.glb' : 'model-2.glb',
      meshOptimized: activeGltf.meshOptimization.length === activeMeshes.length,
      originalAcmr: optimization.originalAcmr,
      optimizedAcmr: optimization.optimizedAcmr,
      preservedAttributes: optimization.preservedAttributes,
      sameMeshFeedback: true,
      feedbackChannels: activeModel === 0 ? 1 : secondMaxFeedbackChannels,
      shadows: renderer.shadowMap.enabled && key.castShadow && floor.receiveShadow,
      shadowMapSize: key.shadow.mapSize.x,
      rendererSealed: rendererSeal.isSealed,
      pipelineViolations: rendererSeal.violations,
    };
  },
};
