import * as THREE from 'three';

const MAX_SAMPLE_SECONDS = 0.1;
const MAX_ANGULAR_STEP = Math.PI * 0.5;
const MAX_TRANSLATION_FAR_FRACTION = 0.25;

/** Fixed-state world-pose extrapolator for one VT feedback camera. */
export class PredictedFeedbackCamera {
  readonly camera: THREE.Camera;
  resetCount = 0;

  private readonly previousPosition = new THREE.Vector3();
  private readonly currentPosition = new THREE.Vector3();
  private readonly predictedPosition = new THREE.Vector3();
  private readonly previousQuaternion = new THREE.Quaternion();
  private readonly currentQuaternion = new THREE.Quaternion();
  private readonly predictedQuaternion = new THREE.Quaternion();
  private readonly currentScale = new THREE.Vector3(1, 1, 1);
  private lastSeconds = 0;
  private initialized = false;

  constructor(source: THREE.Camera, readonly horizonMs: number) {
    if (!Number.isFinite(horizonMs) || horizonMs <= 0)
      throw new RangeError('feedback prediction horizon must be positive');
    this.camera = source.clone(false);
    this.copyCurrentCamera(source);
  }

  /** @alloc-effect none */
  sample(source: THREE.Camera, elapsedSeconds: number): THREE.Camera {
    source.updateWorldMatrix(true, false);
    source.matrixWorld.decompose(this.currentPosition, this.currentQuaternion, this.currentScale);
    this.copyCurrentCamera(source);

    if (!this.initialized) {
      this.initialized = true;
      this.previousPosition.copy(this.currentPosition);
      this.previousQuaternion.copy(this.currentQuaternion);
      this.lastSeconds = elapsedSeconds;
      this.publishCurrentPose();
      return this.camera;
    }

    const dt = elapsedSeconds - this.lastSeconds;
    const far = Math.max(1, (source as THREE.PerspectiveCamera).far ?? 1);
    const translation = this.previousPosition.distanceTo(this.currentPosition);
    const rotation = this.previousQuaternion.angleTo(this.currentQuaternion);
    const reset = !Number.isFinite(dt) || dt <= 0 || dt > MAX_SAMPLE_SECONDS ||
      translation > far * MAX_TRANSLATION_FAR_FRACTION || rotation > MAX_ANGULAR_STEP;

    if (reset) {
      if (dt !== 0) this.resetCount++;
      this.publishCurrentPose();
    } else {
      const factor = 1 + this.horizonMs / (dt * 1000);
      this.predictedPosition.copy(this.previousPosition).lerp(this.currentPosition, factor);
      this.predictedQuaternion.slerpQuaternions(
        this.previousQuaternion, this.currentQuaternion, factor,
      );
      this.publishPose(this.predictedPosition, this.predictedQuaternion);
    }

    this.previousPosition.copy(this.currentPosition);
    this.previousQuaternion.copy(this.currentQuaternion);
    this.lastSeconds = elapsedSeconds;
    return this.camera;
  }

  /** @alloc-effect none */
  private copyCurrentCamera(source: THREE.Camera): void {
    this.camera.projectionMatrix.copy(source.projectionMatrix);
    this.camera.projectionMatrixInverse.copy(source.projectionMatrixInverse);
    this.camera.coordinateSystem = source.coordinateSystem;
    this.camera.layers.mask = source.layers.mask;
    const sourcePerspective = source as THREE.PerspectiveCamera;
    const targetPerspective = this.camera as THREE.PerspectiveCamera;
    if (sourcePerspective.isPerspectiveCamera === true && targetPerspective.isPerspectiveCamera === true) {
      targetPerspective.near = sourcePerspective.near;
      targetPerspective.far = sourcePerspective.far;
      targetPerspective.fov = sourcePerspective.fov;
      targetPerspective.aspect = sourcePerspective.aspect;
      targetPerspective.zoom = sourcePerspective.zoom;
      targetPerspective.focus = sourcePerspective.focus;
      targetPerspective.filmGauge = sourcePerspective.filmGauge;
      targetPerspective.filmOffset = sourcePerspective.filmOffset;
    }
    const sourceOrthographic = source as THREE.OrthographicCamera;
    const targetOrthographic = this.camera as THREE.OrthographicCamera;
    if (sourceOrthographic.isOrthographicCamera === true && targetOrthographic.isOrthographicCamera === true) {
      targetOrthographic.near = sourceOrthographic.near;
      targetOrthographic.far = sourceOrthographic.far;
      targetOrthographic.left = sourceOrthographic.left;
      targetOrthographic.right = sourceOrthographic.right;
      targetOrthographic.top = sourceOrthographic.top;
      targetOrthographic.bottom = sourceOrthographic.bottom;
      targetOrthographic.zoom = sourceOrthographic.zoom;
    }
    this.camera.parent = null;
    this.camera.matrixAutoUpdate = true;
    this.camera.matrixWorldAutoUpdate = true;
  }

  /** @alloc-effect none */
  private publishCurrentPose(): void {
    this.publishPose(this.currentPosition, this.currentQuaternion);
  }

  /** @alloc-effect none */
  private publishPose(position: THREE.Vector3, quaternion: THREE.Quaternion): void {
    this.camera.position.copy(position);
    this.camera.quaternion.copy(quaternion);
    this.camera.scale.copy(this.currentScale);
    this.camera.updateMatrix();
    this.camera.updateMatrixWorld(true);
  }
}
