import { describe, expect, test } from 'bun:test';
import { assertVirtualTextureDimensions, createPackedPageTableLayout, packedMipTailIndex, packedPageTableIndex, pageGridAtMip } from './virtual-texture-layout.ts';

describe('packed virtual-texture page tables', () => {
  test('lays out square mips vertically without overlap', () => {
    const layout = createPackedPageTableLayout(32, 32);
    expect(layout.maxMip).toBe(5);
    expect([...layout.mipOffsets]).toEqual([0, 32, 48, 56, 60, 62]);
    expect(layout.height).toBe(63);
    expect(packedPageTableIndex(layout, 0, 31, 31)).toBe(31 * 32 + 31);
    expect(packedMipTailIndex(layout)).toBe(62 * 32 + 1);
  });

  test('supports rectangular and non-power-of-two page grids', () => {
    const layout = createPackedPageTableLayout(7, 3);
    expect(layout.maxMip).toBe(3);
    expect([...layout.mipOffsets]).toEqual([0, 3, 5, 6]);
    expect(pageGridAtMip(layout, 0)).toEqual({ width: 7, height: 3 });
    expect(pageGridAtMip(layout, 1)).toEqual({ width: 4, height: 2 });
    expect(pageGridAtMip(layout, 2)).toEqual({ width: 2, height: 1 });
    expect(pageGridAtMip(layout, 3)).toEqual({ width: 1, height: 1 });
    expect(packedPageTableIndex(layout, 1, 3, 1)).toBe((3 + 1) * 7 + 3);
  });

  test('rejects zero dimensions and invalid coordinates', () => {
    expect(() => assertVirtualTextureDimensions(0, 1000)).toThrow();
    expect(() => createPackedPageTableLayout(0, 3)).toThrow();
    const layout = createPackedPageTableLayout(8, 4);
    expect(() => packedPageTableIndex(layout, 1, 4, 0)).toThrow();
    expect(() => packedPageTableIndex(layout, 0, 0, 4)).toThrow();
  });
});
