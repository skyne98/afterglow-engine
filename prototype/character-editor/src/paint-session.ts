import { hasNativeWorkerTransport } from '../../../crates/afterglow-web/web/src/engine/workers/native-transport.ts';
import { NativePaint } from '../../../crates/afterglow-web/web/src/engine/paint/native-paint.ts';
import { initializeWebProfiling } from '../../../crates/afterglow-web/web/src/engine/telemetry/web-profiling.ts';
import { attachProfiling } from '../../../crates/afterglow-web/web/src/engine/telemetry/profiling.ts';
import { TelemetryRecorder, TelemetryDescriptorKind } from '../../../crates/afterglow-telemetry/web/src/telemetry.ts';
import profilingWorkerUrl from '../../../crates/afterglow-web/web/src/workers/diagnostics-worker.ts?worker&url';
import profilingTransportUrl from '../../../crates/afterglow-web/web/assets/afterglow_rpc.wasm?url';

const WEB_TRACE = [
  { category: 0, categoryName: 'paint', name: 'paint.stroke_sample', kind: TelemetryDescriptorKind.Instant, argument0: 'sample_count' },
  { category: 0, categoryName: 'paint', name: 'paint.worker_message', kind: TelemetryDescriptorKind.Instant },
];

/** The editor uses the same commands on both targets. */
export class PaintSession {
  onmessage: ((event: { data: any }) => void) | null = null;
  onerror: ((event: { message: string }) => void) | null = null;
  private readonly backend: Worker | NativePaint;
  private initial = true;
  private readonly trace: TelemetryRecorder | null;

  constructor(private readonly canvas: HTMLCanvasElement) {
    const native = hasNativeWorkerTransport();
    this.backend = native ? new NativePaint(canvas)
      : new Worker(new URL('./paint-engine-worker.ts', import.meta.url), { type: 'module' });
    this.trace = native ? null : new TelemetryRecorder(WEB_TRACE, new ArrayBuffer(1024 * 40));
    if (!native) {
      void initializeWebProfiling({ workerUrl: profilingWorkerUrl, transportUrl: profilingTransportUrl,
        serverUrl: new URL(location.href).searchParams.get('profilingServer') ?? undefined })
        .then(ready => { if (ready && this.trace) attachProfiling('afterglow-engine.paint', this.trace); });
    }
    this.backend.onmessage = (event: { data: any }) => {
      this.trace?.instant(1, 0, 0, 0);
      this.onmessage?.(event);
    };
    this.backend.onerror = (event: { message: string }) => this.onerror?.(event);
  }

  flush(): Promise<void> {
    return this.backend instanceof NativePaint ? this.backend.flush() : Promise.resolve();
  }

  send(message: any, transfer: Transferable[] = []): void {
    if (this.backend instanceof NativePaint) {
      // Native admission uses installed RAM, not the browser deviceMemory cap.
      this.backend.send(message.cmd === 'init'
        ? { ...message, memoryLimitMiB: message.nativeMemoryLimitMiB }
        : message);
      return;
    }
    if (message.cmd === 'strokeSample') this.trace?.instant(0, 0, 1, 0);
    if (message.cmd === 'init' && this.initial) {
      this.initial = false;
      const canvas = this.canvas.transferControlToOffscreen();
      this.backend.postMessage({ ...message, canvas }, [canvas]);
    } else this.backend.postMessage(message, transfer);
  }
}
