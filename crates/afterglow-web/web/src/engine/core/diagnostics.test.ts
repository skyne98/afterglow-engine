import { describe, expect, test } from 'bun:test';
import {
  DiagnosticCode, DiagnosticSource, DiagnosticStatus, EngineDiagnostics,
  type DiagnosticRecord,
} from './diagnostics.ts';

function record(): DiagnosticRecord {
  return { sequence: 0, code: DiagnosticCode.Unknown, source: DiagnosticSource.Runtime, detail: null };
}

describe('EngineDiagnostics', () => {
  test('requires a positive fixed capacity', () => {
    expect(() => new EngineDiagnostics(0)).toThrow(RangeError);
    expect(() => new EngineDiagnostics(1.5)).toThrow(RangeError);
  });

  test('preserves FIFO order and caller-owned output', () => {
    const diagnostics = new EngineDiagnostics(2);
    const first = { message: 'first' };
    expect(diagnostics.tryRecord(DiagnosticCode.DeviceLost, DiagnosticSource.Renderer, first))
      .toBe(DiagnosticStatus.Recorded);
    expect(diagnostics.tryRecord(DiagnosticCode.WorkerFailure, DiagnosticSource.Worker, 42))
      .toBe(DiagnosticStatus.Recorded);
    expect(diagnostics.highWater).toBe(2);

    const out = record();
    expect(diagnostics.readInto(0, out)).toBe(true);
    expect(out.code).toBe(DiagnosticCode.DeviceLost);
    expect(out.detail).toBe(first);
    expect(diagnostics.shiftInto(out)).toBe(true);
    expect(out.sequence).toBe(1);
    expect(diagnostics.shiftInto(out)).toBe(true);
    expect(out.sequence).toBe(2);
    expect(diagnostics.shiftInto(out)).toBe(false);
  });

  test('drops newest deterministically when full and reuses released slots', () => {
    const diagnostics = new EngineDiagnostics(1);
    const out = record();
    expect(diagnostics.tryRecord(DiagnosticCode.RuntimeState, DiagnosticSource.Runtime, 'kept'))
      .toBe(DiagnosticStatus.Recorded);
    expect(diagnostics.tryRecord(DiagnosticCode.Unknown, DiagnosticSource.Game, 'dropped'))
      .toBe(DiagnosticStatus.CapacityExceeded);
    expect(diagnostics.dropped).toBe(1);
    expect(diagnostics.shiftInto(out)).toBe(true);
    expect(out.detail).toBe('kept');
    expect(diagnostics.tryRecord(DiagnosticCode.Unknown, DiagnosticSource.Game, 'next'))
      .toBe(DiagnosticStatus.Recorded);
    expect(diagnostics.readInto(0, out)).toBe(true);
    expect(out.detail).toBe('next');
  });

  test('clear releases details without resetting stable telemetry', () => {
    const diagnostics = new EngineDiagnostics(2);
    diagnostics.tryRecord(DiagnosticCode.Unknown, DiagnosticSource.Game, {});
    diagnostics.clear();
    expect(diagnostics.count).toBe(0);
    expect(diagnostics.highWater).toBe(1);
  });
});
