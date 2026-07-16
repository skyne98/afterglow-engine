/**
 * WebGPU-only renderer bootstrap.
 *
 * Three's WebGPURenderer silently installs a WebGL2 fallback callback. The
 * engine must fail closed instead: a renderer is usable only after an adapter,
 * device, and WebGPU backend have all been verified.
 */

export type WebGPUOnlyRenderer = {
  backend: {
    isWebGPUBackend?: boolean;
    device?: unknown;
    createRenderPipeline: (renderObject: unknown, promises: Promise<unknown>[]) => void;
    createComputePipeline: (computePipeline: unknown, bindings: unknown) => void;
  };
  afterglowAdapterInfo?: { vendor?: string; architecture?: string; device?: string; description?: string };
  _getFallback: unknown;
  onDeviceLost: (info: { api?: string; message?: string; reason?: string | null }) => void;
  init: () => Promise<void>;
  compileAsync: (scene: unknown, camera: unknown) => Promise<unknown>;
  render: (scene: unknown, camera: unknown) => void;
  /** Legacy demos only; canonical RendererHost uses render() after init. */
  renderAsync: (scene: unknown, camera: unknown) => Promise<unknown>;
  setPixelRatio: (ratio: number) => void;
  setSize: (width: number, height: number) => void;
  domElement: HTMLCanvasElement;
  dispose: () => void;
};

export function disableWebGLFallback(renderer: WebGPUOnlyRenderer): void {
  // Three Renderer.init() uses this private callback to replace its WebGPU
  // backend with WebGLBackend after any initialization failure. r185 exposes no
  // public WebGPU-required option, so clear it before init and assert below.
  renderer._getFallback = null;
}

export function assertWebGPUBackend(renderer: WebGPUOnlyRenderer): void {
  if (renderer.backend?.isWebGPUBackend !== true || renderer.backend.device == null) {
    throw new Error('Afterglow requires a live WebGPU backend; WebGL fallback is forbidden.');
  }
}

export function showWebGPUFailure(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  const panel = document.createElement('pre');
  panel.id = 'afterglow-webgpu-failure';
  panel.textContent = `Afterglow requires hardware WebGPU.\n\n${message}`;
  panel.style.cssText = 'box-sizing:border-box;margin:0;min-height:100vh;padding:24px;background:#11151c;color:#ff9a9a;font:16px/1.5 ui-monospace,monospace;white-space:pre-wrap';
  document.body.replaceChildren(panel);
  console.error('Afterglow WebGPU startup failed:', error);
}

export type WebGPUOnlyRendererFactory = (parameters: Record<string, unknown>) => WebGPUOnlyRenderer;

/** Temporary bridge for legacy demos; canonical code uses RendererHost. */
export const legacyWindowRendererFactory: WebGPUOnlyRendererFactory = parameters => {
  const legacyWindow = window as unknown as {
    THREE: { WebGPURenderer: new (options: Record<string, unknown>) => WebGPUOnlyRenderer };
  };
  return new legacyWindow.THREE.WebGPURenderer(parameters);
};

/** Create a renderer that can never initialize or continue under WebGL. */
export async function createWebGPUOnlyRenderer(
  parameters: Record<string, unknown> = {},
  factory: WebGPUOnlyRendererFactory,
): Promise<WebGPUOnlyRenderer> {
  const gpu = navigator.gpu;
  if (!gpu) throw new Error('navigator.gpu is unavailable. WebGL fallback is disabled.');

  const adapter = await gpu.requestAdapter();
  if (!adapter) throw new Error('Unable to acquire a hardware WebGPU adapter. WebGL fallback is disabled.');

  // Let Three request the device so it retains every adapter feature it normally
  // enables (for example BC/ASTC texture compression). init() below is still
  // fail-closed because its fallback callback has been cleared.
  const renderer = factory(parameters);
  renderer.afterglowAdapterInfo = adapter.info;
  disableWebGLFallback(renderer);

  try {
    await renderer.init();
    assertWebGPUBackend(renderer);
  } catch (error) {
    renderer.dispose();
    throw error;
  }

  const onDeviceLost = renderer.onDeviceLost.bind(renderer);
  renderer.onDeviceLost = info => {
    onDeviceLost(info);
    showWebGPUFailure(new Error(`WebGPU device lost (${info.reason ?? 'unknown'}): ${info.message ?? 'no detail'}`));
  };

  return renderer;
}
