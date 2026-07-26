import { describe, expect, test } from 'bun:test';
import {
  countSourceRuns,
  decodePageIdentity,
  replayBatches,
  rescheduleRequests,
  type ReplayRequest,
} from './replay-dungeon-vt.ts';

function request(overrides: Partial<ReplayRequest> = {}): ReplayRequest {
  return {
    key: 0x2000_0000,
    textureId: 1,
    material: 0,
    channel: 0,
    mip: 0,
    x: 0,
    y: 0,
    tail: false,
    detectedAt: 0,
    admittedAt: 0,
    timestamp: 0,
    bytes: 10,
    lane: 1,
    priority: 86,
    sourceOffset: 100,
    dispatched: true,
    ...overrides,
  };
}

describe('Dungeon VT trace replay', () => {
  test('decodes the stable packed page identity', () => {
    const textureId = 7;
    const mip = 5;
    const x = 321;
    const y = 654;
    const key = textureId * 0x2000_0000 + mip + (x << 6) + (y << 17);
    expect(decodePageIdentity(key)).toEqual({
      textureId,
      material: 2,
      channel: 0,
      mip,
      x,
      y,
      tail: false,
    });
    expect(decodePageIdentity(textureId * 0x2000_0000 + 0x1000_0000).tail).toBe(true);
  });

  test('replays independent non-resetting urgent, focus, and peripheral deadlines', () => {
    const batches = replayBatches([
      request({ key: 1, timestamp: 0, lane: 1 }),
      request({ key: 2, timestamp: 5_000_000, lane: 1 }),
      request({ key: 3, timestamp: 500_000, lane: 0 }),
      request({ key: 4, timestamp: 17_000_000, lane: 1 }),
      request({ key: 5, timestamp: 0, lane: 2 }),
      request({ key: 6, timestamp: 63_000_000, lane: 2 }),
      request({ key: 7, timestamp: 65_000_000, lane: 2 }),
    ]);
    expect(batches.map(batch => ({ lane: batch.lane, keys: batch.requests.map(value => value.key) })))
      .toEqual([
        { lane: 0, keys: [3] },
        { lane: 1, keys: [1, 2] },
        { lane: 1, keys: [4] },
        { lane: 2, keys: [5, 6] },
        { lane: 2, keys: [7] },
      ]);
  });

  test('source sorting reduces adjacent runs without changing a batch', () => {
    const requests = [
      request({ key: 1, sourceOffset: 100 }),
      request({ key: 2, sourceOffset: 200 }),
      request({ key: 3, sourceOffset: 110 }),
    ];
    expect(countSourceRuns(requests, false)).toBe(3);
    expect(countSourceRuns(requests, true)).toBe(2);
    expect(requests.map(value => value.key)).toEqual([1, 2, 3]);
  });

  test('mip-deficit sensitivity reverses the current coarse-first rung', () => {
    const coarse = request({ key: 1, mip: 8, priority: 70, timestamp: 0 });
    const fine = request({ key: 2, mip: 0, priority: 86, timestamp: 1 });
    const replayed = rescheduleRequests([coarse, fine], true, false);
    expect(replayed.map(value => value.key)).toEqual([2, 1]);
  });
});
