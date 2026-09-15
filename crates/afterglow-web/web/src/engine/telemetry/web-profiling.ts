import { CapturePump, RecorderSource } from '../../../../../afterglow-telemetry/web/src/capture.ts';
import type { TelemetryRecorder } from './telemetry.ts';
import { DiagnosticsClient } from '../../workers/diagnostics.client.ts';
import { Rpc } from '../../workers/rpc.ts';
import { hasNativeWorkerTransport } from '../workers/native-transport.ts';

interface ProfilingOwner {
  add(name: string, recorder: TelemetryRecorder): boolean;
  stats(): { connected: boolean; failures: number; batches: number; bytes: number };
}
const host = globalThis as typeof globalThis & { afterglowProfiling?: ProfilingOwner };
let initialization: Promise<boolean> | null = null;

/** Bootstrap-only. The browser worker owns WebSocket I/O and uses the existing RPC rings. */
export function initializeWebProfiling(options: {
  workerUrl: string;
  transportUrl: string;
  serverUrl?: string;
}): Promise<boolean> {
  if (hasNativeWorkerTransport() || host.afterglowProfiling) return Promise.resolve(!!host.afterglowProfiling);
  if (initialization) return initialization;
  initialization = initialize(options);
  return initialization;
}
async function initialize(options: { workerUrl: string; transportUrl: string; serverUrl?: string }): Promise<boolean> {
  if (!globalThis.crossOriginIsolated || typeof SharedArrayBuffer === 'undefined') return false;
  let rpc: Rpc | null = null;
  try {
    rpc = await Rpc.create({ mainWasmUrl: options.transportUrl, workerJsUrl: options.workerUrl,
      workerWasmUrl: '', workerInit: { url: options.serverUrl ?? 'ws://127.0.0.1:8086/' } });
    const pump = new CapturePump(new DiagnosticsClient(rpc));
    const recorders = new WeakSet<TelemetryRecorder>();
    let nextSource = 1;
    const owner: ProfilingOwner = {
      add(name, recorder) {
        if (nextSource > 8 || recorders.has(recorder)) return false;
        try {
          if (!pump.add(new RecorderSource(nextSource, name, recorder))) return false;
          nextSource++; recorders.add(recorder); return true;
        } catch { return false; }
      },
      stats: () => ({ connected: pump.connected, failures: pump.failures, batches: pump.batches, bytes: pump.bytes }),
    };
    Object.defineProperty(host, 'afterglowProfiling', { value: Object.freeze(owner) });
    async function tick(): Promise<void> { await pump.step(); setTimeout(tick, 50); }
    setTimeout(tick, 50);
    return true;
  } catch {
    rpc?.terminate();
    return false;
  }
}
