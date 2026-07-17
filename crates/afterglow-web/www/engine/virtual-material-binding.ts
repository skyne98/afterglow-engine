import * as THREE from 'three/webgpu';
import * as TSL from 'three/tsl';
import type { FeedbackRenderable } from './virtual-texture-feedback-coordinator.ts';
import {
  createVirtualGltfMaterialPair,
  type VirtualGltfMaterialOptions,
  type VirtualGltfMaterialPair,
} from './virtual-texture-material.ts';
import type { VirtualMaterialSet, VirtualTextureStore } from './virtual-texture.ts';

export interface VirtualMaterialBindingOptions {
  scene: THREE.Scene;
  camera: THREE.Camera;
  root: THREE.Object3D;
  mesh: THREE.Mesh;
  store: VirtualTextureStore;
  set: VirtualMaterialSet;
  feedbackPixelScale: THREE.Vector2;
  material?: Readonly<VirtualGltfMaterialOptions>;
}

/** One prewarmed visible/feedback material pair for procedural geometry. */
export class VirtualMaterialBinding implements FeedbackRenderable {
  readonly feedbackScene: THREE.Scene;
  readonly feedbackCamera: THREE.Camera;
  readonly feedbackPassCount: number;
  readonly pair: VirtualGltfMaterialPair;
  private rootWasVisible = true;
  private disposed = false;
  private enabled = true;

  constructor(private readonly options: VirtualMaterialBindingOptions) {
    const runtime = Object.assign({}, THREE, TSL);
    this.pair = createVirtualGltfMaterialPair(
      runtime, options.store, options.set, options.feedbackPixelScale, options.material,
    );
    this.feedbackScene = options.scene;
    this.feedbackCamera = options.camera;
    this.feedbackPassCount = this.pair.feedbackMaterials.length;
    const source = options.mesh.material;
    if (Array.isArray(source)) throw new Error('virtual material binding requires one source material');
    options.mesh.material = this.pair.material;
    source.dispose();
  }

  /** @alloc-effect none */
  setFeedbackEnabled(enabled: boolean): void { this.enabled = enabled; }
  /** @alloc-effect none */
  isFeedbackActive(): boolean { return this.enabled && !this.disposed && this.options.root.visible; }
  /** @alloc-effect none */
  beginFeedbackPass(localPass: number): void {
    this.rootWasVisible = this.options.root.visible;
    this.options.root.visible = true;
    const index = Math.min(localPass, this.pair.feedbackMaterials.length - 1);
    const feedback = this.pair.feedbackMaterials[index];
    if (feedback) this.options.mesh.material = feedback;
  }
  /** @alloc-effect none */
  endFeedbackPass(_localPass: number): void {
    this.options.mesh.material = this.pair.material;
    this.options.root.visible = this.rootWasVisible;
  }
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.options.mesh.visible = false;
    this.pair.material.dispose();
    for (const feedback of this.pair.feedbackMaterials) feedback.dispose();
  }
}
