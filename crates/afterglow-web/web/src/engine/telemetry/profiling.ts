import type { TelemetryRecorder } from './telemetry.ts';

/** Attach a caller-owned recorder during bootstrap. EngineMemory owns engine storage. */
export function attachProfiling(name: string, recorder: TelemetryRecorder): boolean {
  const owner = (globalThis as typeof globalThis & {
    afterglowProfiling?: { add(name: string, recorder: TelemetryRecorder): boolean };
  }).afterglowProfiling;
  return owner?.add(name, recorder) ?? false;
}
