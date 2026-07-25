import { describe, expect, test } from 'bun:test';
import {
  EngineTelemetry,
  TelemetryCaptureState,
  TelemetryDescriptorKind,
  TelemetryMetricBank,
  TelemetryMetricKind,
  TelemetryMetricStatus,
  TelemetryPhase,
  TelemetryRecorder,
  TelemetryRecordStatus,
  TELEMETRY_BATCH_HEADER_BYTES,
  TELEMETRY_HISTOGRAM_BUCKETS,
  TELEMETRY_RECORD_BYTES,
  type TelemetryDescriptor,
  type TelemetryMetricDescriptor,
} from './telemetry.ts';

const descriptors: readonly TelemetryDescriptor[] = [
  { category: 1, categoryName: 'io', name: 'pread', kind: TelemetryDescriptorKind.Span, argument0: 'offset', argument1: 'bytes' },
  { category: 1, categoryName: 'io', name: 'complete', kind: TelemetryDescriptorKind.Instant, argument0: 'bytes' },
  { category: 7, categoryName: 'gpu', name: 'upload', kind: TelemetryDescriptorKind.AsyncSpan },
  { category: 1, categoryName: 'rpc', name: 'request', kind: TelemetryDescriptorKind.Flow },
];

function readU53(words: Uint32Array, offset: number): number {
  return (words[offset] ?? 0) + (words[offset + 1] ?? 0) * 0x1_0000_0000;
}

describe('TelemetryRecorder', () => {
  test('requires fixed whole-record storage', () => {
    expect(() => new TelemetryRecorder(descriptors, new ArrayBuffer(0))).toThrow(RangeError);
    expect(() => new TelemetryRecorder(descriptors, new ArrayBuffer(TELEMETRY_RECORD_BYTES + 1))).toThrow(RangeError);
  });

  test('does not read the clock while disabled or category-filtered', () => {
    let clockReads = 0;
    const recorder = new TelemetryRecorder(
      descriptors,
      new ArrayBuffer(TELEMETRY_RECORD_BYTES),
      () => { clockReads++; return 10; },
    );
    expect(recorder.instant(1, 0)).toBe(TelemetryRecordStatus.Disabled);
    const categories = new Uint32Array(1);
    categories[0] = 1 << 7;
    expect(recorder.arm(4, categories)).toBe(true);
    expect(recorder.instant(1, 0)).toBe(TelemetryRecordStatus.CategoryDisabled);
    expect(clockReads).toBe(0);
  });

  test('writes the exact 40-byte ABI and preserves records when full', () => {
    let tick = 0x1_0000_0002;
    const buffer = new ArrayBuffer(TELEMETRY_RECORD_BYTES * 2);
    const recorder = new TelemetryRecorder(descriptors, buffer, () => tick++);
    expect(recorder.arm(12)).toBe(true);
    expect(recorder.spanBegin(0, 0x2_0000_0003, 17, 29)).toBe(TelemetryRecordStatus.Recorded);
    expect(recorder.spanEnd(0, 0x2_0000_0003, 31, 37)).toBe(TelemetryRecordStatus.Recorded);
    expect(recorder.instant(1, 1)).toBe(TelemetryRecordStatus.CapacityExceeded);
    expect(recorder.stop()).toBe(true);

    const words = new Uint32Array(buffer);
    expect(readU53(words, 0)).toBe(0x1_0000_0002);
    expect(readU53(words, 2)).toBe(0x2_0000_0003);
    expect(readU53(words, 4)).toBe(17);
    expect(readU53(words, 6)).toBe(29);
    expect(words[8]).toBe(0);
    expect(words[9]).toBe(TelemetryPhase.SpanBegin);
    const snapshot = recorder.snapshot();
    expect(snapshot?.epoch).toBe(12);
    expect(snapshot?.count).toBe(2);
    expect(snapshot?.dropped).toBe(1);
    expect(snapshot?.buffer).toBe(buffer);
    expect(recorder.state).toBe(TelemetryCaptureState.Frozen);

    const batch = new Uint8Array(recorder.encodedBatchBytes());
    expect(recorder.encodeBatchInto(batch, 9, 4)).toBe(batch.length);
    expect(new TextDecoder().decode(batch.subarray(0, 4))).toBe('AGTB');
    const headerWords = new Uint32Array(batch.buffer, 8, (TELEMETRY_BATCH_HEADER_BYTES - 8) / 4);
    expect(headerWords[0]).toBe(9);
    expect(headerWords[1]).toBe(12);
    expect(headerWords[2]).toBe(4);
    expect(headerWords[4]).toBe(2);
    expect(headerWords[5]).toBe(1);
    expect(batch.slice(TELEMETRY_BATCH_HEADER_BYTES)).toEqual(new Uint8Array(buffer));
  });

  test('validates descriptor kind and capture transitions', () => {
    const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(TELEMETRY_RECORD_BYTES));
    expect(recorder.stop()).toBe(false);
    expect(recorder.arm(0x1_0000_0000)).toBe(false);
    expect(recorder.arm(1)).toBe(true);
    expect(recorder.instant(0, 0)).toBe(TelemetryRecordStatus.WrongDescriptorKind);
    expect(recorder.asyncBegin(2, 8)).toBe(TelemetryRecordStatus.Recorded);
    expect(recorder.stop()).toBe(true);
    expect(recorder.encodeBatchInto(new Uint8Array(80), -1, 1)).toBe(0);
    expect(recorder.reset()).toBe(true);
    expect(recorder.state).toBe(TelemetryCaptureState.Idle);
  });
});

const metricDescriptors: readonly TelemetryMetricDescriptor[] = [
  { category: 1, categoryName: 'io', name: 'bytes', kind: TelemetryMetricKind.Counter },
  { category: 1, categoryName: 'io', name: 'pending', kind: TelemetryMetricKind.Gauge },
  { category: 1, categoryName: 'io', name: 'max', kind: TelemetryMetricKind.Maximum },
  { category: 1, categoryName: 'io', name: 'latency', kind: TelemetryMetricKind.HistogramLog2 },
];

describe('TelemetryMetricBank', () => {
  test('uses fixed cells for scalar and histogram metrics', () => {
    const cells = new Float64Array(3 + TELEMETRY_HISTOGRAM_BUCKETS);
    const metrics = new TelemetryMetricBank(metricDescriptors, cells);
    expect(metrics.counterAdd(0, 5)).toBe(TelemetryMetricStatus.Updated);
    expect(metrics.counterAdd(0, 7)).toBe(TelemetryMetricStatus.Updated);
    expect(metrics.gaugeSet(1, -3)).toBe(TelemetryMetricStatus.Updated);
    expect(metrics.maximum(2, 20)).toBe(TelemetryMetricStatus.Updated);
    expect(metrics.maximum(2, 10)).toBe(TelemetryMetricStatus.Updated);
    expect(metrics.histogramLog2(3, 8)).toBe(TelemetryMetricStatus.Updated);
    expect(metrics.counterAdd(1, 1)).toBe(TelemetryMetricStatus.WrongMetricKind);
    expect(metrics.readCell(0)).toBe(12);
    expect(metrics.readCell(1)).toBe(-3);
    expect(metrics.readCell(2)).toBe(20);
    expect(metrics.readCell(3, 3)).toBe(1);
  });

  test('engine facade uses caller-owned EngineMemory-compatible storage', () => {
    const trace = new ArrayBuffer(TELEMETRY_RECORD_BYTES);
    const cells = new Float64Array(3 + TELEMETRY_HISTOGRAM_BUCKETS);
    const telemetry = new EngineTelemetry(descriptors, metricDescriptors, trace, cells, () => 1);
    expect(telemetry.trace.buffer).toBe(trace);
    expect(telemetry.metrics.cells).toBe(cells);
  });
});
