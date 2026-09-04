import { describe, expect, test } from 'bun:test';
import {
  PAINT_TILE_WRITE_BATCH_BYTES,
  PaintTileStore,
  paintEvictionRankBefore,
  paintTileColumnBounds,
  paintTileKey,
  paintTileWriteBatchBytes,
  protectedTileRegionContains,
} from './paint-tile-store.ts';

describe('paintTileKey', () => {
  test('keeps document, layer, and tile coordinates in key order', () => {
    expect(paintTileKey('doc-1', 3, -2, 7)).toEqual(['doc-1', 3, -2, 7]);
  });
});

describe('paint tile batches', () => {
  test('uses one exact compound-key column and a 2 MiB write batch', async () => {
    expect(paintTileColumnBounds('doc', 2, 7, 3, 9)).toEqual([
      ['doc', 2, 7, 3],
      ['doc', 2, 7, 9],
    ]);
    const data = new ArrayBuffer(32 * 1024);
    const records = Array.from({ length: 64 }, (_, tx) => ({
      key: paintTileKey('doc', 0, tx, 0),
      data,
    }));
    expect(paintTileWriteBatchBytes(records)).toBe(PAINT_TILE_WRITE_BATCH_BYTES);
    const over = [...records, { key: paintTileKey('doc', 0, 64, 0), data }];
    await expect(new PaintTileStore().putMany(over)).rejects.toThrow('exceeds 2 MiB');
  });

  test('selects far-behind tiles before near or forward tiles', () => {
    const farBehind = [0, 0, 1, 1, 1, -100, 10_000] as const;
    const nearBehind = [0, 1, 2, 1, 1, -10, 100] as const;
    const farForward = [0, 2, 9, 1, 0, 200, 40_000] as const;
    expect(paintEvictionRankBefore(farBehind, nearBehind)).toBe(true);
    expect(paintEvictionRankBefore(nearBehind, farForward)).toBe(true);
  });
});

describe('protectedTileRegionContains', () => {
  test('moves the usable edge inward but keeps document edges', () => {
    const middle = [10, 10, 42, 42] as const;
    expect(protectedTileRegionContains(middle, 17, 18, 250, 250, 8)).toBe(false);
    expect(protectedTileRegionContains(middle, 18, 18, 250, 250, 8)).toBe(true);
    expect(protectedTileRegionContains(middle, 34, 34, 250, 250, 8)).toBe(true);
    expect(protectedTileRegionContains(middle, 35, 34, 250, 250, 8)).toBe(false);

    expect(protectedTileRegionContains([0, 0, 32, 32], 0, 0, 250, 250, 8)).toBe(true);
    expect(protectedTileRegionContains([217, 217, 249, 249], 249, 249, 250, 250, 8)).toBe(true);
  });
});
