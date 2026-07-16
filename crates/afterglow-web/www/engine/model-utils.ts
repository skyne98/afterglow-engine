import * as THREE from 'three/webgpu';

export const enum ModelCollectionStatus {
  Complete = 0,
  CapacityExceeded = 1,
}

/** Fixed-capacity primitive collection with one retained traversal callback. */
export class ModelPrimitives {
  readonly items: Array<THREE.Mesh | null>;
  count = 0;
  private overflow = false;
  private readonly collectObject: (object: THREE.Object3D) => void;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError('model primitive capacity must be positive');
    this.items = new Array<THREE.Mesh | null>(capacity).fill(null);
    this.collectObject = (object: THREE.Object3D): void => {
      if (!(object instanceof THREE.Mesh)) return;
      if (this.count === this.capacity) { this.overflow = true; return; }
      this.items[this.count++] = object;
    };
  }

  collect(root: THREE.Object3D): ModelCollectionStatus {
    for (let index = 0; index < this.count; index++) this.items[index] = null;
    this.count = 0;
    this.overflow = false;
    root.traverse(this.collectObject);
    return this.overflow ? ModelCollectionStatus.CapacityExceeded : ModelCollectionStatus.Complete;
  }
}

/** Exact deformed bounds for static, skinned, and morphed Mesh primitives. */
export function computeDeformedBoundsInto(
  primitives: ModelPrimitives,
  out: THREE.Box3,
  vertexScratch: THREE.Vector3,
): THREE.Box3 {
  out.makeEmpty();
  for (let primitiveIndex = 0; primitiveIndex < primitives.count; primitiveIndex++) {
    const mesh = primitives.items[primitiveIndex];
    if (!mesh) continue;
    const positions = mesh.geometry.getAttribute('position');
    if (!positions) continue;
    for (let vertexIndex = 0; vertexIndex < positions.count; vertexIndex++) {
      mesh.getVertexPosition(vertexIndex, vertexScratch);
      vertexScratch.applyMatrix4(mesh.matrixWorld);
      out.expandByPoint(vertexScratch);
    }
  }
  return out;
}

export const enum ModelNormalizationStatus {
  Normalized = 0,
  EmptyBounds = 1,
  InvalidTargetHeight = 2,
}

/** Scale, center X/Z, and ground Y on an engine-owned presentation pivot. */
export function normalizeModelPivot(
  pivot: THREE.Object3D,
  targetHeight: number,
  boundsScratch: THREE.Box3,
  sizeScratch: THREE.Vector3,
  centerScratch: THREE.Vector3,
): ModelNormalizationStatus {
  if (!Number.isFinite(targetHeight) || targetHeight <= 0)
    return ModelNormalizationStatus.InvalidTargetHeight;
  boundsScratch.setFromObject(pivot);
  if (boundsScratch.isEmpty()) return ModelNormalizationStatus.EmptyBounds;
  boundsScratch.getSize(sizeScratch);
  if (!(sizeScratch.y > 0)) return ModelNormalizationStatus.EmptyBounds;
  pivot.scale.multiplyScalar(targetHeight / sizeScratch.y);
  boundsScratch.setFromObject(pivot);
  boundsScratch.getCenter(centerScratch);
  pivot.position.x -= centerScratch.x;
  pivot.position.y -= boundsScratch.min.y;
  pivot.position.z -= centerScratch.z;
  pivot.updateMatrixWorld(true);
  return ModelNormalizationStatus.Normalized;
}

/** Ground the current animated/deformed pose without changing X/Z presentation. */
export function groundDeformedModel(
  pivot: THREE.Object3D,
  primitives: ModelPrimitives,
  boundsScratch: THREE.Box3,
  vertexScratch: THREE.Vector3,
): boolean {
  pivot.updateMatrixWorld(true);
  computeDeformedBoundsInto(primitives, boundsScratch, vertexScratch);
  if (boundsScratch.isEmpty()) return false;
  pivot.position.y -= boundsScratch.min.y;
  pivot.updateMatrixWorld(true);
  return true;
}

/** Fixed clip/action set. Every action is created during bootstrap. */
export class AnimationSet {
  readonly mixer: THREE.AnimationMixer;
  private readonly actions: Array<THREE.AnimationAction | null>;
  private activeIndex = -1;
  private enabled = true;
  private disposed = false;

  constructor(
    private readonly root: THREE.Object3D,
    clips: readonly THREE.AnimationClip[],
    readonly capacity: number,
  ) {
    if (!Number.isInteger(capacity) || capacity < 0)
      throw new RangeError('animation capacity must be a non-negative integer');
    if (clips.length > capacity)
      throw new RangeError(`animation clip count ${clips.length} exceeds capacity ${capacity}`);
    this.mixer = new THREE.AnimationMixer(root);
    this.actions = new Array<THREE.AnimationAction | null>(capacity).fill(null);
    for (let index = 0; index < clips.length; index++) {
      const clip = clips[index];
      if (clip) this.actions[index] = this.mixer.clipAction(clip);
    }
  }

  get activeClip(): number { return this.activeIndex; }
  get isEnabled(): boolean { return this.enabled; }

  play(index: number): boolean {
    if (this.disposed || !Number.isInteger(index) || index < 0 || index >= this.actions.length)
      return false;
    const next = this.actions[index];
    if (!next) return false;
    if (this.activeIndex >= 0) this.actions[this.activeIndex]?.stop();
    next.reset().play();
    next.paused = !this.enabled;
    this.activeIndex = index;
    return true;
  }

  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    if (this.activeIndex >= 0) {
      const action = this.actions[this.activeIndex];
      if (action) action.paused = !enabled;
    }
  }

  update(deltaSeconds: number): void {
    if (!this.disposed && this.enabled && this.activeIndex >= 0)
      this.mixer.update(deltaSeconds);
  }

  setTime(seconds: number): void {
    if (!this.disposed && this.activeIndex >= 0) this.mixer.setTime(seconds);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.mixer.stopAllAction();
    this.mixer.uncacheRoot(this.root);
    for (let index = 0; index < this.actions.length; index++) this.actions[index] = null;
    this.activeIndex = -1;
  }
}

/** Disposable scene-owned skeleton visualization. */
export class SkeletonDebugAdapter {
  readonly helper: THREE.SkeletonHelper;
  private disposed = false;

  constructor(private readonly scene: THREE.Scene, root: THREE.Object3D) {
    this.helper = new THREE.SkeletonHelper(root);
    this.helper.visible = false;
    scene.add(this.helper);
  }

  setVisible(visible: boolean): void {
    if (!this.disposed) this.helper.visible = visible;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.scene.remove(this.helper);
    this.helper.dispose();
  }
}
