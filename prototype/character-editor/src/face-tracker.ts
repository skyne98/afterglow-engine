/**
 * Webcam face tracking for the character editor.
 *
 * Uses MediaPipe Face Landmarker (tasks-vision) in VIDEO mode to predict the
 * 52 ARKit blendshape coefficients from a webcam, then maps those
 * coefficients to the character's glTF morph targets by name.
 *
 * The mapping, dead-shape handling, and smoothing logic are pure and unit
 * tested. The WebcamFaceTracker class owns the browser-only parts (webcam
 * capture, MediaPipe wasm, and the per-frame detect loop).
 */

/** The 52 ARKit blendshape names in canonical order. */
export const ARKIT_BLENDSHAPES: readonly string[] = [
  'browDownLeft', 'browDownRight', 'browInnerUp',
  'browOuterUpLeft', 'browOuterUpRight',
  'cheekPuff', 'cheekSquintLeft', 'cheekSquintRight',
  'eyeBlinkLeft', 'eyeBlinkRight',
  'eyeLookDownLeft', 'eyeLookDownRight',
  'eyeLookInLeft', 'eyeLookInRight',
  'eyeLookOutLeft', 'eyeLookOutRight',
  'eyeLookUpLeft', 'eyeLookUpRight',
  'eyeSquintLeft', 'eyeSquintRight',
  'eyeWideLeft', 'eyeWideRight',
  'jawForward', 'jawLeft', 'jawOpen', 'jawRight',
  'mouthClose', 'mouthDimpleLeft', 'mouthDimpleRight',
  'mouthFrownLeft', 'mouthFrownRight',
  'mouthFunnel', 'mouthLeft',
  'mouthLowerDownLeft', 'mouthLowerDownRight',
  'mouthPressLeft', 'mouthPressRight',
  'mouthPucker', 'mouthRight',
  'mouthRollLower', 'mouthRollUpper',
  'mouthShrugLower', 'mouthShrugUpper',
  'mouthSmileLeft', 'mouthSmileRight',
  'mouthStretchLeft', 'mouthStretchRight',
  'mouthUpperUpLeft', 'mouthUpperUpRight',
  'noseSneerLeft', 'noseSneerRight', 'tongueOut',
];

/**
 * Blendshapes the MediaPipe blendshape model cannot signal reliably.
 * Community testing (github issues #4403, #5329; face-mesh-to-blendshapes)
 * reports these as effectively dead: the model never moves them. Forcing them
 * to zero keeps the character neutral instead of drifting.
 */
export const DEAD_BLENDSHAPES: ReadonlySet<string> = new Set([
  'jawForward', 'jawLeft', 'jawRight',
  'mouthDimpleLeft', 'mouthDimpleRight',
  'cheekPuff', 'tongueOut',
]);

const ARKIT_INDEX_BY_NAME: ReadonlyMap<string, number> = new Map(
  ARKIT_BLENDSHAPES.map((name, index) => [name, index]),
);

/** Precomputed name-to-index table for one character's morph list. */
export interface BlendshapeMapping {
  /** glTF morph index for each ARKit category index; -1 when the rig lacks it. */
  morphIndexByCategory: Int16Array;
  /** 1 when the ARKit category is a dead shape that must stay zero. */
  deadByCategory: Uint8Array;
  /** Number of ARKit shapes the rig actually owns. */
  count: number;
}

/** Build the category-to-morph table for a character's morph names. */
export function buildBlendshapeMapping(morphNames: readonly string[]): BlendshapeMapping {
  const indexByName = new Map<string, number>();
  for (let i = 0; i < morphNames.length; i++) indexByName.set(morphNames[i], i);
  const morphIndexByCategory = new Int16Array(ARKIT_BLENDSHAPES.length);
  const deadByCategory = new Uint8Array(ARKIT_BLENDSHAPES.length);
  let count = 0;
  for (let c = 0; c < ARKIT_BLENDSHAPES.length; c++) {
    const name = ARKIT_BLENDSHAPES[c];
    const morphIndex = indexByName.get(name);
    morphIndexByCategory[c] = morphIndex === undefined ? -1 : morphIndex;
    deadByCategory[c] = DEAD_BLENDSHAPES.has(name) ? 1 : 0;
    if (morphIndex !== undefined) count++;
  }
  return { morphIndexByCategory, deadByCategory, count };
}

export interface BlendshapeCategory {
  categoryName: string;
  score: number;
}

/**
 * Fixed-size one-pole smoother for blendshape coefficients.
 * Values below `snap` are forced to zero (MediaPipe emits small noise on
 * neutral faces). No allocation after construction.
 */
export class BlendshapeSmoother {
  private readonly last = new Float32Array(ARKIT_BLENDSHAPES.length);
  private readonly seen = new Uint8Array(ARKIT_BLENDSHAPES.length);

  constructor(
    private readonly alpha = 0.55,
    private readonly snap = 0.012,
  ) {}

  reset(): void {
    this.last.fill(0);
    this.seen.fill(0);
  }

  /** Smooth one coefficient; pass the ARKit category index. */
  smooth(categoryIndex: number, target: number): number {
    if (target < this.snap) {
      this.last[categoryIndex] = 0;
      this.seen[categoryIndex] = 1;
      return 0;
    }
    const previous = this.seen[categoryIndex] ? this.last[categoryIndex] : target;
    const value = previous + (target - previous) * this.alpha;
    this.last[categoryIndex] = value;
    this.seen[categoryIndex] = 1;
    return value;
  }
}

/**
 * Apply MediaPipe blendshape categories to glTF morph influences.
 * Dead shapes are forced to zero. Returns the number of morph writes.
 */
export function applyBlendshapes(
  mapping: BlendshapeMapping,
  categories: readonly BlendshapeCategory[],
  apply: (morphIndex: number, value: number) => boolean,
  smoother?: BlendshapeSmoother,
): number {
  let applied = 0;
  for (const category of categories) {
    const categoryIndex = ARKIT_INDEX_BY_NAME.get(category.categoryName);
    if (categoryIndex === undefined) continue;
    const morphIndex = mapping.morphIndexByCategory[categoryIndex];
    if (morphIndex < 0) continue;
    const target = mapping.deadByCategory[categoryIndex] ? 0 : Math.min(Math.max(category.score, 0), 1);
    const value = smoother ? smoother.smooth(categoryIndex, target) : target;
    if (apply(morphIndex, value)) applied++;
  }
  return applied;
}

/** Result of one tracked frame. */
export interface FaceTrackFrame {
  /** True when a face was detected this frame. */
  faceDetected: boolean;
  /** Morph writes applied this frame. */
  applied: number;
  /** Frames per second of the detect loop. */
  fps: number;
}

export interface FaceTrackerCallbacks {
  /** Called after every detect with the frame summary. */
  onFrame?: (frame: FaceTrackFrame) => void;
  /** Called with a user-readable status message. */
  onStatus?: (message: string) => void;
  /** Apply one morph coefficient to the character. */
  applyMorph: (morphIndex: number, value: number) => boolean;
}

/**
 * Browser-side webcam tracker. Owns the MediaPipe Face Landmarker and the
 * requestAnimationFrame detect loop. Starts and stops cleanly.
 */
export class WebcamFaceTracker {
  private landmarker?: import('@mediapipe/tasks-vision').FaceLandmarker;
  private video?: HTMLVideoElement;
  private stream?: MediaStream;
  private raf = 0;
  private running = false;
  private lastTimestamp = 0;
  private lastFrameTime = 0;
  private frameCount = 0;
  private fps = 0;
  private readonly mapping: BlendshapeMapping;
  private readonly smoother = new BlendshapeSmoother();

  /** True while the tracker owns the camera. */
  get active(): boolean {
    return this.running;
  }

  /** Number of the character's morph targets the tracker can drive. */
  get mappedCount(): number {
    return this.mapping.count;
  }

  constructor(
    private readonly morphNames: readonly string[],
    private readonly callbacks: FaceTrackerCallbacks,
    private readonly previewContainer?: HTMLElement,
  ) {
    this.mapping = buildBlendshapeMapping(morphNames);
  }

  async start(): Promise<void> {
    if (this.running) return;
    const onStatus = this.callbacks.onStatus;
    onStatus?.('Requesting webcam…');
    const stream = await navigator.mediaDevices.getUserMedia({
      video: { width: { ideal: 640 }, height: { ideal: 480 }, facingMode: 'user' },
      audio: false,
    });
    this.stream = stream;
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.autoplay = true;
    video.srcObject = stream;
    video.style.width = '100%';
    video.style.display = 'block';
    this.video = video;
    this.previewContainer?.replaceChildren(video);
    if (this.previewContainer) this.previewContainer.hidden = false;
    await video.play();
    onStatus?.('Loading face model…');
    const { FilesetResolver, FaceLandmarker } = await import('@mediapipe/tasks-vision');
    const wasmFileset = await FilesetResolver.forVisionTasks('/mediapipe/wasm');
    let delegate: 'GPU' | 'CPU' = 'GPU';
    try {
      this.landmarker = await FaceLandmarker.createFromOptions(wasmFileset, {
        baseOptions: { modelAssetPath: '/mediapipe/face_landmarker.task', delegate },
        runningMode: 'VIDEO',
        numFaces: 1,
        outputFaceBlendshapes: true,
        outputFacialTransformationMatrixes: false,
      });
    } catch (error) {
      delegate = 'CPU';
      onStatus?.('GPU delegate failed, retrying on CPU…');
      this.landmarker = await FaceLandmarker.createFromOptions(wasmFileset, {
        baseOptions: { modelAssetPath: '/mediapipe/face_landmarker.task', delegate },
        runningMode: 'VIDEO',
        numFaces: 1,
        outputFaceBlendshapes: true,
      });
    }
    this.smoother.reset();
    this.running = true;
    this.lastFrameTime = performance.now();
    this.frameCount = 0;
    this.raf = requestAnimationFrame(this.loop);
    onStatus?.(`Face tracking active (${this.mapping.count} morphs mapped).`);
  }

  stop(): void {
    if (!this.running && !this.landmarker && !this.stream) return;
    this.running = false;
    cancelAnimationFrame(this.raf);
    this.stream?.getTracks().forEach((track) => track.stop());
    this.stream = undefined;
    this.video?.remove();
    this.video = undefined;
    this.previewContainer?.replaceChildren();
    if (this.previewContainer) this.previewContainer.hidden = true;
    this.landmarker?.close();
    this.landmarker = undefined;
  }

  private readonly loop = (): void => {
    if (!this.running) return;
    this.raf = requestAnimationFrame(this.loop);
    const video = this.video;
    const landmarker = this.landmarker;
    if (!video || !landmarker || video.readyState < 2 || video.videoWidth === 0) return;

    const now = performance.now();
    this.frameCount++;
    if (now - this.lastFrameTime >= 1000) {
      this.fps = (this.frameCount * 1000) / (now - this.lastFrameTime);
      this.frameCount = 0;
      this.lastFrameTime = now;
    }

    // detectForVideo requires strictly increasing timestamps.
    const timestamp = now > this.lastTimestamp ? now : this.lastTimestamp + 1;
    this.lastTimestamp = timestamp;

    const result = landmarker.detectForVideo(video, timestamp);
    let applied = 0;
    let faceDetected = false;
    if (result.faceBlendshapes.length > 0) {
      faceDetected = true;
      applied = applyBlendshapes(
        this.mapping,
        result.faceBlendshapes[0].categories,
        this.callbacks.applyMorph,
        this.smoother,
      );
    }
    this.callbacks.onFrame?.({ faceDetected, applied, fps: this.fps });
  };
}
