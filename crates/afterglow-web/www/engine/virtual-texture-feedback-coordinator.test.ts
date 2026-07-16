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
  getEntryById(): undefined { return undefined; }
  recordFrameTime(value: number): void { this.frameTime = value; }
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
      renderables: 1, passes: 2, cadence: 1,
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
      renderables: 1, passes: 2, cadence: 8,
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
      renderables: 1, passes: 2, cadence: 1,
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

  test('restores all state when feedback rendering throws', () => {
    const renderer = new FakeRenderer();
    renderer.reads.push(deferredRead());
    renderer.throwOnRender = true;
    const coordinator = new VirtualTextureFeedbackCoordinator(renderer, new FakeStore(), {
      renderables: 1, passes: 1, cadence: 1,
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
