import * as THREE from 'three/webgpu';
import * as TSL from 'three/tsl';
import type {
  OptimizedGltfAsset, GltfMaterialTextureLayout, GltfTextureSamplingLayout,
} from '../assets/asset-store.ts';
import type { FeedbackRenderable } from './virtual-texture-feedback-coordinator.ts';
import {
  createVirtualGltfMaterialPair,
  VirtualTextureAddressMode,
  type VirtualGltfMaterialOptions,
  type VirtualGltfMaterialPair,
  type VirtualGltfTextureSampling,
  type VirtualTextureSampling,
} from './virtual-texture-material.ts';
import type { VirtualMaterialSet, VirtualTextureEntry, VirtualTextureStore } from './virtual-texture.ts';

function collectMaterialTextures(material: THREE.Material, output: Set<THREE.Texture>): void {
  if (material instanceof THREE.MeshStandardMaterial) {
    for (const texture of [
      material.map, material.normalMap, material.roughnessMap, material.metalnessMap,
      material.aoMap, material.emissiveMap, material.alphaMap, material.lightMap,
    ]) if (texture) output.add(texture);
  } else if (material instanceof THREE.MeshBasicMaterial) {
    for (const texture of [material.map, material.alphaMap, material.aoMap, material.lightMap])
      if (texture) output.add(texture);
  }
}

interface BindingRecord {
  mesh: THREE.Mesh;
  sourceMaterial: THREE.Material;
  pair: VirtualGltfMaterialPair | null;
  visible: boolean;
}

export interface VirtualGltfBindingOptions {
  primitiveCapacity: number;
  feedbackScene: THREE.Scene;
  feedbackRoot: THREE.Object3D;
  /** Roots hidden while this binding renders feedback in a shared scene. */
  exclusiveRoots?: readonly THREE.Object3D[];
  feedbackCamera: THREE.Camera;
  feedbackPixelScale: THREE.Vector2;
  resolveImage(imageIndex: number): VirtualTextureEntry | undefined;
  addressMode?: VirtualTextureAddressMode;
  qualityBias?: number;
  pairFactory?: (
    store: VirtualTextureStore,
    set: VirtualMaterialSet,
    pixelScale: THREE.Vector2,
    options: Readonly<VirtualGltfMaterialOptions>,
  ) => VirtualGltfMaterialPair;
}

function textureSampling(
  layout: GltfTextureSamplingLayout | null,
  override: VirtualTextureAddressMode | undefined,
): VirtualTextureSampling | undefined {
  if (!layout) return undefined;
  if (layout.wrapS !== layout.wrapT)
    throw new Error('virtual glTF textures require identical S/T address modes');
  let filterMode = 0;
  if (layout.magFilter === 9728 && layout.minFilter === 9728) filterMode = 1;
  else if (layout.magFilter !== 9729 || ![9729, 9985, 9987].includes(layout.minFilter))
    throw new Error('virtual glTF texture uses an unsupported mixed filter mode');
  let addressMode = VirtualTextureAddressMode.Repeat;
  if (layout.wrapS === 33071) addressMode = VirtualTextureAddressMode.Clamp;
  else if (layout.wrapS === 33648) addressMode = VirtualTextureAddressMode.MirrorRepeat;
  else if (layout.wrapS !== 10497)
    throw new Error(`unsupported virtual glTF wrapping ${layout.wrapS}`);
  const cosine = Math.cos(layout.rotation), sine = Math.sin(layout.rotation);
  return {
    channel: layout.texCoord,
    matrix: [
      layout.scale[0] * cosine, layout.scale[0] * sine, 0,
      -layout.scale[1] * sine, layout.scale[1] * cosine, 0,
      layout.offset[0], layout.offset[1], 1,
    ],
    addressMode: override ?? addressMode,
    filterMode,
  };
}

function materialOptions(
  source: THREE.MeshStandardMaterial,
  layout: GltfMaterialTextureLayout,
  options: VirtualGltfBindingOptions,
): VirtualGltfMaterialOptions {
  const sampling: VirtualGltfTextureSampling = {};
  const albedo = textureSampling(layout.baseColorSampling, options.addressMode);
  const normal = textureSampling(layout.normalSampling, options.addressMode);
  const masks = textureSampling(layout.metallicRoughnessSampling, options.addressMode);
  const emissive = textureSampling(layout.emissiveSampling, options.addressMode);
  if (albedo) sampling.albedo = albedo;
  if (normal) sampling.normal = normal;
  if (masks) sampling.masks = masks;
  if (emissive) sampling.emissive = emissive;
  const physical = source instanceof THREE.MeshPhysicalMaterial ? source : null;
  return {
    addressMode: options.addressMode ?? VirtualTextureAddressMode.Repeat,
    qualityBias: options.qualityBias ?? 0,
    sampling,
    baseColorFactor: [source.color.r, source.color.g, source.color.b, source.opacity],
    roughnessFactor: source.roughness,
    metalnessFactor: source.metalness,
    normalScale: [source.normalScale.x, source.normalScale.y],
    emissiveFactor: [source.emissive.r, source.emissive.g, source.emissive.b],
    ...(physical ? {
      transmissionFactor: physical.transmission,
      thicknessFactor: physical.thickness,
      ior: physical.ior,
    } : {}),
    transparent: source.transparent,
    alphaTest: source.alphaTest,
    depthWrite: source.depthWrite,
    depthTest: source.depthTest,
    blending: source.blending,
    side: source.side,
  };
}

function imageSet(
  layout: GltfMaterialTextureLayout,
  resolve: (imageIndex: number) => VirtualTextureEntry | undefined,
): VirtualMaterialSet | null {
  if (layout.baseColorImage === null) return null;
  const albedo = resolve(layout.baseColorImage);
  if (!albedo) throw new Error(`virtual glTF image ${layout.baseColorImage} is unavailable`);
  const optional = (index: number | null): VirtualTextureEntry | undefined =>
    index === null ? undefined : resolve(index);
  const set: VirtualMaterialSet = { albedo };
  const normal = optional(layout.normalImage);
  const masks = optional(layout.metallicRoughnessImage);
  const emissive = optional(layout.emissiveImage);
  if (normal) set.normal = normal;
  if (masks) set.masks = masks;
  if (emissive) set.emissive = emissive;
  return set;
}

/** Stable-index glTF material replacement and exact feedback-state owner. */
export class VirtualGltfBinding implements FeedbackRenderable {
  readonly feedbackScene: THREE.Scene;
  readonly feedbackPassCount: number;
  readonly feedbackCamera: THREE.Camera;

  private readonly records: Array<BindingRecord | null>;
  private readonly pairs: Array<VirtualGltfMaterialPair | null>;
  private recordCount = 0;
  private enabled = true;
  private rootWasVisible = true;
  private disposed = false;

  private constructor(
    scene: THREE.Scene,
    private readonly feedbackRoot: THREE.Object3D,
    private readonly exclusiveRoots: readonly THREE.Object3D[],
    private readonly exclusiveVisibility: Uint8Array,
    records: Array<BindingRecord | null>,
    recordCount: number,
    pairs: Array<VirtualGltfMaterialPair | null>,
    passCount: number,
    camera: THREE.Camera,
  ) {
    this.feedbackScene = scene;
    this.feedbackCamera = camera;
    this.records = records;
    this.recordCount = recordCount;
    this.pairs = pairs;
    this.feedbackPassCount = passCount;
  }

  static create(
    asset: OptimizedGltfAsset,
    store: VirtualTextureStore,
    options: VirtualGltfBindingOptions,
  ): VirtualGltfBinding {
    if (!Number.isInteger(options.primitiveCapacity) || options.primitiveCapacity <= 0)
      throw new RangeError('virtual glTF primitive capacity must be positive');
    const records = new Array<BindingRecord | null>(options.primitiveCapacity).fill(null);
    const layouts: Array<GltfMaterialTextureLayout | null> = new Array(asset.materialTextures.length).fill(null);
    for (const layout of asset.materialTextures) {
      if (!Number.isInteger(layout.index) || layout.index < 0 || layout.index >= layouts.length)
        throw new RangeError(`invalid glTF material index ${layout.index}`);
      if (layouts[layout.index] !== null) throw new Error(`duplicate glTF material index ${layout.index}`);
      layouts[layout.index] = layout;
    }
    const pairs: Array<VirtualGltfMaterialPair | null> = new Array(layouts.length).fill(null);
    const sources: Array<THREE.Material | null> = new Array(layouts.length).fill(null);
    let pairFactory = options.pairFactory;
    if (!pairFactory) {
      const runtime = Object.assign({}, THREE, TSL);
      pairFactory = (targetStore, set, pixelScale, pairOptions) =>
        createVirtualGltfMaterialPair(runtime, targetStore, set, pixelScale, pairOptions);
    }
    let recordCount = 0;
    let passCount = 1;
    try {
      asset.scene.traverse((object) => {
        if (!(object instanceof THREE.Mesh)) return;
        if (recordCount === records.length)
          throw new RangeError('virtual glTF primitive capacity exceeded');
        if (Array.isArray(object.material))
          throw new Error('virtual glTF binding requires one material per primitive');
        const source = object.material;
        const materialIndex = asset.materialIndices.get(source);
        if (materialIndex === undefined) {
          if (asset.materialIndices.size === 0 && asset.materialTextures.length === 0) {
            records[recordCount++] = { mesh: object, sourceMaterial: source, pair: null, visible: object.visible };
            return;
          }
          throw new Error(`glTF material has no stable parser index: ${source.name}`);
        }
        const layout = layouts[materialIndex];
        if (!layout) throw new Error(`glTF material layout ${materialIndex} is unavailable`);
        let pair = pairs[materialIndex] ?? null;
        if (!pair && layout.baseColorImage !== null) {
          if (!(source instanceof THREE.MeshStandardMaterial))
            throw new Error(`virtual glTF material ${materialIndex} is not a standard PBR material`);
          const set = imageSet(layout, options.resolveImage);
          if (!set) throw new Error(`glTF material ${materialIndex} lost its base-color image`);
          pair = pairFactory(store, set, options.feedbackPixelScale, materialOptions(source, layout, options));
          pairs[materialIndex] = pair;
          sources[materialIndex] = source;
          passCount = Math.max(passCount, pair.feedbackMaterials.length);
        }
        records[recordCount++] = { mesh: object, sourceMaterial: source, pair, visible: object.visible };
        if (pair) object.material = pair.material;
      });
    } catch (error) {
      for (let index = 0; index < recordCount; index++) {
        const record = records[index];
        if (record) record.mesh.material = record.sourceMaterial;
      }
      for (const pair of pairs) {
        pair?.material.dispose();
        for (const feedback of pair?.feedbackMaterials ?? []) feedback.dispose();
      }
      throw error;
    }

    const importedTextures = new Set<THREE.Texture>();
    const retainedTextures = new Set<THREE.Texture>();
    for (const source of sources) if (source) collectMaterialTextures(source, importedTextures);
    for (let index = 0; index < recordCount; index++) {
      const record = records[index];
      if (record && !record.pair) collectMaterialTextures(record.sourceMaterial, retainedTextures);
    }
    for (const texture of importedTextures) {
      if (retainedTextures.has(texture)) continue;
      texture.dispose();
      const data = texture.source.data;
      if (typeof ImageBitmap !== 'undefined' && data instanceof ImageBitmap) data.close();
    }
    for (const source of sources) source?.dispose();

    const exclusiveRoots = options.exclusiveRoots ? Array.from(options.exclusiveRoots) : [];
    return new VirtualGltfBinding(
      options.feedbackScene, options.feedbackRoot, exclusiveRoots,
      new Uint8Array(exclusiveRoots.length), records, recordCount, pairs, passCount,
      options.feedbackCamera,
    );
  }

  /** @alloc-effect none */
  setFeedbackEnabled(enabled: boolean): void { this.enabled = enabled; }

  /** @alloc-effect none */
  isFeedbackActive(): boolean {
    return this.enabled && !this.disposed && this.feedbackScene.visible && this.feedbackRoot.visible;
  }

  /** @alloc-effect none */
  beginFeedbackPass(localPass: number): void {
    this.rootWasVisible = this.feedbackRoot.visible;
    this.feedbackRoot.visible = true;
    for (let index = 0; index < this.exclusiveRoots.length; index++) {
      const root = this.exclusiveRoots[index];
      if (!root) continue;
      this.exclusiveVisibility[index] = root.visible ? 1 : 0;
      root.visible = false;
    }
    for (let index = 0; index < this.recordCount; index++) {
      const record = this.records[index];
      if (!record) continue;
      record.visible = record.mesh.visible;
      if (!record.pair) {
        record.mesh.visible = false;
        continue;
      }
      const feedbackIndex = Math.min(localPass, record.pair.feedbackMaterials.length - 1);
      const feedback = record.pair.feedbackMaterials[feedbackIndex];
      if (feedback) record.mesh.material = feedback;
    }
  }

  /** @alloc-effect none */
  endFeedbackPass(_localPass: number): void {
    for (let index = 0; index < this.recordCount; index++) {
      const record = this.records[index];
      if (!record) continue;
      record.mesh.visible = record.visible;
      if (record.pair) record.mesh.material = record.pair.material;
    }
    for (let index = 0; index < this.exclusiveRoots.length; index++) {
      const root = this.exclusiveRoots[index];
      if (root) root.visible = this.exclusiveVisibility[index] !== 0;
    }
    this.feedbackRoot.visible = this.rootWasVisible;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (let index = 0; index < this.recordCount; index++) {
      const record = this.records[index];
      if (record?.pair) record.mesh.visible = false;
    }
    for (const pair of this.pairs) {
      pair?.material.dispose();
      for (const feedback of pair?.feedbackMaterials ?? []) feedback.dispose();
    }
    for (let index = 0; index < this.records.length; index++) this.records[index] = null;
    this.recordCount = 0;
  }
}
