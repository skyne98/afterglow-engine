import { describe, expect, test } from 'bun:test';
import {
  assertWebGPUBackend,
  createWebGPUOnlyRenderer,
  disableWebGLFallback,
  type WebGPUOnlyRenderer,
} from './webgpu-only.ts';

function renderer(partialBackend: Partial<WebGPUOnlyRenderer['backend']>): WebGPUOnlyRenderer {
  return {
    backend: {
      createRenderPipeline() {},
      createComputePipeline() {},
      ...partialBackend,
    },
    _getFallback: () => ({ isWebGLBackend: true }),
    onDeviceLost: () => {},
    async init() {},
    async compileAsync() {},
    render() {},
    async renderAsync() {},
    setPixelRatio() {},
    setSize() {},
    domElement: {} as HTMLCanvasElement,
    dispose() {},
  };
}

describe('WebGPU-only renderer guard', () => {
  test('clears Three r185 fallback callback before initialization', () => {
    const candidate = renderer({ isWebGPUBackend: true, device: {} });
    disableWebGLFallback(candidate);
    expect(candidate._getFallback).toBeNull();
  });

  test('accepts only a live WebGPU backend', () => {
    expect(() => assertWebGPUBackend(renderer({ isWebGPUBackend: true, device: {} }))).not.toThrow();
    expect(() => assertWebGPUBackend(renderer({ isWebGPUBackend: false, device: {} }))).toThrow('WebGL fallback is forbidden');
    expect(() => assertWebGPUBackend(renderer({ isWebGPUBackend: true }))).toThrow('WebGL fallback is forbidden');
  });

  test('fails initialization instead of invoking Three WebGL fallback', async () => {
    const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
    let fallbackCalled = false;
    const candidate = renderer({ isWebGPUBackend: true, device: {} }) as WebGPUOnlyRenderer & { init: () => Promise<void> };
    candidate.init = async () => {
      if (candidate._getFallback !== null) fallbackCalled = true;
      throw new Error('WebGPU device initialization failed');
    };

    try {
      Object.defineProperty(globalThis, 'navigator', {
        configurable: true,
        value: { gpu: { requestAdapter: async () => ({ info: { vendor: 'test-vendor', architecture: 'test-arch' } }) } },
      });
      await expect(createWebGPUOnlyRenderer({}, () => candidate)).rejects.toThrow('WebGPU device initialization failed');
      expect(candidate.afterglowAdapterInfo).toMatchObject({ vendor: 'test-vendor', architecture: 'test-arch' });
      expect(fallbackCalled).toBeFalse();
    } finally {
      if (originalNavigator) Object.defineProperty(globalThis, 'navigator', originalNavigator);
      else delete (globalThis as { navigator?: unknown }).navigator;
    }
  });
});
