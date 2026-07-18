import type * as THREE from 'three';
import type { EngineRenderPass } from './runtime.ts';
import type { RenderWorkerInput } from './frame.ts';
import {
  VirtualTextureFeedbackPass,
  type FeedbackTextureStore,
} from './virtual-texture-feedback-pass.ts';
import type { VirtualPageRequest } from './virtual-texture.ts';
import type { RenderFrame } from './types.ts';

export const enum FeedbackRegistrationStatus {
  Registered = 0,
  CapacityExceeded = 1,
  InvalidPassCount = 2,
  Sealed = 3,
}

export interface FeedbackRenderable {
  readonly feedbackScene: THREE.Scene;
  readonly feedbackCamera: THREE.Camera;
  readonly feedbackPassCount: number;
  isFeedbackActive(): boolean;
  beginFeedbackPass(localPass: number): void;
  endFeedbackPass(localPass: number): void;
}

interface FeedbackRenderer {
  shadowMap: { enabled: boolean };
  getRenderTarget(): THREE.RenderTarget | null;
  setRenderTarget(target: THREE.RenderTarget | null): void;
  render(scene: THREE.Scene, camera: THREE.Camera): void;
  compileAsync(scene: THREE.Scene, camera: THREE.Camera): Promise<unknown>;
  readRenderTargetPixelsAsync(
    target: THREE.RenderTarget, x: number, y: number, width: number, height: number,
  ): Promise<ArrayBufferView>;
}

interface CoordinatedFeedbackStore extends FeedbackTextureStore {
  /** @alloc-effect none */
  recordFrameTime(frameTimeMs: number): void;
  processFeedbackBatch(
    feedbackMaps: ReadonlyArray<ReadonlyMap<unknown, VirtualPageRequest> | null>,
    mapCount: number,
  ): unknown;
  poll(): void;
}

interface RenderableSlot {
  renderable: FeedbackRenderable | null;
  passOffset: number;
}

/** Fixed-capacity owner of feedback targets, snapshots, state, and atomic merge. */
export interface VirtualTextureGpuTimings {
  gpuMainMs: number;
  gpuFeedbackMs: number;
  gpuTotalMs: number;
}

export class VirtualTextureFeedbackCoordinator implements EngineRenderPass, RenderWorkerInput {
  vtCpuUs = 0;
  feedbackSubmitUs = 0;
  readonly pixelScale: THREE.Vector2;
  readonly stats = {
    submittedSnapshots: 0,
    completedSnapshots: 0,
    discardedSnapshots: 0,
    deferredSnapshots: 0,
    registrationOverflows: 0,
    activePasses: 0,
  };

  private readonly passes: VirtualTextureFeedbackPass[];
  private readonly renderables: RenderableSlot[];
  private readonly activeRenderable: Int32Array;
  private readonly activeLocalPass: Uint16Array;
  private readonly heldResults: Array<Map<number, VirtualPageRequest> | null>;
  private renderableCount = 0;
  private registeredPassCount = 0;
  private awaitingPassCount = 0;
  private discardAwaiting = false;
  private sealed = false;
  private disposed = false;

  constructor(
    private readonly renderer: FeedbackRenderer,
    private readonly store: CoordinatedFeedbackStore,
    capacities: { renderables: number; passes: number; cadence: number; scale?: number },
  ) {
    if (!Number.isInteger(capacities.renderables) || capacities.renderables <= 0)
      throw new RangeError('feedback renderable capacity must be positive');
    if (!Number.isInteger(capacities.passes) || capacities.passes <= 0)
      throw new RangeError('feedback pass capacity must be positive');
    if (!Number.isInteger(capacities.cadence) || capacities.cadence <= 0)
      throw new RangeError('feedback cadence must be positive');
    this.cadence = capacities.cadence;
    this.passes = new Array<VirtualTextureFeedbackPass>(capacities.passes);
    this.renderables = new Array<RenderableSlot>(capacities.renderables);
    this.heldResults = new Array<Map<number, VirtualPageRequest> | null>(capacities.passes).fill(null);
    for (let index = 0; index < capacities.passes; index++)
      this.passes[index] = new VirtualTextureFeedbackPass(capacities.scale ?? 0.125);
    for (let index = 0; index < capacities.renderables; index++)
      this.renderables[index] = { renderable: null, passOffset: 0 };
    this.activeRenderable = new Int32Array(capacities.passes);
    this.activeLocalPass = new Uint16Array(capacities.passes);
    const firstPass = this.passes[0];
    if (!firstPass) throw new Error('feedback coordinator failed to reserve its first pass');
    this.pixelScale = firstPass.pixelScale;
  }

  readonly cadence: number;

  register(renderable: FeedbackRenderable): FeedbackRegistrationStatus {
    if (this.sealed) return FeedbackRegistrationStatus.Sealed;
    if (!Number.isInteger(renderable.feedbackPassCount) || renderable.feedbackPassCount <= 0)
      return FeedbackRegistrationStatus.InvalidPassCount;
    if (this.renderableCount === this.renderables.length ||
        this.registeredPassCount + renderable.feedbackPassCount > this.passes.length) {
      this.stats.registrationOverflows++;
      return FeedbackRegistrationStatus.CapacityExceeded;
    }
    const slot = this.renderables[this.renderableCount];
    if (!slot) return FeedbackRegistrationStatus.CapacityExceeded;
    slot.renderable = renderable;
    slot.passOffset = this.registeredPassCount;
    this.registeredPassCount += renderable.feedbackPassCount;
    this.renderableCount++;
    return FeedbackRegistrationStatus.Registered;
  }

  resize(displayWidth: number, displayHeight: number): void {
    for (let index = 0; index < this.passes.length; index++) this.passes[index]?.resize(displayWidth, displayHeight);
  }

  async warm(): Promise<void> {
    if (this.disposed) throw new Error('cannot warm a disposed feedback coordinator');
    const previousTarget = this.renderer.getRenderTarget();
    const shadows = this.renderer.shadowMap.enabled;
    this.renderer.shadowMap.enabled = false;
    try {
      for (let recordIndex = 0; recordIndex < this.renderableCount; recordIndex++) {
        const record = this.renderables[recordIndex];
        const renderable = record?.renderable;
        if (!record || !renderable) continue;
        for (let localPass = 0; localPass < renderable.feedbackPassCount; localPass++) {
          const pass = this.passes[record.passOffset + localPass];
          if (!pass) continue;
          renderable.beginFeedbackPass(localPass);
          try {
            this.renderer.setRenderTarget(pass.target);
            await this.renderer.compileAsync(renderable.feedbackScene, renderable.feedbackCamera);
            this.renderer.render(renderable.feedbackScene, renderable.feedbackCamera);
          } finally {
            renderable.endFeedbackPass(localPass);
          }
        }
      }
    } finally {
      this.renderer.setRenderTarget(previousTarget);
      this.renderer.shadowMap.enabled = shadows;
    }
  }

  seal(): void { this.sealed = true; }

  setGpuTimingEnabled(enabled: boolean): void {
    const renderer = this.renderer as unknown as { // @unsafe-cast reason=ThreePrivateTimestampTracking issue=DME-030 expires=2026-10-01
      backend: { trackTimestamp?: boolean; timestampQueryPool?: Record<string, { trackTimestamp?: boolean } | undefined> };
    };
    renderer.backend.trackTimestamp = enabled;
    for (const pool of Object.values(renderer.backend.timestampQueryPool ?? {})) if (pool) pool.trackTimestamp = enabled;
  }

  async resolveGpuTimings(out: VirtualTextureGpuTimings): Promise<VirtualTextureGpuTimings> {
    const renderer = this.renderer as unknown as { // @unsafe-cast reason=ThreePrivateTimestampReadback issue=DME-030 expires=2026-10-01
      resolveTimestampsAsync(type: string): Promise<number>;
      _renderContexts?: Map<unknown, { id: number }>;
      backend: { timestampQueryPool?: { render?: { timestamps?: Map<string, number>; getTimestampFrames?(): unknown[] } } };
    };
    out.gpuTotalMs = await renderer.resolveTimestampsAsync('render');
    const contexts = renderer._renderContexts, pool = renderer.backend.timestampQueryPool?.render;
    const timestamps = pool?.timestamps, feedbackTarget = this.passes[0]?.target;
    if (!contexts || !timestamps || !feedbackTarget) return out;
    const main = contexts.get(null)?.id, feedback = contexts.get(feedbackTarget)?.id;
    let mainFrame = -1, feedbackFrame = -1;
    for (const [uid, duration] of timestamps) {
      const parts = uid.split(':'), context = Number(parts[2]), id = Number(parts[3]?.slice(1));
      if (context === main && id > mainFrame) { mainFrame = id; out.gpuMainMs = duration; }
      else if (context === feedback && id > feedbackFrame) { feedbackFrame = id; out.gpuFeedbackMs = duration; }
    }
    timestamps.clear();
    const frames = pool?.getTimestampFrames?.(); if (frames) frames.length = 0;
    return out;
  }

  /** @alloc-effect none */
  recordFrameTime(frameTimeMs: number): void { this.store.recordFrameTime(frameTimeMs); }

  /** Worker-stage hook: publish only complete logical snapshots, then advance VT. */
  poll(): void {
    const started = performance.now();
    this.consumeCompletedSnapshot();
    this.store.poll();
    this.vtCpuUs = (performance.now() - started) * 1000;
  }

  render(frame: Readonly<RenderFrame>): void {
    this.feedbackSubmitUs = 0;
    if (this.disposed || frame.frameId % this.cadence !== 0) return;
    const started = performance.now();
    if (this.awaitingPassCount !== 0) {
      this.stats.deferredSnapshots++;
      return;
    }
    let activeCount = 0;
    for (let recordIndex = 0; recordIndex < this.renderableCount; recordIndex++) {
      const renderable = this.renderables[recordIndex]?.renderable;
      if (!renderable || !renderable.isFeedbackActive()) continue;
      for (let localPass = 0; localPass < renderable.feedbackPassCount; localPass++) {
        if (activeCount >= this.passes.length) break;
        this.activeRenderable[activeCount] = recordIndex;
        this.activeLocalPass[activeCount] = localPass;
        activeCount++;
      }
    }
    this.stats.activePasses = activeCount;
    if (activeCount === 0) return;
    for (let index = 0; index < activeCount; index++) {
      if (this.passes[index]?.canSubmit !== true) {
        this.stats.deferredSnapshots++;
        return;
      }
    }

    const shadows = this.renderer.shadowMap.enabled;
    this.renderer.shadowMap.enabled = false;
    let submitted = 0;
    try {
      for (let index = 0; index < activeCount; index++) {
        const renderable = this.renderables[this.activeRenderable[index] ?? -1]?.renderable;
        const localPass = this.activeLocalPass[index] ?? 0;
        const pass = this.passes[index];
        if (!renderable || !pass) continue;
        renderable.beginFeedbackPass(localPass);
        try {
          if (pass.submit(
            this.renderer, renderable.feedbackScene, renderable.feedbackCamera, this.store,
          )) submitted++;
        } finally {
          renderable.endFeedbackPass(localPass);
        }
      }
    } finally {
      this.renderer.shadowMap.enabled = shadows;
    }
    this.awaitingPassCount = submitted;
    this.discardAwaiting = submitted !== activeCount;
    if (submitted !== 0) this.stats.submittedSnapshots++;
    if (this.discardAwaiting) this.stats.deferredSnapshots++;
    this.feedbackSubmitUs = (performance.now() - started) * 1000;
  }

  private consumeCompletedSnapshot(): boolean {
    if (this.awaitingPassCount === 0) return false;
    let complete = true;
    for (let index = 0; index < this.awaitingPassCount; index++) {
      if (this.heldResults[index] === null) this.heldResults[index] = this.passes[index]?.consume() ?? null;
      if (this.heldResults[index] === null) complete = false;
    }
    if (!complete) return false;
    if (this.discardAwaiting) {
      this.stats.discardedSnapshots++;
    } else {
      this.store.processFeedbackBatch(this.heldResults, this.awaitingPassCount);
      this.stats.completedSnapshots++;
    }
    for (let index = 0; index < this.awaitingPassCount; index++) this.heldResults[index] = null;
    this.awaitingPassCount = 0;
    this.discardAwaiting = false;
    return true;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (let index = 0; index < this.passes.length; index++) this.passes[index]?.dispose();
    for (let index = 0; index < this.renderables.length; index++) {
      const slot = this.renderables[index];
      if (slot) slot.renderable = null;
    }
    for (let index = 0; index < this.heldResults.length; index++) this.heldResults[index] = null;
    this.awaitingPassCount = 0;
  }
}
