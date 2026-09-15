import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { decodeTelemetryBatch } from './batch.ts';
import { TelemetryRecorder, TelemetryDescriptorKind } from './telemetry.ts';

function fixture(): Uint8Array {
  const hex = readFileSync(new URL('../../tests/fixtures/rolling-batch.hex', import.meta.url), 'utf8').replace(/\s/g, '');
  return new Uint8Array(Buffer.from(hex, 'hex'));
}

test('DGTB retains identities, raw ticks, sequence and overwrite counts', () => {
  const batch = decodeTelemetryBatch(fixture(), 3);
  expect(batch.identity).toEqual({ session: [1, 2, 3, 4], sourceId: 1, generation: 1, clockDomain: 1, clockGeneration: 1 });
  expect(batch.firstSequence).toBe(2n);
  expect(batch.nextSequence).toBe(5n);
  expect(batch.overwritten).toBe(2n);
  expect(batch.rolling).toBe(true);
  expect(batch.records.map(record => record.timestamp)).toEqual([120n, 130n, 140n]);
  const bytes = fixture();
  new DataView(bytes.buffer).setBigUint64(104, 0xffff_ffff_ffff_ffffn, true);
  expect(decodeTelemetryBatch(bytes, 3).records[0]!.correlation).toBe(0xffff_ffff_ffff_ffffn);
});

test('DGTB rejects old formats, malformed framing, identities and records', () => {
  const valid = fixture();
  for (let length = 0; length < valid.length; length++) expect(() => decodeTelemetryBatch(valid.slice(0, length), 3)).toThrow();
  for (const magic of ['AGTB', 'AGTL']) {
    const bytes = valid.slice();
    bytes.set(new TextEncoder().encode(magic));
    expect(() => decodeTelemetryBatch(bytes, 3)).toThrow('magic');
  }
  for (const [offset, value] of [[4, 2], [6, 0], [20, 2], [28, 1], [56, 0], [60, 0], [64, 9], [72, 9], [88, 9], [132, 9], [133, 1], [134, 1], [176, 0]]) {
    const bytes = valid.slice(); bytes[offset!] = value!;
    expect(() => decodeTelemetryBatch(bytes, 3)).toThrow();
  }
  const noIdentity = valid.slice(); noIdentity.fill(0, 40, 56);
  expect(() => decodeTelemetryBatch(noIdentity, 3)).toThrow('identity');
  expect(() => decodeTelemetryBatch(valid, 2)).toThrow('capacity');
  expect(() => decodeTelemetryBatch(new Uint8Array([...valid, 0]), 3)).toThrow('length');
  for (const cap of [-1, NaN, Infinity, 0.5]) expect(() => decodeTelemetryBatch(valid, cap)).toThrow(RangeError);
});

test('invalid producer identities cannot change output', () => {
  const recorder = new TelemetryRecorder([{ category: 0, categoryName: 'app', name: 'event', kind: TelemetryDescriptorKind.Instant }], new ArrayBuffer(40), () => 1);
  recorder.arm(1); recorder.instant(0, 1); recorder.stop();
  const identity = { session: [1, 0, 0, 0] as const, sourceId: 1, generation: 1, clockDomain: 1, clockGeneration: 1 };
  for (const change of [{ generation: 0 }, { clockGeneration: 0 }, { session: [0, 0, 0, 0] as const }, { sourceId: -1 }, { clockDomain: 2 ** 32 }]) {
    const output = new Uint8Array(recorder.encodedBatchBytes()).fill(0xa5);
    expect(recorder.encodeBatchInto(output, { ...identity, ...change })).toBe(0);
    expect(output.every(value => value === 0xa5)).toBe(true);
  }
});
