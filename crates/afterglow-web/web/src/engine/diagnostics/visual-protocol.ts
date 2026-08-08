import {
  RuntimeReadinessStage,
  type EngineRuntime,
  type RuntimeReadinessSnapshot,
} from '../core/runtime.ts';
import type { RendererDimensionSnapshot } from '../renderer/renderer-host.ts';

export const AFTERGLOW_DIAGNOSTIC_VERSION = 1;
export const AFTERGLOW_DIAGNOSTIC_GLOBAL = '__afterglowDiagnosticV1';

export interface VisualDiagnosticSnapshot {
  version: number;
  readiness: RuntimeReadinessSnapshot;
  dimensions: RendererDimensionSnapshot;
  diagnostics: number;
  droppedDiagnostics: number;
  adapter: {
    vendor: string | undefined;
    architecture: string | undefined;
    device: string | undefined;
    description: string | undefined;
  };
  postSealPipelines: number;
  frameId: number;
}

export interface VisualDiagnosticProtocol {
  readonly version: 1;
  snapshot(): VisualDiagnosticSnapshot;
  waitForGameReady(timeoutMs?: number): Promise<VisualDiagnosticSnapshot>;
  shutdown(): Promise<void>;
}

function waitAnimationFrame(): Promise<void> {
  return new Promise(resolve => requestAnimationFrame(() => resolve()));
}

/** Install the sole versioned automation surface. Diagnostic bundles only. */
export function installVisualDiagnosticProtocol(runtime: EngineRuntime): VisualDiagnosticProtocol {
  const readiness: RuntimeReadinessSnapshot = {
    stage: RuntimeReadinessStage.Bootstrap,
    firstUpdateFrame: 0,
    firstPresentationFrame: 0,
    fatalDiagnostics: 0,
  };
  const dimensions: RendererDimensionSnapshot = {
    logicalWidth: 0, logicalHeight: 0, pixelRatio: 0,
    canvasWidth: 0, canvasHeight: 0, surfaceWidth: 0, surfaceHeight: 0,
    feedbackWidth: 0, feedbackHeight: 0, cameraAspect: 0,
  };
  const protocol: VisualDiagnosticProtocol = {
    version: AFTERGLOW_DIAGNOSTIC_VERSION,
    snapshot(): VisualDiagnosticSnapshot {
      runtime.readReadinessInto(readiness);
      const host = runtime.rendererHost;
      host.readDimensionsInto(dimensions);
      return {
        version: AFTERGLOW_DIAGNOSTIC_VERSION,
        readiness: { ...readiness },
        dimensions: { ...dimensions },
        diagnostics: runtime.diagnostics.count,
        droppedDiagnostics: runtime.diagnostics.dropped,
        adapter: { ...host.adapterInfo },
        postSealPipelines: host.sealMonitor.violations,
        frameId: runtime.frame.frameId,
      };
    },
    async waitForGameReady(timeoutMs = 30_000): Promise<VisualDiagnosticSnapshot> {
      const deadline = performance.now() + timeoutMs;
      for (;;) {
        runtime.readReadinessInto(readiness);
        if (readiness.stage === RuntimeReadinessStage.GameReady) return protocol.snapshot();
        if (readiness.stage === RuntimeReadinessStage.Fatal ||
            readiness.stage === RuntimeReadinessStage.Shutdown)
          throw new Error(`engine readiness failed at stage ${readiness.stage}`);
        if (performance.now() >= deadline)
          throw new Error(`engine readiness timed out at stage ${readiness.stage}`);
        await waitAnimationFrame();
      }
    },
    shutdown(): Promise<void> { return runtime.close(); },
  };
  Object.defineProperty(globalThis, AFTERGLOW_DIAGNOSTIC_GLOBAL, {
    configurable: true,
    enumerable: false,
    value: protocol,
  });
  return protocol;
}
