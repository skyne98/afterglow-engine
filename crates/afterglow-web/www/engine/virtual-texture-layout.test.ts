import { describe, expect, test } from 'bun:test';
import {
  assertVirtualTextureSize,
  createPackedPageTableLayout,
  packedMipTailIndex,
  packedPageTableIndex,
  pagesAtMip,
} from './virtual-texture-layout.js';

describe('packed virtual-texture page tables', () => {
  test('lays out every mip vertically without overlap', () => {
    const layout = createPackedPageTableLayout(32);
    expect(layout.maxMip).toBe(5);
    expect([...layout.mipOffsets]).toEqual([0, 32, 48, 56, 60, 62]);
    expect(layout.width).toBe(32);
    expect(layout.height).toBe(63);
    expect(packedPageTableIndex(layout, 0, 31, 31)).toBe(31 * 32 + 31);
    expect(packedPageTableIndex(layout, 1, 0, 0)).toBe(32 * 32);
    expect(packedPageTableIndex(layout, 5, 0, 0)).toBe(62 * 32);
    expect(packedMipTailIndex(layout)).toBe(62 * 32 + 1);
  });

  test('supports the demo 256K texture', () => {
    const layout = createPackedPageTableLayout(2048);
    expect(layout.maxMip).toBe(11);
    expect(layout.height).toBe(4095);
    expect(layout.storageWidth).toBe(2048);
    expect(pagesAtMip(layout, 11)).toBe(1);
  });

  test('rejects invalid dimensions and coordinates', () => {
    expect(() => assertVirtualTextureSize(1000, 128)).toThrow();
    expect(() => createPackedPageTableLayout(3)).toThrow();
    const layout = createPackedPageTableLayout(8);
    expect(() => packedPageTableIndex(layout, 1, 4, 0)).toThrow();
    expect(() => packedPageTableIndex(layout, 4, 0, 0)).toThrow();
  });
});
