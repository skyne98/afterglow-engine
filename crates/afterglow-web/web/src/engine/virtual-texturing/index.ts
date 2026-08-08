import {
  DEFAULT_VIRTUAL_MATERIAL_MIP_BIASES,
  type VirtualMaterialMipBiases,
} from './virtual-texture.ts';
export { VirtualGltfBinding, type VirtualGltfBindingOptions } from './virtual-gltf-binding.ts';
export {
  VirtualMaterialBinding,
  type VirtualMaterialBindingOptions,
} from './virtual-material-binding.ts';
export {
  VirtualPomSceneBinding,
  type VirtualPomSceneBindingOptions,
} from './virtual-pom-binding.ts';
export {
  type VirtualPomMaterialOptions,
  type VirtualPomMaterialPair,
} from './virtual-texture-material.ts';
export {
  DEFAULT_VIRTUAL_MATERIAL_MIP_BIASES,
  type VirtualMaterialMipBiases,
};
export {
  FORMAT_R16F,
  FORMAT_R8,
  FORMAT_RGBA,
  SLOT_SIZE,
} from './virtual-texture-format.ts';
export {
  MemoryTextureWriteStatus,
  MemoryVirtualTextureSource,
  type MemoryPageSourceOptions,
  type MemoryVirtualTextureAddressMode,
  type MemoryVirtualTextureFormat,
  type MemoryVirtualTextureMipFilter,
} from './memory-page-source.ts';
export {
  decodeMemoryTextureSnapshot,
  encodeMemoryTextureSnapshot,
  restoreMemoryTextureSnapshot,
  type DecodedMemoryTextureSnapshot,
  type RestoreMemoryTextureCapacities,
} from './memory-texture-snapshot.ts';
export {
  MemoryTexturePersistenceStatus,
  VirtualTextureSystem,
  type MemoryTextureLoadResult,
  type VirtualTextureDescriptor,
  type VirtualTextureHandle,
  type VirtualTextureInfo,
  type VirtualTextureMaterialSet,
  type VirtualTexturePoolConfig,
  type VirtualTextureStorageFormat,
  type VirtualTextureSystemOptions,
} from './virtual-texture-system.ts';
export {
  VirtualTextureNodeBinding,
  virtualTextureNode,
  type VirtualTextureFeedbackNodeOptions,
  type VirtualTextureSampleNodeOptions,
} from './virtual-texture-nodes.ts';
export {
  VirtualShaderBinding,
  type VirtualShaderBindingOptions,
} from './virtual-shader-binding.ts';
export {
  VirtualTextureTuning,
  type VirtualTextureRuntimeCapacities,
} from './virtual-texture-tuning.ts';
export {
  FeedbackRegistrationStatus,
  VirtualTextureFeedbackCoordinator,
  type FeedbackRenderable,
  type VirtualTextureFeedbackOptions,
  type VirtualTextureGpuTimings,
} from './virtual-texture-feedback-coordinator.ts';
export { generateTerrainPage } from './procedural-vt.ts';
export {
  assertPomGeneratedWgsl,
  validatePomShaderWarmup,
  type PomShaderWarmupResult,
} from './surface-detail.ts';
