import { describe, expect, test } from 'bun:test';
import {
  PAINT_GROW_TILES,
  paintMemoryLimitMiB,
  paintMemoryPolicy,
  paintTileWorkerCount,
} from './paint-memory.ts';

describe('paintMemoryLimitMiB', () => {
  test('uses 25 percent with fallback, blocks, ceiling, and override', () => {
    expect(paintMemoryLimitMiB(8)).toBe(2048);
    expect(paintMemoryLimitMiB(4)).toBe(1024);
    expect(paintMemoryLimitMiB(0.25)).toBe(64);
    expect(paintMemoryLimitMiB()).toBe(1024);
    expect(paintMemoryLimitMiB(8, 1537)).toBe(1536);
    expect(paintMemoryLimitMiB(8, 3000)).toBe(2048);
  });
});

describe('paintMemoryPolicy', () => {
  test('subtracts history, worker, and safety memory', () => {
    expect(paintTileWorkerCount(32)).toBe(15);
    expect(paintTileWorkerCount(8)).toBe(3);
    expect(paintMemoryPolicy(256, 7).maximumTiles).toBe(0);
    expect(paintMemoryPolicy(2048, 15)).toEqual({
      totalMiB: 2048,
      initialTiles: 4096,
      maximumTiles: 55_552,
      growTiles: PAINT_GROW_TILES,
    });
  });
});
