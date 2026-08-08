import * as THREE from 'three/webgpu';
import type { FeedbackRenderable } from './virtual-texture-feedback-coordinator.ts';

export interface VirtualShaderBindingOptions {
  readonly scene: THREE.Scene;
  readonly camera: THREE.Camera;
  readonly root: THREE.Object3D;
  readonly mesh: THREE.Mesh;
  readonly visibleMaterial: THREE.Material;
  readonly feedbackNodes: readonly THREE.Node<'uvec4'>[];
  readonly maxLayers: number;
}

/**
 * Geometry-agnostic ownership for one arbitrary visible material and a fixed
 * list of VT feedback nodes. Layers may come from disk, RAM, or procedural
 * sources because feedback is expressed only in texture identities.
 */
export class VirtualShaderBinding implements FeedbackRenderable {
  readonly feedbackScene: THREE.Scene;
  readonly feedbackCamera: THREE.Camera;
  readonly feedbackPassCount: number;
  readonly feedbackMaterials: readonly THREE.MeshBasicNodeMaterial[];
  private rootWasVisible = true;
  private enabled = true;
  private disposed = false;

  constructor(private readonly options: Readonly<VirtualShaderBindingOptions>) {
    if (!Number.isInteger(options.maxLayers) || options.maxLayers < 1 ||
        options.feedbackNodes.length < 1 || options.feedbackNodes.length > options.maxLayers)
      throw new RangeError('virtual shader layer capacity exceeded');
    const materials: THREE.MeshBasicNodeMaterial[] = [];
    for (const node of options.feedbackNodes) {
      const material = new THREE.MeshBasicNodeMaterial({ side: options.visibleMaterial.side });
      material.fragmentNode = node;
      materials.push(material);
    }
    this.feedbackMaterials = materials;
    this.feedbackPassCount = materials.length;
    this.feedbackScene = options.scene;
    this.feedbackCamera = options.camera;
    const old = options.mesh.material;
    options.mesh.material = options.visibleMaterial;
    if (!Array.isArray(old) && old !== options.visibleMaterial) old.dispose();
  }

  /** @alloc-effect none */
  setFeedbackEnabled(enabled: boolean): void { this.enabled = enabled; }
  /** @alloc-effect none */
  isFeedbackActive(): boolean { return this.enabled && !this.disposed && this.options.root.visible; }
  /** @alloc-effect none */
  beginFeedbackPass(localPass: number): void {
    this.rootWasVisible = this.options.root.visible;
    this.options.root.visible = true;
    const material = this.feedbackMaterials[Math.min(localPass, this.feedbackMaterials.length - 1)];
    if (material) this.options.mesh.material = material;
  }
  /** @alloc-effect none */
  endFeedbackPass(_localPass: number): void {
    this.options.mesh.material = this.options.visibleMaterial;
    this.options.root.visible = this.rootWasVisible;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.options.mesh.visible = false;
    this.options.visibleMaterial.dispose();
    for (const material of this.feedbackMaterials) material.dispose();
  }
}
