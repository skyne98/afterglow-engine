import { expect, test } from 'bun:test';
import { decodeTelemetryBatch } from './batch.ts';
import { TelemetryRecorder, TelemetryDescriptorKind, TelemetryCaptureRetention, TelemetryRecordStatus } from './telemetry.ts';

const descriptors = [{ category: 0, categoryName: 'app', name: 'sample', kind: TelemetryDescriptorKind.Instant }];
const identity = { session: [1, 2, 3, 4] as const, sourceId: 1, generation: 1, clockDomain: 1, clockGeneration: 1 };

test('continued capture retains identity, cumulative losses and unique sequences', () => {
  for (const rolling of [false, true]) {
    let tick = 0;
    const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(80), () => ++tick);
    expect(recorder.resume()).toBe(false);
    recorder.arm(1, undefined, rolling ? TelemetryCaptureRetention.Rolling : TelemetryCaptureRetention.Prefix);
    expect(recorder.resume()).toBe(false);
    let previous = 0n;
    for (let window = 0; window < 4; window++) {
      for (let index = 0; index < 3; index++) recorder.instant(0, window * 3 + index);
      recorder.stop();
      const bytes = new Uint8Array(recorder.encodedBatchBytes());
      expect(recorder.encodeBatchInto(bytes, identity)).toBe(bytes.length);
      const batch = decodeTelemetryBatch(bytes, 2);
      const first = BigInt(rolling ? window * 3 + 1 : window * 2);
      expect(batch.firstSequence).toBe(first);
      expect(batch.nextSequence).toBe(first + 2n);
      expect(batch.firstSequence >= previous).toBe(true);
      expect(batch.overwritten).toBe(BigInt(rolling ? window + 1 : 0));
      expect(batch.dropped).toBe(BigInt(rolling ? 0 : window + 1));
      expect(recorder.snapshot()!.firstSequence).toBe(Number(first));
      previous = batch.nextSequence;
      expect(recorder.resume()).toBe(true);
    }
    recorder.stop();
    const snapshot = recorder.snapshot()!;
    expect(snapshot.firstSequence).toBe(Number(previous));
    expect(snapshot.nextSequence).toBe(Number(previous));
    recorder.reset(); recorder.arm(2); recorder.stop();
    expect(recorder.snapshot()!.nextSequence).toBe(0);
    expect(recorder.snapshot()!.dropped).toBe(0);
    expect(recorder.snapshot()!.overwritten).toBe(0);
  }
});

test('sequence exhaustion cannot wrap or change retained records', () => {
  let clockCalls = 0;
  const recorder = new TelemetryRecorder(descriptors, new ArrayBuffer(40), () => ++clockCalls);
  recorder.arm(1, undefined, TelemetryCaptureRetention.Rolling);
  Object.assign(recorder, { nextSequence: Number.MAX_SAFE_INTEGER - 1 });
  expect(recorder.instant(0, 1)).toBe(TelemetryRecordStatus.Recorded);
  const before = recorder.buffer.slice(0);
  expect(recorder.instant(0, 2)).toBe(TelemetryRecordStatus.SequenceExhausted);
  expect(recorder.buffer).toEqual(before);
  expect(clockCalls).toBe(1);
  recorder.stop();
  expect(recorder.snapshot()!.nextSequence).toBe(Number.MAX_SAFE_INTEGER);
  recorder.resume();
  expect(recorder.instant(0, 3)).toBe(TelemetryRecordStatus.SequenceExhausted);
});
