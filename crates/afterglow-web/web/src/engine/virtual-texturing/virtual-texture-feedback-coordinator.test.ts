import { describe, expect, test } from 'bun:test';
import * as THREE from 'three';
import {
  FeedbackRegistrationStatus,
  VirtualTextureFeedbackCoordinator,
  type FeedbackRenderable,
} from './virtual-texture-feedback-coordinator.ts';
import type { VirtualPageRequest } from './virtual-texture.ts';

interface DeferredRead {
  promise: Promise<ArrayBufferView>;
  resolve(value: ArrayBufferView): void;
}

function deferredRead(): DeferredRead {
  let complete: ((value: ArrayBufferView) => void) | null = null;
  const promise = new Promise<ArrayBufferView>((resolve) => { complete = resolve; });
  return {
    promise,
    resolve(value) {
      if (complete === null) throw new Error('read resolver was not initialized');
      complete(value);
    },
  };
}

class FakeRenderer {
  shadowMap = { enabled: true };
  target: THREE.RenderTarget | null = null;
  needsFrameBufferTarget = true;
  timestampFrames: number[] = [];
  timestamps = new Map<string, number>();
  backend = {
    trackTimestamp: true,
    timestampQueryPool: {
      render: {
        timestamps: this.timestamps,
        getTimestampFrames: (): number[] => this.timestampFrames,
      },
    },
  };
  _renderContexts = {
    get: (target: unknown): { id: number } => ({ id: target === null ? 2 : 3 }),
  };
  reads: DeferredRead[] = [];
  readIndex = 0;
  renders = 0;
  compiles = 0;
  throwOnRender = false;

  getRenderTarget(): THREE.RenderTarget | null { return this.target; }
  setRenderTarget(target: THREE.RenderTarget | null): void { this.target = target; }
  render(): void {
    this.renders++;
    if (this.throwOnRender) throw new Error('feedback render failed');
  }
  async compileAsync(): Promise<void> { this.compiles++; }
  async resolveTimestampsAsync(): Promise<number> {
    let total = 0;
    const frame = this.timestampFrames[this.timestampFrames.length - 1];
    for (const [uid, duration] of this.timestamps) if (uid.endsWith(`:f${frame}`)) total += duration;
    return total;
  }
  readRenderTargetPixelsAsync(): Promise<ArrayBufferView> {
    const read = this.reads[this.readIndex++];
    if (!read) throw new Error('missing deferred feedback read');
    return read.promise;
  }
}

class FakeStore {
  batches = 0;
  batchCount = 0;
  polls = 0;
  frameTime = 0;
  publicationFrameId = -1;
  getEntryById(): undefined { return undefined; }
  recordFrameTime(value: number): void { this.frameTime = value; }
  setPublicationFrameId(value: number): void { this.publicationFrameId = value; }
  processFeedbackBatch(
    _maps: ReadonlyArray<ReadonlyMap<unknown, VirtualPageRequest> | null>, count: number,
  ): void { this.batches++; this.batchCount = count; }
  poll(): void { this.polls++; }
}

class FakeRenderable implements FeedbackRenderable {
  readonly feedbackScene = new THREE.Scene();
  readonly feedbackCamera = new THREE.PerspectiveCamera();
  active = true;
  begins = 0;
  ends = 0;
  constructor(readonly feedbackPassCount: number) {}
  isFeedbackActive(): boolean { return this.active; }
  beginFeedbackPass(): void { this.begins++; }
  endFeedbackPass(): void { this.ends++; }
}

async function flushReads(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe('VirtualTextureFeedbackCoordinator', () => {
  test('publishes all passes as one atomic visibility epoch', async () => {
    const renderer = new FakeRenderer();
    const store = new FakeStore();
    const first = deferredRead();
    const second = deferredRead();
    renderer.reads.push(first, second);
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, store, {
      renderables: 1, passes: 2, cadenceMs: 1,
    });
    coordinator.resize(800, 600);
    const renderable = new FakeRenderable(2);
    expect(coordinator.register(renderable)).toBe(FeedbackRegistrationStatus.Registered);
    coordinator.seal();
    coordinator.render({ frameId: 1, deltaSeconds: 0.016, elapsedSeconds: 0.016 });
    expect(renderer.shadowMap.enabled).toBe(true);
    expect(renderable.begins).toBe(2);
    expect(renderable.ends).toBe(2);

    first.resolve(new Uint32Array([0, 0]));
    await flushReads();
    coordinator.poll();
    expect(store.batches).toBe(0);
    second.resolve(new Uint32Array([0, 0]));
    await flushReads();
    coordinator.poll();
    expect(store.batches).toBe(1);
    expect(store.batchCount).toBe(2);
    expect(coordinator.stats.completedSnapshots).toBe(1);
  });

  test('has fixed registration capacity and rejects late registration', () => {
    const coordinator = new VirtualTextureFeedbackCoordinator(new FakeRenderer(), new FakeStore(), {
      renderables: 1, passes: 2, cadenceMs: 8,
    });
    expect(coordinator.register(new FakeRenderable(0))).toBe(FeedbackRegistrationStatus.InvalidPassCount);
    expect(coordinator.register(new FakeRenderable(2))).toBe(FeedbackRegistrationStatus.Registered);
    expect(coordinator.register(new FakeRenderable(1))).toBe(FeedbackRegistrationStatus.CapacityExceeded);
    coordinator.seal();
    expect(coordinator.register(new FakeRenderable(1))).toBe(FeedbackRegistrationStatus.Sealed);
  });

  test('warms every registered variant and restores target, shadow, and material state', async () => {
    const renderer = new FakeRenderer();
    const originalTarget = new THREE.RenderTarget(1, 1);
    renderer.target = originalTarget;
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 2, cadenceMs: 1,
    });
    const renderable = new FakeRenderable(2);
    coordinator.register(renderable);
    await coordinator.warm();
    expect(renderer.compiles).toBe(2);
    expect(renderer.renders).toBe(2);
    expect(renderer.target).toBe(originalTarget);
    expect(renderer.shadowMap.enabled).toBe(true);
    expect(renderable.begins).toBe(2);
    expect(renderable.ends).toBe(2);
  });

  test('resolves one logical frame into scene, output, feedback, and total', async () => {
    const renderer = new FakeRenderer();
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 2, cadenceMs: 1,
    });
    renderer.timestampFrames.push(8, 9);
    renderer.timestamps.set('r:1:1:f8', 99);
    renderer.timestamps.set('r:1:1:f9', 4);
    renderer.timestamps.set('r:2:2:f9', 1);
    renderer.timestamps.set('r:3:3:f9', 0.25);
    renderer.timestamps.set('r:4:3:f9', 0.15);
    renderer.timestamps.set('malformed', 100);
    const out = {
      gpuTimingValid: false, resolvedFrameId: -1, gpuSceneMs: -1,
      gpuOutputMs: -1, gpuFeedbackMs: -1, gpuTotalMs: -1,
    };
    await coordinator.resolveGpuTimings(out);
    expect(out).toEqual({
      gpuTimingValid: true, resolvedFrameId: 9, gpuSceneMs: 4,
      gpuOutputMs: 1, gpuFeedbackMs: 0.4, gpuTotalMs: 5.4,
    });
    expect(renderer.timestamps.size).toBe(0);
    expect(renderer.timestampFrames.length).toBe(0);
  });

  test('treats the canvas context as scene work without an output transform', async () => {
    const renderer = new FakeRenderer();
    renderer.needsFrameBufferTarget = false;
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 1, cadenceMs: 1,
    });
    renderer.timestampFrames.push(10);
    renderer.timestamps.set('r:1:2:f10', 3);
    renderer.timestamps.set('r:2:3:f10', 0.1);
    const out = {
      gpuTimingValid: true, resolvedFrameId: 99, gpuSceneMs: 99,
      gpuOutputMs: 99, gpuFeedbackMs: 99, gpuTotalMs: 99,
    };
    await coordinator.resolveGpuTimings(out);
    expect(out).toEqual({
      gpuTimingValid: true, resolvedFrameId: 10, gpuSceneMs: 3,
      gpuOutputMs: 0, gpuFeedbackMs: 0.1, gpuTotalMs: 3.1,
    });
  });

  test('reports unavailable timing deterministically when no frame resolved', async () => {
    const renderer = new FakeRenderer();
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 1, cadenceMs: 1,
    });
    const out = {
      gpuTimingValid: true, resolvedFrameId: 99, gpuSceneMs: 99,
      gpuOutputMs: 99, gpuFeedbackMs: 99, gpuTotalMs: 99,
    };
    await coordinator.resolveGpuTimings(out);
    expect(out).toEqual({
      gpuTimingValid: false, resolvedFrameId: -1, gpuSceneMs: 0,
      gpuOutputMs: 0, gpuFeedbackMs: 0, gpuTotalMs: 0,
    });
  });

  test('uses a 55 ms monotonic cadence without catch-up bursts', async () => {
    const renderer = new FakeRenderer();
    const reads = [deferredRead(), deferredRead(), deferredRead()];
    renderer.reads.push(...reads);
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 1, cadenceMs: 55,
    });
    coordinator.register(new FakeRenderable(1));
    coordinator.render({ frameId: 0, deltaSeconds: 0, elapsedSeconds: 0 });
    expect(renderer.renders).toBe(1);
    reads[0]!.resolve(new Uint32Array([0, 0]));
    await flushReads(); coordinator.poll();
    coordinator.render({ frameId: 7, deltaSeconds: 1 / 144, elapsedSeconds: 0.05 });
    expect(renderer.renders).toBe(1);
    coordinator.render({ frameId: 8, deltaSeconds: 1 / 144, elapsedSeconds: 0.056 });
    expect(renderer.renders).toBe(2);
    reads[1]!.resolve(new Uint32Array([0, 0]));
    await flushReads(); coordinator.poll();
    coordinator.render({ frameId: 30, deltaSeconds: 0.444, elapsedSeconds: 0.5 });
    expect(renderer.renders).toBe(3);
    reads[2]!.resolve(new Uint32Array([0, 0]));
    await flushReads(); coordinator.poll();
    coordinator.render({ frameId: 31, deltaSeconds: 0.001, elapsedSeconds: 0.501 });
    expect(renderer.renders).toBe(3);
    expect(() => new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 1, cadenceMs: 0,
    })).toThrow('cadence');
  });

  test('restores all state when feedback rendering throws', () => {
    const renderer = new FakeRenderer();
    renderer.reads.push(deferredRead());
    renderer.throwOnRender = true;
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 1, cadenceMs: 1,
    });
    const renderable = new FakeRenderable(1);
    coordinator.register(renderable);
    expect(() => coordinator.render({ frameId: 1, deltaSeconds: 0.016, elapsedSeconds: 0.016 }))
      .toThrow('feedback render failed');
    expect(renderer.target).toBeNull();
    expect(renderer.shadowMap.enabled).toBe(true);
    expect(renderable.ends).toBe(1);
  });
});
