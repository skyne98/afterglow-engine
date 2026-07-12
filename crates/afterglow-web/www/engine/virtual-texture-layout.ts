// Pure virtual-texture layout helpers. This module intentionally has no Three.js
// or DOM dependency so the GPU memory layout can be regression-tested directly.

export interface PackedPageTableLayout {
  /** Logical mip-zero page width. */
  width: number;
  /** Physical texture row stride; at least two for the mip-tail entry. */
  storageWidth: number;
  height: number;
  maxMip: number;
  mipOffsets: Uint32Array;
}

export function assertVirtualTextureSize(virtualSize: number, pageSize: number): void {
  if (!Number.isSafeInteger(virtualSize) || virtualSize <= 0)
    throw new RangeError('virtualSize must be a positive safe integer');
  if (!Number.isSafeInteger(pageSize) || pageSize <= 0)
    throw new RangeError('pageSize must be a positive safe integer');
  if (virtualSize % pageSize !== 0)
    throw new RangeError(`virtualSize ${virtualSize} must be divisible by pageSize ${pageSize}`);
  const grid = virtualSize / pageSize;
  if ((grid & (grid - 1)) !== 0)
    throw new RangeError(`page grid ${grid} must be a power of two`);
}

export function createPackedPageTableLayout(pageGrid: number): PackedPageTableLayout {
  if (!Number.isSafeInteger(pageGrid) || pageGrid <= 0 || (pageGrid & (pageGrid - 1)) !== 0)
    throw new RangeError('pageGrid must be a positive power of two');
  const maxMip = Math.log2(pageGrid);
  const mipOffsets = new Uint32Array(maxMip + 1);
  let height = 0;
  for (let mip = 0; mip <= maxMip; mip++) {
    mipOffsets[mip] = height;
    height += Math.max(1, pageGrid >> mip);
  }
  return { width: pageGrid, storageWidth: Math.max(2, pageGrid), height, maxMip, mipOffsets };
}

export function pagesAtMip(layout: PackedPageTableLayout, mip: number): number {
  if (!Number.isInteger(mip) || mip < 0 || mip > layout.maxMip)
    throw new RangeError(`mip ${mip} is outside 0..${layout.maxMip}`);
  return Math.max(1, layout.width >> mip);
}

export function packedPageTableIndex(
  layout: PackedPageTableLayout,
  mip: number,
  x: number,
  y: number,
): number {
  const pages = pagesAtMip(layout, mip);
  if (!Number.isInteger(x) || !Number.isInteger(y) || x < 0 || y < 0 || x >= pages || y >= pages)
    throw new RangeError(`page (${x},${y}) is outside mip ${mip}'s ${pages}x${pages} grid`);
  return (layout.mipOffsets[mip] + y) * layout.storageWidth + x;
}

export function packedMipTailIndex(layout: PackedPageTableLayout): number {
  return layout.mipOffsets[layout.maxMip] * layout.storageWidth + 1;
}
