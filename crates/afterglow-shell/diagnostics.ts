import { CapturePump, RecorderSource } from '../afterglow-telemetry/web/src/capture.ts';
import type { TelemetryRecorder } from '../afterglow-telemetry/web/src/telemetry.ts';
import { DiagnosticsClient } from '../afterglow-web/web/src/workers/diagnostics.client.ts';
import { NativeRpcTransport, nativeWorkerIds } from '../afterglow-web/web/src/engine/workers/native-transport.ts';

declare const Deno: { core: { ops: {
  op_diagnostics_capture(epoch: number, a: number, b: number, c: number, d: number): string;
  op_diagnostics_drain(): Uint8Array;
} } };

const ids = nativeWorkerIds('diagnostics');
if (ids.length === 1) {
  const ops = Deno.core.ops;
  const pump = new CapturePump(new DiagnosticsClient(new NativeRpcTransport(ids[0]!)));
  const encoder = new TextEncoder();
  pump.add({
    register: (epoch, session) => encoder.encode(ops.op_diagnostics_capture(epoch, session[0], session[1], session[2], session[3])),
    drain: () => ops.op_diagnostics_drain(),
    stop: () => { ops.op_diagnostics_capture(0, 0, 0, 0, 0); },
  });
  let nextSource = 1;
  const recorders = new WeakSet<TelemetryRecorder>();
  const profiling = {
    add(name: string, recorder: TelemetryRecorder): boolean {
      if (nextSource > 7 || recorders.has(recorder)) return false;
      try {
        const source = new RecorderSource(nextSource, name, recorder);
        if (!pump.add(source)) return false;
        recorders.add(recorder);
        nextSource++;
        return true;
      } catch { return false; }
    },
    stats: () => ({ connected: pump.connected, failures: pump.failures, batches: pump.batches, bytes: pump.bytes }),
  };
  Object.defineProperty(globalThis, 'afterglowProfiling', { value: Object.freeze(profiling) });
  // This cold task never blocks a frame and never overlaps another transfer.
  async function tick(): Promise<void> {
    await pump.step();
    setTimeout(tick, 50);
  }
  setTimeout(tick, 50);
}
