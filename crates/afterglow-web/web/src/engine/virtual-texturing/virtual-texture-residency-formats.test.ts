import { describe, expect, test } from 'bun:test';
import { FORMAT_R16F, FORMAT_R8, SLOT_SIZE } from './virtual-texture-format.ts';
import { PageCache } from './virtual-texture-residency.ts';

const page = { path: 'runtime', textureId: 1, mip: 0, x: 0, y: 0, pinned: false, cacheKey: 7 };

describe('uncompressed VT atlas pools', () => {
  test('tracks exact R8 and R16F page byte layouts', () => {
    const r8 = new PageCache(FORMAT_R8, SLOT_SIZE * 2);
    const r16 = new PageCache(FORMAT_R16F, SLOT_SIZE * 2);
    expect(r8.slotDataSize).toBe(SLOT_SIZE * SLOT_SIZE);
    expect(r16.slotDataSize).toBe(SLOT_SIZE * SLOT_SIZE * 2);
    const acquired = r8.acquire(page);
    r8.commit(page, acquired.slot, new Uint8Array(r8.slotDataSize).fill(3));
    expect(r8.replaceByKey(7, new Uint8Array(r8.slotDataSize).fill(9))).toBe(acquired.slot);
    expect(() => r8.replaceByKey(7, new Uint8Array(4))).toThrow('expected');
  });
});
