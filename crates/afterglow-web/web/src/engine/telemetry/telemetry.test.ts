import { expect, test } from 'bun:test';
import { Telemetry } from '../../../../../afterglow-telemetry/web/src/telemetry.ts';
import {
  EngineTelemetry, TelemetryRes, TelemetryCaptureState, TELEMETRY_RECORD_BYTES,
} from './telemetry.ts';

test('the engine uses the generic producer without a second implementation', () => {
  expect(EngineTelemetry).toBe(Telemetry);
  const buffer = new ArrayBuffer(TELEMETRY_RECORD_BYTES);
  const cells = new Float64Array(0);
  const engine = new EngineTelemetry([], [], buffer, cells, () => 1);
  expect(engine.trace.buffer).toBe(buffer);
  expect(engine.metrics.cells).toBe(cells);
  expect(engine.trace.state).toBe(TelemetryCaptureState.Idle);
  expect(TelemetryRes).toBeDefined();
});
