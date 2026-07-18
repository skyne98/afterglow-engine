import { describe, expect, test } from 'bun:test';
import { EngineDiagnostics } from './diagnostics.ts';
import { RendererHost, type RendererViewport } from './renderer-host.ts';
import type { WebGPUOnlyRenderer } from './webgpu-only.ts';

class FakeViewport implements RendererViewport {
  width = 800;
  height = 600;
  pixelRatio = 3;
  listener: (() => void) | null = null;
  unsubscribed = 0;

  subscribe(listener: () => void): () => void {
    this.listener = listener;
    return (): void => { this.unsubscribed++; this.listener = null; };
  }
}

interface FakeRenderer extends WebGPUOnlyRenderer {
  initialized: number;
  compiled: number;
  rendered: number;
  disposed: number;
  ratios: number[];
  sizes: number[];
}

function fakeRenderer(device: EventTarget): FakeRenderer {
  const canvas = { parentNode: null } as unknown as HTMLCanvasElement;
  const renderer: FakeRenderer = {
    backend: {
      isWebGPUBackend: true,
      device,
      createRenderPipeline() {},
      createComputePipeline() {},
    },
    _getFallback: () => null,
    onDeviceLost() {},
    shadowMap: { enabled: true, type: 0 },
    getRenderTarget() { return null; },
    setRenderTarget() {},
    async readRenderTargetPixelsAsync() { return new Uint32Array(); },
    domElement: canvas,
    initialized: 0,
    compiled: 0,
    rendered: 0,
    disposed: 0,
    ratios: [],
    sizes: [],
    async init() { renderer.initialized++; },
    async compileAsync() { renderer.compiled++; },
    render() { renderer.rendered++; },
    async renderAsync() { renderer.rendered++; },
    setPixelRatio(value) { renderer.ratios.push(value); },
    setSize(width, height) { renderer.sizes.push(width, height); },
    dispose() { renderer.disposed++; },
  };
  return renderer;
}

async function withNavigator<T>(run: () => Promise<T>): Promise<T> {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  try {
    Object.defineProperty(globalThis, 'navigator', {
      configurable: true,
      value: { gpu: { requestAdapter: async () => ({ info: { vendor: 'test' } }) } },
    });
    return await run();
  } finally {
    if (original) Object.defineProperty(globalThis, 'navigator', original);
    else Reflect.deleteProperty(globalThis, 'navigator');
  }
}

describe('RendererHost', () => {
  test('owns viewport, warmup, rendering, pipeline seal, GPU errors, and disposal', async () => {
    await withNavigator(async () => {
      const diagnostics = new EngineDiagnostics(4);
      const viewport = new FakeViewport();
      let shaderModules = 0;
      const device = Object.assign(new EventTarget(), {
        createShaderModule(_descriptor: GPUShaderModuleDescriptor) { shaderModules++; return Object.create(null); },
      });
      const originalShaderFactory = device.createShaderModule;
      const renderer = fakeRenderer(device);
      const children: unknown[] = [];
      const container = {
        appendChild(child: unknown) { children.push(child); (child as { parentNode: unknown }).parentNode = container; },
        removeChild(child: unknown) {
          const index = children.indexOf(child);
          if (index >= 0) children.splice(index, 1);
          (child as { parentNode: unknown }).parentNode = null;
          return child;
        },
      } as unknown as HTMLElement;
      const host = await RendererHost.create({
        scene: {}, camera: {}, diagnostics, viewport, container,
        factory: () => renderer,
      });
      expect(renderer.initialized).toBe(1);
      expect(host.adapterInfo.vendor).toBe('test');
      expect(renderer.ratios).toEqual([2]);
      expect(renderer.sizes).toEqual([800, 600]);
      expect(children.length).toBe(1);
      await host.warm();
      expect(renderer.compiled).toBe(1);
      const inspected: string[] = [];
      await host.inspectShaderModulesDuring(async () => {
        host.device.createShaderModule({ code: 'test shader' });
      }, source => inspected.push(source));
      expect(inspected).toEqual(['test shader']);
      expect(shaderModules).toBe(1);
      expect(device.createShaderModule).toBe(originalShaderFactory);
      await expect(host.inspectShaderModulesDuring(async () => {
        throw new Error('warmup failed');
      }, () => {})).rejects.toThrow('warmup failed');
      expect(device.createShaderModule).toBe(originalShaderFactory);
      await expect(host.inspectShaderModulesDuring(async () => {
        await host.inspectShaderModulesDuring(async () => {}, () => {});
      }, () => {})).rejects.toThrow('already active');
      expect(device.createShaderModule).toBe(originalShaderFactory);
      host.render();
      expect(renderer.rendered).toBe(1);
      host.seal();
      await expect(host.inspectShaderModulesDuring(async () => {}, () => {}))
        .rejects.toThrow('bootstrap-only');
      renderer.backend.createRenderPipeline?.({}, []);
      expect(host.sealMonitor.renderPipelineViolations).toBe(1);
      device.dispatchEvent(new Event('uncapturederror'));
      expect(diagnostics.count).toBe(1);
      host.dispose();
      host.dispose();
      expect(renderer.disposed).toBe(1);
      expect(viewport.unsubscribed).toBe(1);
      expect(children.length).toBe(0);
    });
  });

  test('rolls the renderer back when required host methods are missing', async () => {
    await withNavigator(async () => {
      const renderer = fakeRenderer(new EventTarget());
      Reflect.deleteProperty(renderer, 'setSize');
      await expect(RendererHost.create({
        scene: {}, camera: {}, diagnostics: new EngineDiagnostics(1),
        viewport: new FakeViewport(), factory: () => renderer, showFailure: false,
        container: { appendChild() {} } as unknown as HTMLElement,
      })).rejects.toThrow('missing a required host method');
      expect(renderer.disposed).toBe(1);
    });
  });
});
