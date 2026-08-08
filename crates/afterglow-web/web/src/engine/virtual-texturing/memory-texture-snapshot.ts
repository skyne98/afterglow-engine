import {
  MemoryTextureWriteStatus,
  MemoryVirtualTextureSource,
  type MemoryPageSourceOptions,
  type MemoryVirtualTextureAddressMode,
  type MemoryVirtualTextureFormat,
  type MemoryVirtualTextureMipFilter,
} from './memory-page-source.ts';
import { PAGE_SIZE } from './virtual-texture-format.ts';
import type {
  VirtualTextureDescriptor,
  VirtualTextureStorageFormat,
} from './virtual-texture-system.ts';

const MAGIC = 0x544d4741; // AGMT, little-endian
const VERSION = 1;
const HEADER_BYTES = 44;
const RECORD_HEADER_BYTES = 4;

const FORMAT_TO_ID: Readonly<Record<string, number>> = {
  rgba8unorm: 0,
  'rgba8unorm-srgb': 1,
  r8unorm: 2,
  r16float: 3,
};
const ID_TO_FORMAT: readonly VirtualTextureStorageFormat[] = [
  'rgba8unorm', 'rgba8unorm-srgb', 'r8unorm', 'r16float',
];
const ADDRESS_TO_ID: Readonly<Record<MemoryVirtualTextureAddressMode, number>> = {
  clamp: 0, repeat: 1, 'mirror-repeat': 2,
};
const ID_TO_ADDRESS: readonly MemoryVirtualTextureAddressMode[] = [
  'clamp', 'repeat', 'mirror-repeat',
];
const FILTER_TO_ID: Readonly<Record<MemoryVirtualTextureMipFilter, number>> = {
  'linear-color': 0, normal: 1, scalar: 2,
};
const ID_TO_FILTER: readonly MemoryVirtualTextureMipFilter[] = [
  'linear-color', 'normal', 'scalar',
];

interface CanonicalPageRecord {
  x: number;
  y: number;
  storage: Uint8Array;
  offset: number;
  length: number;
}

export interface DecodedMemoryTextureSnapshot {
  readonly descriptor: Readonly<VirtualTextureDescriptor>;
  readonly sourceOptions: Readonly<Pick<MemoryPageSourceOptions, 'mipFilter' | 'defaultTexel'>>;
  readonly revision: number;
  readonly pages: readonly DecodedMemoryTexturePage[];
}

export interface DecodedMemoryTexturePage {
  readonly x: number;
  readonly y: number;
  readonly bytes: Uint8Array;
}

function bytesPerTexel(format: VirtualTextureStorageFormat): number {
  if (format === 'r8unorm') return 1;
  if (format === 'r16float') return 2;
  if (format === 'rgba8unorm' || format === 'rgba8unorm-srgb') return 4;
  throw new Error('mutable texture snapshot cannot contain a compressed format');
}

function memoryFormat(format: VirtualTextureStorageFormat): MemoryVirtualTextureFormat {
  if (format === 'r8unorm') return 'r8unorm';
  if (format === 'r16float') return 'r16float';
  return 'rgba8unorm';
}

function crc32(bytes: Uint8Array, start: number): number {
  let crc = 0xffff_ffff;
  for (let index = start; index < bytes.byteLength; index++) {
    crc ^= bytes[index] ?? 0;
    for (let bit = 0; bit < 8; bit++)
      crc = (crc >>> 1) ^ ((crc & 1) !== 0 ? 0xedb8_8320 : 0);
  }
  return (crc ^ 0xffff_ffff) >>> 0;
}

/** Deterministic, portable sparse canonical-page snapshot. */
export function encodeMemoryTextureSnapshot(
  source: MemoryVirtualTextureSource,
  descriptor: Readonly<VirtualTextureDescriptor>,
): Uint8Array {
  const formatId = FORMAT_TO_ID[descriptor.format];
  if (formatId === undefined) throw new Error('mutable snapshot format is unsupported');
  const pageBytes = PAGE_SIZE * PAGE_SIZE * source.bytesPerTexel;
  const pages: CanonicalPageRecord[] = [];
  source.visitCanonicalPages((x, y, storage, offset, length) => {
    pages.push({ x, y, storage, offset, length });
  });
  pages.sort((a, b) => a.y - b.y || a.x - b.x);
  const totalBytes = HEADER_BYTES + pages.length * (RECORD_HEADER_BYTES + pageBytes);
  if (!Number.isSafeInteger(totalBytes)) throw new RangeError('mutable snapshot is too large');
  const output = new Uint8Array(totalBytes);
  const view = new DataView(output.buffer);
  view.setUint32(0, MAGIC, true);
  view.setUint16(4, VERSION, true);
  view.setUint8(6, formatId);
  view.setUint8(7, ADDRESS_TO_ID[source.options.addressMode]);
  view.setUint8(8, FILTER_TO_ID[source.options.mipFilter]);
  const defaultTexel = source.options.defaultTexel ?? new Uint8Array(source.bytesPerTexel);
  view.setUint8(9, defaultTexel.byteLength);
  view.setUint32(12, descriptor.width, true);
  view.setUint32(16, descriptor.height, true);
  view.setUint32(20, source.contentRevision, true);
  view.setUint32(24, pages.length, true);
  view.setUint32(28, pageBytes, true);
  view.setUint32(32, totalBytes, true);
  output.set(defaultTexel, 40);
  let cursor = HEADER_BYTES;
  for (const page of pages) {
    view.setUint16(cursor, page.x, true);
    view.setUint16(cursor + 2, page.y, true);
    cursor += RECORD_HEADER_BYTES;
    output.set(page.storage.subarray(page.offset, page.offset + page.length), cursor);
    cursor += pageBytes;
  }
  view.setUint32(36, crc32(output, 40), true);
  return output;
}

export function decodeMemoryTextureSnapshot(
  bytes: Uint8Array,
  maxPages: number,
): DecodedMemoryTextureSnapshot {
  if (!Number.isInteger(maxPages) || maxPages < 0)
    throw new RangeError('invalid mutable snapshot page capacity');
  if (bytes.byteLength < HEADER_BYTES) throw new Error('mutable snapshot is truncated');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, true) !== MAGIC) throw new Error('mutable snapshot magic is invalid');
  if (view.getUint16(4, true) !== VERSION) throw new Error('mutable snapshot version is unsupported');
  const format = ID_TO_FORMAT[view.getUint8(6)];
  const addressMode = ID_TO_ADDRESS[view.getUint8(7)];
  const mipFilter = ID_TO_FILTER[view.getUint8(8)];
  if (!format || !addressMode || !mipFilter) throw new Error('mutable snapshot descriptor is invalid');
  const defaultLength = view.getUint8(9);
  const width = view.getUint32(12, true), height = view.getUint32(16, true);
  const revision = view.getUint32(20, true), pageCount = view.getUint32(24, true);
  const pageBytes = view.getUint32(28, true), totalBytes = view.getUint32(32, true);
  if (width < 1 || height < 1 || pageCount > maxPages ||
      width > PAGE_SIZE * 2048 || height > PAGE_SIZE * 2048)
    throw new Error('mutable snapshot dimensions or page count exceed capacity');
  const texelBytes = bytesPerTexel(format);
  if (defaultLength !== texelBytes || pageBytes !== PAGE_SIZE * PAGE_SIZE * texelBytes)
    throw new Error('mutable snapshot texel layout is invalid');
  const expected = HEADER_BYTES + pageCount * (RECORD_HEADER_BYTES + pageBytes);
  if (totalBytes !== bytes.byteLength || expected !== bytes.byteLength)
    throw new Error('mutable snapshot byte length is invalid');
  if (view.getUint32(36, true) !== crc32(bytes, 40))
    throw new Error('mutable snapshot checksum is invalid');
  const defaultTexel = bytes.slice(40, 40 + defaultLength);
  const pagesX = Math.ceil(width / PAGE_SIZE), pagesY = Math.ceil(height / PAGE_SIZE);
  const seen = new Set<number>();
  const pages: DecodedMemoryTexturePage[] = new Array(pageCount);
  let cursor = HEADER_BYTES;
  for (let index = 0; index < pageCount; index++) {
    const x = view.getUint16(cursor, true), y = view.getUint16(cursor + 2, true);
    cursor += RECORD_HEADER_BYTES;
    if (x >= pagesX || y >= pagesY) throw new Error('mutable snapshot page coordinate is invalid');
    const key = y * 2048 + x;
    if (seen.has(key)) throw new Error('mutable snapshot contains duplicate pages');
    seen.add(key);
    pages[index] = { x, y, bytes: bytes.slice(cursor, cursor + pageBytes) };
    cursor += pageBytes;
  }
  return {
    descriptor: { width, height, format, addressMode },
    sourceOptions: { mipFilter, defaultTexel },
    revision,
    pages,
  };
}

export interface RestoreMemoryTextureCapacities {
  readonly pageCapacity: number;
  readonly dirtyCapacity: number;
  readonly outputCapacity: number;
}

/** Build a complete unpublished source; failure leaves no live texture mutated. */
export function restoreMemoryTextureSnapshot(
  snapshot: Readonly<DecodedMemoryTextureSnapshot>,
  capacities: Readonly<RestoreMemoryTextureCapacities>,
): MemoryVirtualTextureSource {
  const baseOptions = {
    width: snapshot.descriptor.width,
    height: snapshot.descriptor.height,
    format: memoryFormat(snapshot.descriptor.format),
    addressMode: snapshot.descriptor.addressMode,
    mipFilter: snapshot.sourceOptions.mipFilter,
    ...capacities,
  };
  const source = new MemoryVirtualTextureSource(
    snapshot.sourceOptions.defaultTexel
      ? { ...baseOptions, defaultTexel: snapshot.sourceOptions.defaultTexel }
      : baseOptions,
  );
  for (const page of snapshot.pages) {
    const x = page.x * PAGE_SIZE, y = page.y * PAGE_SIZE;
    const width = Math.min(PAGE_SIZE, snapshot.descriptor.width - x);
    const height = Math.min(PAGE_SIZE, snapshot.descriptor.height - y);
    const status = source.writeRegion(x, y, width, height, page.bytes, PAGE_SIZE * source.bytesPerTexel);
    if (status !== MemoryTextureWriteStatus.Written)
      throw new Error(`mutable snapshot exceeds restore capacities: ${status}`);
    while (source.pendingDirtyPages !== 0)
      source.drainDirty(capacities.dirtyCapacity, () => true);
  }
  source.restoreContentRevision(snapshot.revision);
  return source;
}
