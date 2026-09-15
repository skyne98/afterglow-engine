import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  Telemetry,
  TelemetryCaptureRetention,
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

const identity = { session: [1, 2, 3, 4] as const, sourceId: 1, generation: 1, clockDomain: 1, clockGeneration: 1 };
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
    expect(new TelemetryRecorder(descriptors, new ArrayBuffer(TELEMETRY_RECORD_BYTES)).ticksPerSecond).toBe(1_000_000_000);
    for (const rate of [0, -1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
      expect(() => new TelemetryRecorder(descriptors, new ArrayBuffer(40), () => 0, rate)).toThrow(RangeError);
    }
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
    expect(recorder.encodeBatchInto(batch, { ...identity, sourceId: 9, clockDomain: 4 })).toBe(batch.length);
    expect(new TextDecoder().decode(batch.subarray(0, 4))).toBe('DGTB');
    const headerWords = new Uint32Array(batch.buffer, 8, (TELEMETRY_BATCH_HEADER_BYTES - 8) / 4);
    expect(headerWords[0]).toBe(9);
    expect(headerWords[1]).toBe(12);
    expect(headerWords[2]).toBe(4);
    expect(headerWords[4]).toBe(2);
    expect(headerWords[5]).toBe(0);
    expect(new DataView(batch.buffer).getBigUint64(80, true)).toBe(1n);
    expect(batch.slice(TELEMETRY_BATCH_HEADER_BYTES)).toEqual(new Uint8Array(buffer));
  });

  test('invalid or aliased batch output never changes either buffer', () => {
    let tick = 10;
    const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(280), () => tick++);
    recorder.arm(1);
    recorder.instant(1, 0);
    recorder.instant(1, 0);
    recorder.stop();
    const bytes = new Uint8Array(recorder.buffer);
    const original = bytes.slice();
    expect(recorder.encodeBatchInto(bytes, identity)).toBe(0);
    expect(bytes).toEqual(original);
    for (const [offset, value] of [[76, 0], [76, 9], [77, 1], [78, 1], [40, 1]]) {
      bytes.set(original);
      bytes[offset!] = value!;
      const output = new Uint8Array(recorder.encodedBatchBytes()).fill(0xa5);
      expect(recorder.encodeBatchInto(output, identity)).toBe(0);
      expect(output.every(byte => byte === 0xa5)).toBe(true);
    }
    bytes.set(original);
    // The same allocation is safe when the source and output do not overlap.
    expect(recorder.encodeBatchInto(new Uint8Array(recorder.buffer, 80), identity)).toBe(176);
    expect(bytes.slice(0, 80)).toEqual(original.slice(0, 80));
  });

  test('retains chronological rolling windows at every wrap position', () => {
    for (let capacity = 1; capacity <= 9; capacity++) {
      for (let writes = 0; writes <= capacity * 4 + 1; writes++) {
        let tick = 100;
        const buffer = new ArrayBuffer(capacity * TELEMETRY_RECORD_BYTES);
        const recorder = new TelemetryRecorder(descriptors, buffer, () => tick++);
        expect(recorder.arm(19, undefined, TelemetryCaptureRetention.Rolling)).toBe(true);
        for (let index = 0; index < writes; index++) {
          expect(recorder.instant(1, index, index)).toBe(TelemetryRecordStatus.Recorded);
        }
        expect(recorder.stop()).toBe(true);
        const snapshot = recorder.snapshot()!;
        expect(snapshot.count).toBe(Math.min(writes, capacity));
        expect(snapshot.overwritten).toBe(Math.max(0, writes - capacity));
        expect(snapshot.dropped).toBe(0);
        expect(snapshot.retention).toBe(TelemetryCaptureRetention.Rolling);
        const words = new Uint32Array(buffer);
        for (let offset = 0; offset < snapshot.count; offset++) {
          const index = Math.max(0, writes - capacity) + offset;
          expect(readU53(words, offset * 10)).toBe(100 + index);
          expect(readU53(words, offset * 10 + 2)).toBe(index);
          expect(readU53(words, offset * 10 + 4)).toBe(index);
          expect(words[offset * 10 + 8]).toBe(1);
          expect(words[offset * 10 + 9]).toBe(TelemetryPhase.Instant);
        }
        const output = new Uint8Array(recorder.encodedBatchBytes()).fill(0xa5);
        expect(recorder.encodeBatchInto(output, identity)).toBe(output.length);
        const header = new DataView(output.buffer);
        expect(header.getUint32(20, true)).toBe(1);
        expect(header.getBigUint64(64, true)).toBe(BigInt(snapshot.overwritten));
        expect(header.getBigUint64(72, true)).toBe(BigInt(writes));
        expect(header.getBigUint64(88, true)).toBe(BigInt(snapshot.overwritten));
        expect(recorder.instant(1, 0)).toBe(TelemetryRecordStatus.Disabled);
        expect(recorder.reset()).toBe(true);
        expect(recorder.arm(20)).toBe(true);
        expect(recorder.stop()).toBe(true);
        expect(recorder.snapshot()?.count).toBe(0);
        expect(recorder.snapshot()?.overwritten).toBe(0);
        expect(recorder.snapshot()?.retention).toBe(TelemetryCaptureRetention.Prefix);
      }
    }
  });

  test('rolling record bytes match the shared Rust fixture', () => {
    let tick = 100;
    const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(3 * TELEMETRY_RECORD_BYTES), () => {
      const value = tick;
      tick += 10;
      return value;
    });
    recorder.arm(1, undefined, TelemetryCaptureRetention.Rolling);
    for (let index = 0; index < 5; index++) recorder.instant(1, index, index);
    recorder.stop();
    const hex = readFileSync(new URL('../../tests/fixtures/rolling-batch.hex', import.meta.url), 'utf8').replace(/\s/g, '');
    const output = new Uint8Array(recorder.encodedBatchBytes());
    expect(recorder.encodeBatchInto(output, identity)).toBe(output.length);
    expect(output).toEqual(new Uint8Array(Buffer.from(hex, 'hex')));
  });

  test('filtered and invalid events do not remove rolling history', () => {
    let reads = 0;
    const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(40), () => ++reads);
    const categories = new Uint32Array([1 << 1]);
    expect(recorder.arm(1, categories, TelemetryCaptureRetention.Rolling)).toBe(true);
    expect(recorder.instant(1, 7)).toBe(TelemetryRecordStatus.Recorded);
    expect(recorder.asyncBegin(2, 8)).toBe(TelemetryRecordStatus.CategoryDisabled);
    expect(recorder.instant(2, 8)).toBe(TelemetryRecordStatus.WrongDescriptorKind);
    expect(recorder.instant(99, 8)).toBe(TelemetryRecordStatus.InvalidDescriptor);
    expect(reads).toBe(1);
    expect(recorder.overwritten).toBe(0);
    recorder.stop();
    expect(readU53(new Uint32Array(recorder.buffer), 2)).toBe(7);
  });

  test('validates descriptor kind and capture transitions', () => {
    const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(TELEMETRY_RECORD_BYTES));
    expect(recorder.stop()).toBe(false);
    expect(recorder.arm(0x1_0000_0000)).toBe(false);
    expect(recorder.arm(1, undefined, 99 as TelemetryCaptureRetention)).toBe(false);
    expect(recorder.arm(1)).toBe(true);
    expect(recorder.instant(0, 0)).toBe(TelemetryRecordStatus.WrongDescriptorKind);
    expect(recorder.asyncBegin(2, 8)).toBe(TelemetryRecordStatus.Recorded);
    expect(recorder.stop()).toBe(true);
    expect(recorder.encodeBatchInto(new Uint8Array(80), { ...identity, sourceId: -1 })).toBe(0);
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

test('independent applications do not share capture or metric state', () => {
  const service = new Telemetry(descriptors, metricDescriptors,
    new ArrayBuffer(TELEMETRY_RECORD_BYTES * 2), new Float64Array(35), () => 11);
  const command = new Telemetry(descriptors, metricDescriptors,
    new ArrayBuffer(TELEMETRY_RECORD_BYTES * 2), new Float64Array(35), () => 22);
  expect(service.trace.arm(1)).toBe(true);
  expect(service.trace.instant(1, 1)).toBe(TelemetryRecordStatus.Recorded);
  expect(command.trace.instant(1, 1)).toBe(TelemetryRecordStatus.Disabled);
  service.metrics.counterAdd(0, 7);
  expect(command.metrics.readCell(0)).toBe(0);
  expect(command.trace.arm(2)).toBe(true);
  expect(command.trace.instant(1, 1)).toBe(TelemetryRecordStatus.Recorded);
  service.trace.stop();
  expect(command.trace.state).toBe(TelemetryCaptureState.Armed);
  expect(service.trace.snapshot()?.epoch).toBe(1);
  command.trace.stop();
  expect(command.trace.snapshot()?.epoch).toBe(2);
  expect(new Uint32Array(service.trace.buffer)[0]).toBe(11);
  expect(new Uint32Array(command.trace.buffer)[0]).toBe(22);
});

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

  test('a standalone producer uses caller-owned storage', () => {
    const trace = new ArrayBuffer(TELEMETRY_RECORD_BYTES);
    const cells = new Float64Array(3 + TELEMETRY_HISTOGRAM_BUCKETS);
    const telemetry = new Telemetry(descriptors, metricDescriptors, trace, cells, () => 1);
    expect(telemetry.trace.buffer).toBe(trace);
    expect(telemetry.metrics.cells).toBe(cells);
  });
});
