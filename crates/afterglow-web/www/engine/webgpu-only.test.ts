import { describe, expect, test } from 'bun:test';
import {
  assertWebGPUBackend,
  createWebGPUOnlyRenderer,
  disableWebGLFallback,
  type WebGPUOnlyRenderer,
} from './webgpu-only.ts';

function renderer(backend: WebGPUOnlyRenderer['backend']): WebGPUOnlyRenderer {
  return {
    backend,
    _getFallback: () => ({ isWebGLBackend: true }),
    onDeviceLost: () => {},
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
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    let fallbackCalled = false;
    const candidate = renderer({ isWebGPUBackend: true, device: {} }) as WebGPUOnlyRenderer & { init: () => Promise<void> };
    candidate.init = async () => {
      if (candidate._getFallback !== null) fallbackCalled = true;
      throw new Error('WebGPU device initialization failed');
    };

    try {
      Object.defineProperty(globalThis, 'navigator', {
        configurable: true,
        value: { gpu: { requestAdapter: async () => ({}) } },
      });
      Object.defineProperty(globalThis, 'window', {
        configurable: true,
        value: { THREE: { WebGPURenderer: function () { return candidate; } } },
      });
      await expect(createWebGPUOnlyRenderer()).rejects.toThrow('WebGPU device initialization failed');
      expect(fallbackCalled).toBeFalse();
    } finally {
      if (originalNavigator) Object.defineProperty(globalThis, 'navigator', originalNavigator);
      else delete (globalThis as { navigator?: unknown }).navigator;
      if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
      else delete (globalThis as { window?: unknown }).window;
    }
  });
});
