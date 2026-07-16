import type * as THREE_TYPES from 'three';
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

export interface VirtualGltfMaterialOptions {
  addressMode?: VirtualTextureAddressMode;
  qualityBias?: number;
  baseColorFactor?: readonly [number, number, number, number];
  roughnessFactor?: number;
  metalnessFactor?: number;
  normalScale?: readonly [number, number];
  emissiveFactor?: readonly [number, number, number];
  transparent?: boolean;
  depthWrite?: boolean;
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
  three: typeof THREE_TYPES,
  store: VirtualTextureStore,
  set: VirtualMaterialSet,
  feedbackPixelScale: THREE_TYPES.Vector2,
  options: Readonly<VirtualGltfMaterialOptions> = {},
): VirtualGltfMaterialPair {
  const addressMode = options.addressMode ?? VirtualTextureAddressMode.Repeat;
  const qualityBias = options.qualityBias ?? 0;
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
  const addressModeNode = three.uint(addressMode);

  const entries = [set.albedo, set.normal, set.masks, set.emissive]
    .filter((entry): entry is VirtualTextureEntry => entry !== undefined);
  const aligned = entries.every(entry => entry.width === set.albedo.width && entry.height === set.albedo.height &&
    entry.pageGridX === set.albedo.pageGridX && entry.pageGridY === set.albedo.pageGridY &&
    entry.maxMip === set.albedo.maxMip);
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
    uv: three.uv(),
    virtualSize,
    pageGrid,
    pageSize: three.float(PAGE_SIZE),
    maxMip: three.float(set.albedo.maxMip),
    textureMaxMip: three.float(set.albedo.textureMaxMip),
    addressMode: addressModeNode,
  });
  const sample = (entry: VirtualTextureEntry) => {
    const entryVirtualSize = aligned ? virtualSize : three.uniform(new three.Vector2(entry.width, entry.height));
    const entryPageGrid = aligned ? pageGrid : three.uniform(new three.Vector2(entry.pageGridX, entry.pageGridY));
    if (aligned) return sampleLevel({
      pageTable: table(entry), atlas, atlasSampler, uv: three.uv(),
      virtualSize: entryVirtualSize, pageGrid: entryPageGrid,
      pageSize: three.float(PAGE_SIZE), pageBorder: three.float(PAGE_BORDER), atlasSize,
      maxMip: three.float(entry.maxMip), resolvedMip: resolve(), addressMode: addressModeNode,
    });
    return sampleVirtual({
      pageTable: table(entry), atlas, atlasSampler, uv: three.uv(),
      virtualSize: entryVirtualSize, pageGrid: entryPageGrid,
      pageSize: three.float(PAGE_SIZE), pageBorder: three.float(PAGE_BORDER), atlasSize,
      maxMip: three.float(entry.maxMip), textureMaxMip: three.float(entry.textureMaxMip),
      addressMode: addressModeNode,
    });
  };

  const material = new three.MeshStandardNodeMaterial({
    side,
    transparent: options.transparent ?? false,
    depthWrite: options.depthWrite ?? !(options.transparent ?? false),
  });
  material.colorNode = three.Fn(() => {
    const texel = sample(set.albedo);
    return three.vec4(
      three.sRGBTransferEOTF(texel.rgb).mul(three.vec3(
        baseColorFactor[0], baseColorFactor[1], baseColorFactor[2],
      )),
      texel.a.mul(baseColorFactor[3]),
    );
  })();
  if (set.normal) {
    material.normalNode = three.normalMap(
      sample(set.normal).xyz,
      three.vec2(normalScale[0], normalScale[1]),
    );
  }
  if (set.emissive) {
    material.emissiveNode = three.sRGBTransferEOTF(sample(set.emissive).rgb).mul(three.vec3(
      emissiveFactor[0], emissiveFactor[1], emissiveFactor[2],
    ));
  }
  if (set.masks) {
    // Independent reads avoid ordering assumptions between Three's roughness
    // and metalness node flows. Both resolve the same linked-material mip.
    material.roughnessNode = sample(set.masks).g.mul(roughnessFactor);
    material.metalnessNode = sample(set.masks).b.mul(metalnessFactor);
  } else {
    material.roughness = roughnessFactor;
    material.metalness = metalnessFactor;
  }

  const feedbackEntries = aligned ? [set.albedo] : entries;
  const feedbackMaterials = feedbackEntries.map(entry => {
    const feedbackMaterial = new three.MeshBasicNodeMaterial({ side });
    feedbackMaterial.fragmentNode = three.Fn(() => feedback({
      sampleUV: three.uv(), gradientUV: three.uv(),
      feedbackPixelScale: three.uniform(feedbackPixelScale),
      virtualSize: three.uniform(new three.Vector2(entry.width, entry.height)),
      pageGrid: three.uniform(new three.Vector2(entry.pageGridX, entry.pageGridY)),
      maxMip: three.float(entry.maxMip), qualityBias: three.float(qualityBias),
      addressMode: addressModeNode, textureId: three.uint(entry.textureId),
    }))();
    return feedbackMaterial;
  });
  return {
    material,
    feedbackMaterial: feedbackMaterials[0],
    feedbackMaterials,
    feedbackEntries,
  };
}
