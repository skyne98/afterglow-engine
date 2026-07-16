import type * as THREE from 'three/webgpu';

interface PipelineBackend {
  createRenderPipeline(renderObject: unknown, promises: Promise<unknown>[]): void;
  createComputePipeline(computePipeline: unknown, bindings: unknown): void;
}

export interface RendererWarmVariant {
  scene: THREE.Scene;
  camera: THREE.Camera;
}

/** Bootstrap-installed monitor for pipeline creation after renderer seal. */
export class RendererSeal {
  private sealed = false;
  renderPipelines = 0;
  computePipelines = 0;
  renderPipelineViolations = 0;
  computePipelineViolations = 0;

  constructor(private readonly backend: PipelineBackend) {
    const originalRender = backend.createRenderPipeline.bind(backend);
    const originalCompute = backend.createComputePipeline.bind(backend);
    const monitor = this;
    backend.createRenderPipeline = function (renderObject: unknown, promises: Promise<unknown>[]): void {
      monitor.renderPipelines++;
      if (monitor.sealed) monitor.renderPipelineViolations++;
      originalRender(renderObject, promises);
    };
    backend.createComputePipeline = function (computePipeline: unknown, bindings: unknown): void {
      monitor.computePipelines++;
      if (monitor.sealed) monitor.computePipelineViolations++;
      originalCompute(computePipeline, bindings);
    };
  }

  seal(): void { this.sealed = true; }
  get isSealed(): boolean { return this.sealed; }
  get violations(): number { return this.renderPipelineViolations + this.computePipelineViolations; }

  assertNoViolations(): void {
    if (this.violations !== 0)
      throw new Error(`renderer created ${this.violations} pipeline(s) after seal`);
  }
}

/** Compile every declared scene/camera material variant before gameplay. */
export async function warmRendererVariants(
  renderer: { compileAsync(scene: THREE.Scene, camera: THREE.Camera): Promise<unknown> },
  variants: readonly RendererWarmVariant[],
): Promise<void> {
  for (const variant of variants) await renderer.compileAsync(variant.scene, variant.camera);
}
