import type * as THREE_TYPES from 'three/webgpu';
import type * as TSL_TYPES from 'three/tsl';

type ThreeWebGpuRuntime = typeof THREE_TYPES & typeof TSL_TYPES;
import type { VirtualMaterialSet, VirtualTextureEntry, VirtualTextureStore } from './virtual-texture.ts';
import {
  PAGE_BORDER, PAGE_SIZE, VT_FEEDBACK_WGSL, VT_RESOLVE_MATERIAL_MIP4_WGSL,
  VT_SAMPLE_LEVEL_WGSL, VT_SAMPLE_WGSL,
} from './virtual-texture.ts';

/** WebGPU address mode values shared by the VT sampling and feedback shaders. */
export enum VirtualTextureAddressMode {
  Clamp = 0,
  Repeat = 1,
  MirrorRepeat = 2,
}

export interface VirtualTextureSampling {
  channel?: number;
  /** Column-major Three.js Matrix3 elements applied after selecting the UV channel. */
  matrix?: readonly [number, number, number, number, number, number, number, number, number];
  addressMode?: VirtualTextureAddressMode;
  /** 0 = linear atlas sampling, 1 = nearest texel sampling. */
  filterMode?: number;
}

export interface VirtualGltfTextureSampling {
  albedo?: VirtualTextureSampling;
  normal?: VirtualTextureSampling;
  masks?: VirtualTextureSampling;
  emissive?: VirtualTextureSampling;
}

export interface VirtualGltfMaterialOptions {
  addressMode?: VirtualTextureAddressMode;
  qualityBias?: number;
  sampling?: Readonly<VirtualGltfTextureSampling>;
  baseColorFactor?: readonly [number, number, number, number];
  roughnessFactor?: number;
  metalnessFactor?: number;
  normalScale?: readonly [number, number];
  emissiveFactor?: readonly [number, number, number];
  transmissionFactor?: number;
  thicknessFactor?: number;
  ior?: number;
  transparent?: boolean;
  alphaTest?: number;
  depthWrite?: boolean;
  depthTest?: boolean;
  blending?: THREE_TYPES.Blending;
  side?: THREE_TYPES.Side;
}

/** A visible glTF PBR material and its matching integer-output feedback material. */
export interface VirtualGltfMaterialPair {
  material: THREE_TYPES.MeshStandardNodeMaterial;
  /** Albedo feedback material; retained for aligned linked sets. */
  feedbackMaterial: THREE_TYPES.MeshBasicNodeMaterial;
  /** One material for linked sets, otherwise one per independently sized channel. */
  feedbackMaterials: readonly THREE_TYPES.MeshBasicNodeMaterial[];
  feedbackEntries: readonly VirtualTextureEntry[];
}

/**
 * Build a glTF-metallic/roughness VT material pair.
 *
 * `set.albedo`, optional `set.normal`, and optional `set.masks` correspond to
 * glTF base color, normal, and metallicRoughness textures. The packed texture
 * uses glTF's channels: G=roughness and B=metalness. Missing texture roles use
 * their scalar factors. Feedback is emitted for albedo; the
 * store's linked-material group expands that request to every physical channel.
 *
 * The returned materials are geometry-agnostic. Three.js therefore applies its
 * normal vertex path for Mesh, InstancedMesh, SkinnedMesh, and morph targets.
 * Render feedback with the same object (temporarily swapping its material) when
 * exact animated/deformed coverage is required.
 */
export function createVirtualGltfMaterialPair(
  three: ThreeWebGpuRuntime,
  store: VirtualTextureStore,
  set: VirtualMaterialSet,
  feedbackPixelScale: THREE_TYPES.Vector2,
  options: Readonly<VirtualGltfMaterialOptions> = {},
): VirtualGltfMaterialPair {
  const addressMode = options.addressMode ?? VirtualTextureAddressMode.Repeat;
  const qualityBias = options.qualityBias ?? 0;
  type TextureRole = keyof VirtualGltfTextureSampling;
  const roleSampling = (role: TextureRole): Readonly<VirtualTextureSampling> | undefined =>
    options.sampling?.[role];
  const roleAddress = (role: TextureRole) =>
    three.uint(roleSampling(role)?.addressMode ?? addressMode);
  const roleFilter = (role: TextureRole) => three.uint(roleSampling(role)?.filterMode ?? 0);
  const roleUv = (role: TextureRole): THREE_TYPES.Node<'vec2'> => {
    const sampling = roleSampling(role);
    const uv = three.uv(sampling?.channel ?? 0);
    const matrix = sampling?.matrix;
    if (!matrix) return uv;
    return three.vec2(
      uv.x.mul(matrix[0]).add(uv.y.mul(matrix[3])).add(matrix[6]),
      uv.x.mul(matrix[1]).add(uv.y.mul(matrix[4])).add(matrix[7]),
    );
  };
  const sameSampling = (first: TextureRole, second: TextureRole): boolean => {
    const a = roleSampling(first), b = roleSampling(second);
    if ((a?.channel ?? 0) !== (b?.channel ?? 0) ||
        (a?.addressMode ?? addressMode) !== (b?.addressMode ?? addressMode)) return false;
    const am = a?.matrix, bm = b?.matrix;
    if (!am && !bm) return true;
    if (!am || !bm) return false;
    for (let index = 0; index < 9; index++) if (am[index] !== bm[index]) return false;
    return true;
  };
  const baseColorFactor = options.baseColorFactor ?? [1, 1, 1, 1];
  const roughnessFactor = options.roughnessFactor ?? 1;
  const metalnessFactor = options.metalnessFactor ?? 1;
  const normalScale = options.normalScale ?? [1, -1];
  const emissiveFactor = options.emissiveFactor ?? [1, 1, 1];
  const side = options.side ?? three.FrontSide;

  const atlas = three.texture(store.atlasTexture);
  const atlasSampler = three.sampler(atlas);
  const resolveMaterialMip = three.wgslFn(VT_RESOLVE_MATERIAL_MIP4_WGSL);
  const sampleLevel = three.wgslFn(VT_SAMPLE_LEVEL_WGSL);
  const sampleVirtual = three.wgslFn(VT_SAMPLE_WGSL);
  const feedback = three.wgslFn(VT_FEEDBACK_WGSL);
  const virtualSize = three.uniform(new three.Vector2(set.albedo.width, set.albedo.height));
  const pageGrid = three.uniform(new three.Vector2(set.albedo.pageGridX, set.albedo.pageGridY));
  const atlasSize = three.uniform(new three.Vector2(store.atlasWidth, store.atlasHeight));

  const descriptors: Array<{ entry: VirtualTextureEntry; role: TextureRole }> = [
    { entry: set.albedo, role: 'albedo' },
  ];
  if (set.normal) descriptors.push({ entry: set.normal, role: 'normal' });
  if (set.masks) descriptors.push({ entry: set.masks, role: 'masks' });
  if (set.emissive) descriptors.push({ entry: set.emissive, role: 'emissive' });
  const entries = descriptors.map(descriptor => descriptor.entry);
  const aligned = descriptors.every(descriptor =>
    descriptor.entry.width === set.albedo.width && descriptor.entry.height === set.albedo.height &&
    descriptor.entry.pageGridX === set.albedo.pageGridX && descriptor.entry.pageGridY === set.albedo.pageGridY &&
    descriptor.entry.maxMip === set.albedo.maxMip && sameSampling('albedo', descriptor.role));
  if (aligned) store.linkMaterialSet(set);
  const table = (entry: VirtualTextureEntry) => three.texture(entry.pageTableTexture);
  const albedoTable = table(set.albedo);
  const normalTable = table(set.normal ?? set.albedo);
  const masksTable = table(set.masks ?? set.albedo);
  const fourthTable = table(set.emissive ?? set.masks ?? set.albedo);
  const resolve = () => resolveMaterialMip({
    pageTable0: albedoTable,
    pageTable1: normalTable,
    pageTable2: masksTable,
    pageTable3: fourthTable,
    uv: roleUv('albedo'),
    virtualSize,
    pageGrid,
    pageSize: three.float(PAGE_SIZE),
    maxMip: three.float(set.albedo.maxMip),
    textureMaxMip: three.float(set.albedo.textureMaxMip),
    addressMode: roleAddress('albedo'),
  });
  const sample = (entry: VirtualTextureEntry, role: TextureRole) => {
    const entryVirtualSize = aligned ? virtualSize : three.uniform(new three.Vector2(entry.width, entry.height));
    const entryPageGrid = aligned ? pageGrid : three.uniform(new three.Vector2(entry.pageGridX, entry.pageGridY));
    if (aligned) return sampleLevel({
      pageTable: table(entry), atlas, atlasSampler, uv: roleUv(role),
      virtualSize: entryVirtualSize, pageGrid: entryPageGrid,
      pageSize: three.float(PAGE_SIZE), pageBorder: three.float(PAGE_BORDER), atlasSize,
      maxMip: three.float(entry.maxMip), resolvedMip: resolve(),
      filterMode: roleFilter(role), addressMode: roleAddress(role),
    }) as THREE_TYPES.Node<'vec4'>;
    return sampleVirtual({
      pageTable: table(entry), atlas, atlasSampler, uv: roleUv(role),
      virtualSize: entryVirtualSize, pageGrid: entryPageGrid,
      pageSize: three.float(PAGE_SIZE), pageBorder: three.float(PAGE_BORDER), atlasSize,
      maxMip: three.float(entry.maxMip), textureMaxMip: three.float(entry.textureMaxMip),
      filterMode: roleFilter(role), addressMode: roleAddress(role),
    }) as THREE_TYPES.Node<'vec4'>;
  };

  const material = (options.transmissionFactor ?? 0) > 0
    ? new three.MeshPhysicalNodeMaterial({
      side,
      transparent: options.transparent ?? false,
      depthWrite: options.depthWrite ?? !(options.transparent ?? false),
    })
    : new three.MeshStandardNodeMaterial({
      side,
      transparent: options.transparent ?? false,
      depthWrite: options.depthWrite ?? !(options.transparent ?? false),
    });
  if (material instanceof three.MeshPhysicalNodeMaterial) {
    material.transmission = options.transmissionFactor ?? 0;
    material.thickness = options.thicknessFactor ?? 0;
    material.ior = options.ior ?? 1.5;
  }
  material.alphaTest = options.alphaTest ?? 0;
  material.depthTest = options.depthTest ?? true;
  material.blending = options.blending ?? three.NormalBlending;
  material.colorNode = three.Fn(() => {
    const texel = sample(set.albedo, 'albedo');
    const linearColor = three.sRGBTransferEOTF(texel.rgb) as THREE_TYPES.Node<'vec3'>;
    return three.vec4(
      linearColor.mul(three.vec3(
        baseColorFactor[0], baseColorFactor[1], baseColorFactor[2],
      )),
      texel.a.mul(baseColorFactor[3]),
    );
  })();
  if (set.normal) {
    material.normalNode = three.normalMap(
      sample(set.normal, 'normal').xyz,
      three.vec2(normalScale[0], normalScale[1]),
    );
  }
  if (set.emissive) {
    const linearEmissive = three.sRGBTransferEOTF(sample(set.emissive, 'emissive').rgb) as THREE_TYPES.Node<'vec3'>;
    material.emissiveNode = linearEmissive.mul(three.vec3(
      emissiveFactor[0], emissiveFactor[1], emissiveFactor[2],
    ));
  }
  if (set.masks) {
    // Independent reads avoid ordering assumptions between Three's roughness
    // and metalness node flows. Both resolve the same linked-material mip.
    material.roughnessNode = sample(set.masks, 'masks').g.mul(roughnessFactor);
    material.metalnessNode = sample(set.masks, 'masks').b.mul(metalnessFactor);
  } else {
    material.roughness = roughnessFactor;
    material.metalness = metalnessFactor;
  }

  const albedoDescriptor = descriptors[0];
  if (!albedoDescriptor) throw new Error('virtual material requires albedo feedback');
  const feedbackDescriptors = aligned ? [albedoDescriptor] : descriptors;
  const feedbackEntries = feedbackDescriptors.map(descriptor => descriptor.entry);
  const feedbackMaterials = feedbackDescriptors.map(descriptor => {
    const feedbackMaterial = new three.MeshBasicNodeMaterial({ side });
    feedbackMaterial.fragmentNode = three.Fn(() => feedback({
      sampleUV: roleUv(descriptor.role), gradientUV: roleUv(descriptor.role),
      feedbackPixelScale: three.uniform(feedbackPixelScale),
      virtualSize: three.uniform(new three.Vector2(descriptor.entry.width, descriptor.entry.height)),
      pageGrid: three.uniform(new three.Vector2(descriptor.entry.pageGridX, descriptor.entry.pageGridY)),
      maxMip: three.float(descriptor.entry.maxMip), qualityBias: three.float(qualityBias),
      addressMode: roleAddress(descriptor.role), textureId: three.uint(descriptor.entry.textureId),
    }))();
    return feedbackMaterial;
  });
  const feedbackMaterial = feedbackMaterials[0];
  if (!feedbackMaterial) throw new Error('virtual material requires at least one feedback channel');
  return {
    material,
    feedbackMaterial,
    feedbackMaterials,
    feedbackEntries,
  };
}
