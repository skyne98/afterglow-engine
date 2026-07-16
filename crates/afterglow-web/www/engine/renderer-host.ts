import { EngineDiagnostics, DiagnosticCode, DiagnosticSource } from './diagnostics.ts';
import { RendererSeal } from './renderer-seal.ts';
import type { EngineRenderPass } from './runtime.ts';
import {
  createWebGPUOnlyRenderer,
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

export interface RendererHostOptions {
  scene: unknown;
  camera: unknown;
  diagnostics: EngineDiagnostics;
  container?: HTMLElement;
  viewport?: RendererViewport;
  parameters?: Record<string, unknown>;
  factory?: WebGPUOnlyRendererFactory;
  maxPixelRatio?: number;
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
  readonly sealMonitor: RendererSeal;
  renderSubmissions = 0;

  private readonly scene: unknown;
  private readonly camera: unknown;
  private readonly diagnostics: EngineDiagnostics;
  private readonly container: HTMLElement;
  private readonly viewport: RendererViewport;
  private readonly maxPixelRatio: number;
  private readonly deviceTarget: GpuErrorTarget | null;
  private readonly onResize: () => void;
  private readonly onGpuError: (event: Event) => void;
  private unsubscribeResize: (() => void) | null = null;
  private disposed = false;

  private constructor(renderer: WebGPUOnlyRenderer, options: RendererHostOptions) {
    if (!renderer.domElement || !renderer.setPixelRatio || !renderer.setSize ||
        !renderer.compileAsync || !renderer.renderAsync)
      throw new Error('Three WebGPU renderer is missing a required host method');
    this.renderer = renderer;
    this.scene = options.scene;
    this.camera = options.camera;
    this.diagnostics = options.diagnostics;
    this.container = options.container ?? document.body;
    this.viewport = options.viewport ?? new BrowserRendererViewport();
    this.maxPixelRatio = options.maxPixelRatio ?? 2;
    if (!Number.isFinite(this.maxPixelRatio) || this.maxPixelRatio <= 0)
      throw new RangeError('maxPixelRatio must be positive');
    this.sealMonitor = new RendererSeal(requirePipelineBackend(renderer));
    this.deviceTarget = gpuErrorTarget(renderer.backend.device);
    this.onResize = (): void => this.resize();
    this.onGpuError = (event: Event): void => {
      this.diagnostics.tryRecord(DiagnosticCode.UncapturedGpuError, DiagnosticSource.Renderer, event);
    };
    this.container.appendChild(renderer.domElement);
    this.resize();
    this.unsubscribeResize = this.viewport.subscribe(this.onResize);
    this.deviceTarget?.addEventListener('uncapturederror', this.onGpuError);
  }

  static async create(options: RendererHostOptions): Promise<RendererHost> {
    const renderer = await createWebGPUOnlyRenderer(options.parameters, options.factory);
    try {
      return new RendererHost(renderer, options);
    } catch (error) {
      renderer.dispose();
      throw error;
    }
  }

  resize(): void {
    const ratio = Math.min(this.maxPixelRatio, Math.max(0.1, this.viewport.pixelRatio));
    this.renderer.setPixelRatio(ratio);
    this.renderer.setSize(Math.max(1, this.viewport.width), Math.max(1, this.viewport.height));
  }

  async warm(): Promise<void> {
    if (this.disposed) throw new Error('cannot warm a disposed renderer host');
    await this.renderer.compileAsync(this.scene, this.camera);
  }

  seal(): void { this.sealMonitor.seal(); }

  render(): void {
    if (this.disposed) return;
    this.renderSubmissions++;
    void this.renderer.renderAsync(this.scene, this.camera);
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
