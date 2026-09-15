import { test, expect } from 'bun:test';
import { compare, memoryTrend } from './check-paint-stress.ts';

function result() {
  return { width: 16384, height: 16384, radius: 60, elapsedMs: 600_001, strokes: 300,
    comparison: Array.from({ length: 8 }, () => Array(16384).fill(127)),
    frames: { count: 30000 }, state: { error: 0, scratchPeakFileBytes: 1_000_000,
      sharedMemory: { reservedBytes: 100, peakBytes: 200, limitBytes: 300 } },
    memorySamples: Array.from({ length: 60 }, (_, index) => ({ time: index * 10_000, processTreePssKiB: 200_000 })) };
}
test('exact display rows, admission, and stable process floors pass', () => {
  expect(compare(result(), result()).passed).toBe(true);
});
test('one different byte cannot pass', () => {
  const web = result(); web.comparison[7][16383]++;
  expect(compare(result(), web)).toMatchObject({ passed: false, differences: 1, maximumDifference: 1 });
});
test('incomplete runs, missing memory, growth, and exceeded admission cannot pass', () => {
  const native = result(); native.elapsedMs = 100;
  expect(() => compare(native, result())).toThrow('Incomplete');
  expect(() => memoryTrend([])).toThrow('Insufficient');
  const growing = result();
  for (let index = 0; index < 60; index++) growing.memorySamples[index].processTreePssKiB += index * 10_000;
  expect(compare(result(), growing).passed).toBe(false);
  const exceeded = result(); exceeded.state.sharedMemory.peakBytes = 301;
  expect(compare(exceeded, result()).passed).toBe(false);
});
