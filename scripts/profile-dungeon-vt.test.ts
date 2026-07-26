import { describe, expect, test } from 'bun:test';
import { aggregateAgtb, validateAgtb } from './profile-dungeon-vt.ts';

const HEADER_BYTES = 40;
const RECORD_BYTES = 40;

function batch(records: Array<{ timestamp: number; correlation: number; descriptor: number; phase: number; argument0?: number; argument1?: number }>): Uint8Array {
  const bytes = new Uint8Array(HEADER_BYTES + records.length * RECORD_BYTES);
  const view = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode('AGTB'));
  view.setUint16(4, 1, true);
  view.setUint16(6, HEADER_BYTES, true);
  view.setUint32(8, 1, true);
  view.setUint32(12, 20260725, true);
  view.setUint32(16, 1, true);
  view.setUint32(24, records.length, true);
  view.setBigUint64(32, 1_000_000_000n, true);
  for (let index = 0; index < records.length; index++) {
    const record = records[index]!;
    const offset = HEADER_BYTES + index * RECORD_BYTES;
    view.setBigUint64(offset, BigInt(record.timestamp), true);
    view.setBigUint64(offset + 8, BigInt(record.correlation), true);
    view.setBigUint64(offset + 16, BigInt(record.argument0 ?? 0), true);
    view.setBigUint64(offset + 24, BigInt(record.argument1 ?? 0), true);
    view.setUint32(offset + 32, record.descriptor, true);
    view.setUint8(offset + 36, record.phase);
  }
  return bytes;
}

describe('Dungeon VT profile AGTB decoder', () => {
  test('validates the complete batch header and exact byte length', () => {
    const bytes = batch([]);
    expect(validateAgtb(bytes)).toEqual({
      sourceId: 1,
      epoch: 20260725,
      clockDomain: 1,
      flags: 0,
      recordCount: 0,
      droppedRecords: 0,
      ticksPerSecond: 1_000_000_000,
    });
    expect(() => validateAgtb(bytes.subarray(0, bytes.length - 1))).toThrow('shorter');
    const wrongRate = batch([]);
    new DataView(wrongRate.buffer).setBigUint64(32, 1_000_000n, true);
    expect(() => validateAgtb(wrongRate)).toThrow('tick rate');
  });

  test('decodes perceptual priority buckets and three bulk tiers', () => {
    const profile = aggregateAgtb(batch([
      { timestamp: 1, correlation: 9, descriptor: 22, phase: 1, argument0: 12 },
      { timestamp: 2, correlation: 10, descriptor: 14, phase: 4, argument1: 2 },
    ]));
    expect(profile.perceptualPriorityBuckets[2]).toBe(1);
    expect(profile.bulkWaitTierStarts).toEqual([0, 0, 1]);
  });

  test('pairs correlated spans and reports status and unmatched starts', () => {
    const complete = batch([
      { timestamp: 100, correlation: 7, descriptor: 13, phase: 4 },
      { timestamp: 250, correlation: 7, descriptor: 13, phase: 5, argument0: 18_496, argument1: 0 },
      { timestamp: 300, correlation: 8, descriptor: 13, phase: 4 },
    ]);
    const profile = aggregateAgtb(complete);
    expect(profile.unmatchedStarts).toBe(1);
    expect(profile.stages).toContainEqual({
      name: 'vt.page_load',
      records: 3,
      operations: 1,
      totalMs: 0.00015,
      meanMs: 0.00015,
      p50Ms: 0.00015,
      p95Ms: 0.00015,
      p99Ms: 0.00015,
      maxMs: 0.00015,
      argument0Total: 18_496,
      statuses: { '0': 1 },
    });
  });
});
