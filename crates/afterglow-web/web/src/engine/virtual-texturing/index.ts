import {
  FORMAT_RGBA, SLOT_SIZE, VirtualTextureStore, VirtualTextureTuning, type PageDataProvider,
} from './virtual-texture.ts';
export { VirtualGltfBinding, type VirtualGltfBindingOptions } from './virtual-gltf-binding.ts';
export {
  VirtualMaterialBinding, type VirtualMaterialBindingOptions,
} from './virtual-material-binding.ts';
export {
  VirtualPomSceneBinding, type VirtualPomSceneBindingOptions,
} from './virtual-pom-binding.ts';
export {
  type VirtualPomMaterialOptions, type VirtualPomMaterialPair,
} from './virtual-texture-material.ts';
export { FORMAT_RGBA, SLOT_SIZE, VirtualTextureStore, VirtualTextureTuning };
export { VirtualTextureFeedbackPass } from './virtual-texture-feedback-pass.ts';
export function createProceduralVirtualTextureStore(
  provider: PageDataProvider,
  device: GPUDevice,
): VirtualTextureStore {
  return new VirtualTextureStore(
    { async read() { return new Uint8Array(); }, poll() {} }, provider, FORMAT_RGBA, device,
  );
}
export {
  FeedbackRegistrationStatus, VirtualTextureFeedbackCoordinator,
  type FeedbackRenderable, type VirtualTextureGpuTimings,
} from './virtual-texture-feedback-coordinator.ts';
export { generateTerrainPage } from './procedural-vt.ts';
export {
  assertPomGeneratedWgsl,
  validatePomShaderWarmup,
  type PomShaderWarmupResult,
} from './surface-detail.ts';
