import { WebGPURenderer } from 'three/webgpu';
import { EngineDiagnostics, DiagnosticCode, DiagnosticSource } from './diagnostics.ts';
import { RendererSeal } from './renderer-seal.ts';
import type { EngineRenderPass } from './runtime.ts';
import type { VirtualTextureStore } from './virtual-texture.ts';
import { assertHeightTextureGpuFormat } from './height-texture.ts';
import {
  createWebGPUOnlyRenderer,
  showWebGPUFailure,
  type WebGPUOnlyRenderer,
  type WebGPUOnlyRendererFactory,
} from './webgpu-only.ts';

interface PipelineBackend {
  createRenderPipeline(renderObject: unknown, promises: Promise<unknown>[]): void;
  createComputePipeline(computePipeline: unknown, bindings: unknown): void;
}

interface GpuErrorTarget {
  addEventListener(type: 'uncapturederror', listener: (event: Event) => void): void;
  removeEventListener(type: 'uncapturederror', listener: (event: Event) => void): void;
}

export interface RendererViewport {
  readonly width: number;
  readonly height: number;
  readonly pixelRatio: number;
  subscribe(listener: () => void): () => void;
}

class BrowserRendererViewport implements RendererViewport {
  get width(): number { return window.innerWidth; }
  get height(): number { return window.innerHeight; }
  get pixelRatio(): number { return window.devicePixelRatio; }
  subscribe(listener: () => void): () => void {
    window.addEventListener('resize', listener);
    return (): void => window.removeEventListener('resize', listener);
  }
}

export const moduleRendererFactory: WebGPUOnlyRendererFactory = parameters =>
  new WebGPURenderer(parameters) as unknown as WebGPUOnlyRenderer; // @unsafe-cast reason=ThreePrivateRenderer issue=DME-040 expires=2026-10-01

export interface RendererHostOptions {
  scene: unknown;
  camera: unknown;
  diagnostics: EngineDiagnostics;
  container?: HTMLElement;
  viewport?: RendererViewport;
  parameters?: Record<string, unknown>;
  factory?: WebGPUOnlyRendererFactory;
  maxPixelRatio?: number;
  onResize?: (width: number, height: number) => void;
  showFailure?: boolean;
}

function requireGpuDevice(renderer: WebGPUOnlyRenderer): GPUDevice {
  if (typeof renderer.backend.device !== 'object' || renderer.backend.device === null)
    throw new Error('Three WebGPU renderer has no live GPU device');
  return renderer.backend.device as GPUDevice;
}

function requirePipelineBackend(renderer: WebGPUOnlyRenderer): PipelineBackend {
  const backend = renderer.backend;
  return backend as PipelineBackend;
}

function gpuErrorTarget(device: unknown): GpuErrorTarget | null {
  if (typeof device !== 'object' || device === null) return null;
  const candidate = device as Partial<GpuErrorTarget>;
  return candidate.addEventListener && candidate.removeEventListener ? candidate as GpuErrorTarget : null;
}

/** Owns one WebGPU-only Three renderer, viewport listeners, warmup, and seal. */
export class RendererHost implements EngineRenderPass {
  readonly renderer: WebGPUOnlyRenderer;
  readonly device: GPUDevice;
  readonly sealMonitor: RendererSeal;
  readonly timestampSupported: boolean;
  renderSubmissions = 0;
  renderSubmitUs = 0;

  private readonly scene: unknown;
  private readonly camera: unknown;
  private readonly diagnostics: EngineDiagnostics;
  private readonly container: HTMLElement;
  private readonly viewport: RendererViewport;
  private readonly maxPixelRatio: number;
  private readonly deviceTarget: GpuErrorTarget | null;
  private readonly resizeClient: ((width: number, height: number) => void) | undefined;
  private readonly onResize: () => void;
  private readonly onGpuError: (event: Event) => void;
  private unsubscribeResize: (() => void) | null = null;
  private disposed = false;

  private constructor(renderer: WebGPUOnlyRenderer, options: RendererHostOptions) {
    if (!renderer.domElement || !renderer.setPixelRatio || !renderer.setSize ||
        !renderer.compileAsync || !renderer.render)
      throw new Error('Three WebGPU renderer is missing a required host method');
    this.renderer = renderer;
    this.device = requireGpuDevice(renderer);
    this.scene = options.scene;
    this.camera = options.camera;
    this.diagnostics = options.diagnostics;
    this.container = options.container ?? document.body;
    this.viewport = options.viewport ?? new BrowserRendererViewport();
    this.maxPixelRatio = options.maxPixelRatio ?? 2;
    this.resizeClient = options.onResize;
    if (!Number.isFinite(this.maxPixelRatio) || this.maxPixelRatio <= 0)
      throw new RangeError('maxPixelRatio must be positive');
    this.sealMonitor = new RendererSeal(requirePipelineBackend(renderer));
    const timestampBackend = renderer.backend as unknown as { hasTimestamp?: boolean }; // @unsafe-cast reason=ThreePrivateTimestampCapability issue=DME-030 expires=2026-10-01
    this.timestampSupported = Boolean(timestampBackend.hasTimestamp);
    this.deviceTarget = gpuErrorTarget(renderer.backend.device);
    this.onResize = (): void => this.resize();
    this.onGpuError = (event: Event): void => {
      this.diagnostics.tryRecord(DiagnosticCode.UncapturedGpuError, DiagnosticSource.Renderer, event);
    };
    try {
      this.container.appendChild(renderer.domElement);
      this.resize();
      this.unsubscribeResize = this.viewport.subscribe(this.onResize);
      this.deviceTarget?.addEventListener('uncapturederror', this.onGpuError);
    } catch (error) {
      this.unsubscribeResize?.();
      this.unsubscribeResize = null;
      this.deviceTarget?.removeEventListener('uncapturederror', this.onGpuError);
      if (renderer.domElement.parentNode === this.container)
        this.container.removeChild(renderer.domElement);
      throw error;
    }
  }

  static async create(options: RendererHostOptions): Promise<RendererHost> {
    let renderer: WebGPUOnlyRenderer | null = null;
    try {
      renderer = await createWebGPUOnlyRenderer(
        options.parameters,
        options.factory ?? moduleRendererFactory,
      );
      return new RendererHost(renderer, options);
    } catch (error) {
      renderer?.dispose();
      if (options.showFailure !== false) showWebGPUFailure(error);
      throw error;
    }
  }

  resize(): void {
    const ratio = Math.min(this.maxPixelRatio, Math.max(0.1, this.viewport.pixelRatio));
    const width = Math.max(1, this.viewport.width);
    const height = Math.max(1, this.viewport.height);
    this.renderer.setPixelRatio(ratio);
    this.renderer.setSize(width, height);
    this.resizeClient?.(width, height);
  }

  assertHeightTextureFormat(texture: Parameters<typeof assertHeightTextureGpuFormat>[1]): void {
    const backend = this.renderer.backend as unknown as Parameters<typeof assertHeightTextureGpuFormat>[0]; // @unsafe-cast reason=ThreePrivateTextureFormat issue=DME-030 expires=2026-10-01
    assertHeightTextureGpuFormat(backend, texture);
  }

  attachVirtualTextureStore(store: VirtualTextureStore): void {
    const backend = this.renderer.backend as unknown as { // @unsafe-cast reason=ThreePrivateTextureLookup issue=DME-040 expires=2026-10-01
      get(texture: unknown): { texture?: GPUTexture };
    };
    store.attachRenderer({
      backend: {
        device: this.device,
        get: (texture) => backend.get(texture),
      },
    });
  }

  async warm(): Promise<void> {
    if (this.disposed) throw new Error('cannot warm a disposed renderer host');
    await this.renderer.compileAsync(this.scene, this.camera);
  }

  seal(): void { this.sealMonitor.seal(); }

  render(): void {
    if (this.disposed) return;
    this.renderSubmissions++;
    const started = performance.now();
    this.renderer.render(this.scene, this.camera);
    this.renderSubmitUs = (performance.now() - started) * 1000;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.unsubscribeResize?.();
    this.unsubscribeResize = null;
    this.deviceTarget?.removeEventListener('uncapturederror', this.onGpuError);
    const canvas = this.renderer.domElement;
    if (canvas.parentNode === this.container) this.container.removeChild(canvas);
    this.renderer.dispose();
  }
}
