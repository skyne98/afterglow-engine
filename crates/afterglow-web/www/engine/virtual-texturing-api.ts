import {
  FORMAT_RGBA, SLOT_SIZE, VirtualTextureStore, type PageDataProvider,
} from './virtual-texture.ts';
export { VirtualGltfBinding, type VirtualGltfBindingOptions } from './virtual-gltf-binding.ts';
export {
  VirtualMaterialBinding, type VirtualMaterialBindingOptions,
} from './virtual-material-binding.ts';
export { FORMAT_RGBA, SLOT_SIZE };
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
  type FeedbackRenderable,
} from './virtual-texture-feedback-coordinator.ts';
