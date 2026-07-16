// Pure virtual-texture layout helpers. No Three.js/DOM dependency.

export interface PackedPageTableLayout {
  width: number;
  baseHeight: number;
  storageWidth: number;
  height: number;
  maxMip: number;
  mipOffsets: Uint32Array;
}

export function assertVirtualTextureDimensions(width: number, height: number): void {
  for (const [name, value] of [['width', width], ['height', height]] as const)
    if (!Number.isSafeInteger(value) || value <= 0)
      throw new RangeError(`virtual texture ${name} must be a positive safe integer`);
}

export function createPackedPageTableLayout(pageGridX: number, pageGridY = pageGridX): PackedPageTableLayout {
  for (const value of [pageGridX, pageGridY])
    if (!Number.isSafeInteger(value) || value <= 0) throw new RangeError('page grids must be positive integers');
  const maxMip = Math.ceil(Math.log2(Math.max(pageGridX, pageGridY)));
  const mipOffsets = new Uint32Array(maxMip + 1);
  let height = 0;
  for (let mip = 0; mip <= maxMip; mip++) {
    mipOffsets[mip] = height;
    height += pagesAtMipAxis(pageGridY, mip);
  }
  return { width: pageGridX, baseHeight: pageGridY, storageWidth: Math.max(2, pageGridX), height, maxMip, mipOffsets };
}

export function pagesAtMipAxis(base: number, mip: number): number {
  return Math.max(1, Math.ceil(base / 2 ** mip));
}

export function pageGridAtMip(layout: PackedPageTableLayout, mip: number): { width: number; height: number } {
  if (!Number.isInteger(mip) || mip < 0 || mip > layout.maxMip)
    throw new RangeError(`mip ${mip} is outside 0..${layout.maxMip}`);
  return { width: pagesAtMipAxis(layout.width, mip), height: pagesAtMipAxis(layout.baseHeight, mip) };
}

export function packedPageTableIndex(layout: PackedPageTableLayout, mip: number, x: number, y: number): number {
  const grid = pageGridAtMip(layout, mip);
  if (!Number.isInteger(x) || !Number.isInteger(y) || x < 0 || y < 0 || x >= grid.width || y >= grid.height)
    throw new RangeError(`page (${x},${y}) is outside mip ${mip}'s ${grid.width}x${grid.height} grid`);
  return (layout.mipOffsets[mip] + y) * layout.storageWidth + x;
}

export function packedMipTailIndex(layout: PackedPageTableLayout): number {
  return layout.mipOffsets[layout.maxMip] * layout.storageWidth + 1;
}
