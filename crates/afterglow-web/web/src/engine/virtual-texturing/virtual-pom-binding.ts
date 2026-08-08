import * as THREE from 'three/webgpu';
import * as TSL from 'three/tsl';
import type { FeedbackRenderable } from './virtual-texture-feedback-coordinator.ts';
import {
  createVirtualPomMaterialPair,
  type VirtualPomMaterialOptions,
  type VirtualPomMaterialPair,
} from './virtual-texture-material.ts';
import {
  RESOLVE_VIRTUAL_MATERIAL,
  type VirtualTextureMaterialSet,
  type VirtualTextureSystem,
} from './virtual-texture-system.ts';

interface PomRecord {
  readonly source: THREE.Mesh;
  readonly feedback: THREE.Mesh;
  readonly pair: VirtualPomMaterialPair;
}

type VirtualPomPairFactory = (
  textures: VirtualTextureSystem,
  set: Readonly<VirtualTextureMaterialSet>,
  heightTexture: THREE.Texture,
  feedbackPixelScale: THREE.Vector2,
  options?: Readonly<VirtualPomMaterialOptions>,
) => VirtualPomMaterialPair;

export interface VirtualPomSceneBindingOptions {
  camera: THREE.Camera;
  textures: VirtualTextureSystem;
  feedbackPixelScale: THREE.Vector2;
  capacity: number;
  material?: Readonly<VirtualPomMaterialOptions>;
  /** Test/tool injection; production uses the engine POM factory. */
  createPair?: VirtualPomPairFactory;
}

/** Fixed-capacity POM/base material and feedback owner for static meshes. */
export class VirtualPomSceneBinding implements FeedbackRenderable {
  readonly feedbackScene = new THREE.Scene();
  readonly feedbackCamera: THREE.Camera;
  readonly feedbackPassCount = 1;
  private readonly records: Array<PomRecord | null>;
  private count = 0;
  private pomEnabled = true;
  private feedbackEnabled = true;
  private sealed = false;
  private disposed = false;

  constructor(private readonly options: VirtualPomSceneBindingOptions) {
    if (!Number.isInteger(options.capacity) || options.capacity <= 0)
      throw new RangeError('POM binding capacity must be positive');
    this.feedbackCamera = options.camera;
    this.records = new Array<PomRecord | null>(options.capacity).fill(null);
  }

  add(
    mesh: THREE.Mesh,
    set: Readonly<VirtualTextureMaterialSet>,
    heightTexture: THREE.Texture,
  ): VirtualPomMaterialPair {
    if (this.sealed || this.disposed) throw new Error('cannot add to a sealed POM binding');
    if (this.count >= this.records.length) throw new RangeError('POM binding capacity exceeded');
    const source = mesh.material;
    if (Array.isArray(source)) throw new Error('POM binding requires one source material');
    const pairFactory = this.options.createPair ?? ((textures, handles, height, pixelScale, material) => {
      const runtime = Object.assign({}, THREE, TSL);
      const resolved = textures[RESOLVE_VIRTUAL_MATERIAL](handles);
      return createVirtualPomMaterialPair(
        runtime, resolved.store, resolved.set, height, pixelScale, material,
      );
    });
    const pair = pairFactory(
      this.options.textures, set, heightTexture,
      this.options.feedbackPixelScale, this.options.material,
    );
    const feedback = new THREE.Mesh(mesh.geometry, pair.pomFeedbackMaterial);
    feedback.position.copy(mesh.position);
    feedback.quaternion.copy(mesh.quaternion);
    feedback.scale.copy(mesh.scale);
    feedback.matrixAutoUpdate = mesh.matrixAutoUpdate;
    feedback.name = mesh.name;
    mesh.material = pair.pomMaterial;
    this.feedbackScene.add(feedback);
    this.records[this.count++] = { source: mesh, feedback, pair };
    return pair;
  }

  seal(): void { this.sealed = true; }
  /** @alloc-effect none */
  setPomEnabled(enabled: boolean): void {
    this.pomEnabled = enabled;
    for (let index = 0; index < this.count; index++) {
      const record = this.records[index];
      if (!record) continue;
      record.source.material = enabled ? record.pair.pomMaterial : record.pair.baseMaterial;
      record.feedback.material = enabled ? record.pair.pomFeedbackMaterial : record.pair.baseFeedbackMaterial;
    }
  }
  /** @alloc-effect none */
  setFeedbackEnabled(enabled: boolean): void { this.feedbackEnabled = enabled; }
  /** @alloc-effect none */
  isFeedbackActive(): boolean { return this.feedbackEnabled && !this.disposed; }
  /** @alloc-effect none */
  beginFeedbackPass(_localPass: number): void {}
  /** @alloc-effect none */
  endFeedbackPass(_localPass: number): void {}
  /** @alloc-effect none */
  isPomEnabled(): boolean { return this.pomEnabled; }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (let index = this.count - 1; index >= 0; index--) {
      const record = this.records[index];
      if (!record) continue;
      record.source.visible = false;
      this.feedbackScene.remove(record.feedback);
      record.pair.baseMaterial.dispose();
      record.pair.pomMaterial.dispose();
      record.pair.baseFeedbackMaterial.dispose();
      record.pair.pomFeedbackMaterial.dispose();
      this.records[index] = null;
    }
    this.count = 0;
  }
}
