import * as THREE from 'three';

export const PAGE_SIZE = 128;
export const PAGE_BORDER = 4;
export const SLOT_SIZE = PAGE_SIZE + PAGE_BORDER * 2;
export const ATLAS_PAGES_X = 15;
export const ATLAS_PAGES_Y = 15;
export const ATLAS_WIDTH = ATLAS_PAGES_X * SLOT_SIZE;
export const ATLAS_HEIGHT = ATLAS_PAGES_Y * SLOT_SIZE;

export const BLOCK_SIZE = 4;
const COMPRESSED_BYTES_PER_BLOCK = 16;
export const SLOT_BLOCKS_X = SLOT_SIZE / BLOCK_SIZE;
export const SLOT_BLOCKS_Y = SLOT_SIZE / BLOCK_SIZE;

export const FORMAT_BC7 = 0;
export const FORMAT_ASTC = 1;
export const FORMAT_RGBA = 4;
export const FORMAT_R8 = 5;
export const FORMAT_R16F = 6;

export async function detectBestTextureFormat(adapter?: GPUAdapter | null): Promise<number> {
  if (adapter) {
    if (adapter.features.has('texture-compression-bc')) return FORMAT_BC7;
    if (adapter.features.has('texture-compression-astc')) return FORMAT_ASTC;
  }
  return FORMAT_RGBA;
}

export function isCompressedTextureFormat(format: number): boolean {
  return format === FORMAT_BC7 || format === FORMAT_ASTC;
}

export function uncompressedBytesPerTexel(format: number): number {
  if (format === FORMAT_RGBA) return 4;
  if (format === FORMAT_R8) return 1;
  if (format === FORMAT_R16F) return 2;
  throw new RangeError(`texture format ${format} is compressed or unsupported`);
}

export function bytesPerBlock(format: number): number {
  if (isCompressedTextureFormat(format)) return COMPRESSED_BYTES_PER_BLOCK;
  return BLOCK_SIZE * BLOCK_SIZE * uncompressedBytesPerTexel(format);
}

export function threeFormat(format: number): THREE.CompressedPixelFormat {
  if (format === FORMAT_BC7) return THREE.RGBA_BPTC_Format;
  if (format === FORMAT_ASTC) return THREE.RGBA_ASTC_4x4_Format;
  throw new RangeError(`unsupported compressed texture format ${format}`);
}

export function packPageTableEntry(resident: boolean, physicalX: number, physicalY: number): number {
  return (resident ? 1 : 0) |
    ((physicalX & 0xff) << 1) |
    ((physicalY & 0xff) << 9);
}

export function isPageTableEntryResident(entry: number): boolean {
  return (entry & 1) !== 0;
}
