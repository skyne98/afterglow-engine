import * as THREE from 'three/webgpu';
import * as TSL from 'three/tsl';
import type { OptimizedGltfAsset, GltfMaterialTextureLayout } from './asset-store.ts';
import type { FeedbackRenderable } from './virtual-texture-feedback-coordinator.ts';
import {
  createVirtualGltfMaterialPair,
  VirtualTextureAddressMode,
  type VirtualGltfMaterialOptions,
  type VirtualGltfMaterialPair,
} from './virtual-texture-material.ts';
import type { VirtualMaterialSet, VirtualTextureEntry, VirtualTextureStore } from './virtual-texture.ts';

const textureProperties = [
  'map', 'normalMap', 'roughnessMap', 'metalnessMap', 'aoMap', 'emissiveMap',
] as const;

interface BindingRecord {
  mesh: THREE.Mesh;
  sourceMaterial: THREE.Material;
  pair: VirtualGltfMaterialPair | null;
  visible: boolean;
}

export interface VirtualGltfBindingOptions {
  primitiveCapacity: number;
  feedbackScene: THREE.Scene;
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

function materialOptions(source: THREE.Material, options: VirtualGltfBindingOptions): VirtualGltfMaterialOptions {
  if (!(source instanceof THREE.MeshStandardMaterial)) {
    return {
      addressMode: options.addressMode ?? VirtualTextureAddressMode.Repeat,
      qualityBias: options.qualityBias ?? 0,
      transparent: source.transparent,
      depthWrite: source.depthWrite,
      side: source.side,
    };
  }
  return {
    addressMode: options.addressMode ?? VirtualTextureAddressMode.Repeat,
    qualityBias: options.qualityBias ?? 0,
    baseColorFactor: [source.color.r, source.color.g, source.color.b, source.opacity],
    roughnessFactor: source.roughness,
    metalnessFactor: source.metalness,
    normalScale: [source.normalScale.x, source.normalScale.y],
    emissiveFactor: [source.emissive.r, source.emissive.g, source.emissive.b],
    transparent: source.transparent,
    depthWrite: source.depthWrite,
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
  private disposed = false;

  private constructor(
    asset: OptimizedGltfAsset,
    scene: THREE.Scene,
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
        if (materialIndex === undefined)
          throw new Error(`glTF material has no stable parser index: ${source.name}`);
        const layout = layouts[materialIndex];
        if (!layout) throw new Error(`glTF material layout ${materialIndex} is unavailable`);
        let pair = pairs[materialIndex] ?? null;
        if (!pair && layout.baseColorImage !== null) {
          const set = imageSet(layout, options.resolveImage);
          if (!set) throw new Error(`glTF material ${materialIndex} lost its base-color image`);
          pair = pairFactory(store, set, options.feedbackPixelScale, materialOptions(source, options));
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
    for (const source of sources) {
      if (!(source instanceof THREE.MeshStandardMaterial)) continue;
      for (const property of textureProperties) {
        const texture = source[property];
        if (texture) importedTextures.add(texture);
      }
    }
    for (const texture of importedTextures) {
      texture.dispose();
      const data = texture.source.data;
      if (typeof ImageBitmap !== 'undefined' && data instanceof ImageBitmap) data.close();
    }
    for (const source of sources) source?.dispose();

    return new VirtualGltfBinding(
      asset, options.feedbackScene, records, recordCount, pairs, passCount, options.feedbackCamera,
    );
  }

  isFeedbackActive(): boolean { return !this.disposed && this.feedbackScene.visible; }

  beginFeedbackPass(localPass: number): void {
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

  endFeedbackPass(): void {
    for (let index = 0; index < this.recordCount; index++) {
      const record = this.records[index];
      if (!record) continue;
      record.mesh.visible = record.visible;
      if (record.pair) record.mesh.material = record.pair.material;
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const pair of this.pairs) {
      pair?.material.dispose();
      for (const feedback of pair?.feedbackMaterials ?? []) feedback.dispose();
    }
    for (let index = 0; index < this.records.length; index++) this.records[index] = null;
    this.recordCount = 0;
  }
}
