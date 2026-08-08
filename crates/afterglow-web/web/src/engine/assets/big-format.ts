// .big container format parser (JavaScript side).
//
// Binary layout:
//   MAGIC (4 bytes: "BIG1")
//   VERSION (4 bytes: u32 LE; current writer 6, readable 5..6)
//   DATA_OFFSET (8 bytes: u64 LE)
//   HEADER (postcard-encoded BigHeader, from offset 16 to data_offset)
//   CHUNK DATA (raw bytes, from data_offset to end)
//
// Postcard encoding uses varint (LEB128) for u32/u64/i32/i64,
// and length-prefixed bytes for strings and vectors.
//
// BigHeader { version: u32, data_offset: u64, assets: Vec<AssetEntry> }
// AssetEntry { name, asset_type, chunks, virtual_texture: Option<VirtualTextureDirectory> }
// Virtual textures use one offset + a compact size vector per row-major mip,
// rather than serializing a full ChunkInfo for every page.

// ============================================================================
// Varint decoder (postcard / LEB128)
// ============================================================================

function decodeVarint(bytes: Uint8Array, off: number): [number, number] {
  let r = 0;
  for (let shift = 0; shift < 56; shift += 7) {
    if (off >= bytes.length) throw new Error('postcard varint truncated');
    const b = bytes[off++];
    r += (b & 127) * 2 ** shift;
    if (!(b & 128)) return [r, off];
  }
  throw new Error('postcard varint overflows');
}

function decodeU32(bytes: Uint8Array, off: number): [number, number] {
  return decodeVarint(bytes, off);
}

function decodeU64(bytes: Uint8Array, off: number): [bigint, number] {
  let result = 0n;
  for (let shift = 0n; shift < 70n; shift += 7n) {
    if (off >= bytes.length) throw new Error('postcard u64 varint truncated');
    const byte = bytes[off++];
    result |= BigInt(byte & 127) << shift;
    if (!(byte & 128)) {
      if (result > 0xffff_ffff_ffff_ffffn) throw new Error('postcard u64 varint overflows');
      return [result, off];
    }
  }
  throw new Error('postcard u64 varint overflows');
}

function decodeString(bytes: Uint8Array, off: number): [string, number] {
  const [len, o] = decodeVarint(bytes, off);
  const str = new TextDecoder().decode(bytes.subarray(o, o + len));
  return [str, o + len];
}

function decodeVec<T>(
  bytes: Uint8Array, off: number,
  decodeFn: (bytes: Uint8Array, off: number) => [T, number],
): [T[], number] {
  const [len, o] = decodeVarint(bytes, off);
  const result: T[] = [];
  let pos = o;
  for (let i = 0; i < len; i++) {
    const [item, newOff] = decodeFn(bytes, pos);
    result.push(item);
    pos = newOff;
  }
  return [result, pos];
}

function decodeBool(bytes: Uint8Array, off: number): [boolean, number] {
  return [bytes[off] !== 0, off + 1];
}

function decodeU8(bytes: Uint8Array, off: number): [number, number] {
  return [bytes[off], off + 1];
}

// ============================================================================
// .big format types (mirror Rust structs)
// ============================================================================

export type AssetType = 'Texture' | 'Mesh' | 'VirtualTexture';
export type Compression = 'Meshopt' | 'None';
export type TextureEncoding = 'RawRgba8' | 'Basis';
/** Texel format of a resident (non-virtual) `Texture` asset. */
export type TextureFormat = 'Rgba8' | 'R8';

export interface ChunkMeta {
  type: 'Texture' | 'Mesh' | 'VirtualTexturePage' | 'VirtualTextureMipTail' | 'Raw';
  width?: number;
  height?: number;
  format?: TextureFormat;
  indexCount?: number;
  vertexCount?: number;
  positionStride?: number;
  uvStride?: number;
  mip?: number;
  pageX?: number;
  pageY?: number;
  encoding?: TextureEncoding;
}

export interface ChunkInfo {
  offset: bigint;
  compressedSize: bigint;
  uncompressedSize: bigint;
  lodLevel: number;
  mipLevel: number;
  compression: Compression;
  meta: ChunkMeta;
}

export interface VirtualTextureMipDirectory {
  mip: number;
  pagesX: number;
  pagesY: number;
  offset: bigint;
  pageSizes: number[];
}

export interface VirtualTextureTailDirectory {
  firstMip: number;
  offset: bigint;
  size: number;
}

export interface VirtualTextureDirectory {
  width: number;
  height: number;
  encoding: TextureEncoding;
  mips: VirtualTextureMipDirectory[];
  tail: VirtualTextureTailDirectory | null;
}

export interface AssetEntry {
  name: string;
  assetType: AssetType;
  chunks: ChunkInfo[];
  virtualTexture: VirtualTextureDirectory | null;
}

export interface BigHeader {
  version: number;
  dataOffset: bigint;
  assets: AssetEntry[];
}

// ============================================================================
// Postcard decoders for .big structs
// ============================================================================

function decodeAssetType(bytes: Uint8Array, off: number): [AssetType, number] {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: return ['Texture', o];
    case 1: return ['Mesh', o];
    case 2: return ['VirtualTexture', o];
    default: throw new Error(`unknown AssetType variant: ${variant}`);
  }
}

function decodeCompression(bytes: Uint8Array, off: number): [Compression, number] {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: return ['Meshopt', o];
    case 1: return ['None', o];
    default: throw new Error(`unknown Compression variant: ${variant}`);
  }
}

function decodeTextureEncoding(bytes: Uint8Array, off: number): [TextureEncoding, number] {
  const [variant, next] = decodeU32(bytes, off);
  if (variant === 0) return ['RawRgba8', next];
  if (variant === 1) return ['Basis', next];
  throw new Error(`unknown TextureEncoding variant: ${variant}`);
}

function decodeTextureFormat(bytes: Uint8Array, off: number): [TextureFormat, number] {
  const [variant, next] = decodeU32(bytes, off);
  if (variant === 0) return ['Rgba8', next];
  if (variant === 1) return ['R8', next];
  throw new Error(`unknown TextureFormat variant: ${variant}`);
}

function decodeChunkMeta(bytes: Uint8Array, off: number): [ChunkMeta, number] {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: { // Texture { width, height, format }
      const [w, o2] = decodeU32(bytes, o);
      const [h, o3] = decodeU32(bytes, o2);
      const [format, o4] = decodeTextureFormat(bytes, o3);
      return [{ type: 'Texture', width: w, height: h, format }, o4];
    }
    case 1: { // Mesh { index_count, vertex_count, position_stride, uv_stride }
      const [ic, o2] = decodeU32(bytes, o);
      const [vc, o3] = decodeU32(bytes, o2);
      const [ps, o4] = decodeU32(bytes, o3);
      const [us, o5] = decodeU32(bytes, o4);
      return [{ type: 'Mesh', indexCount: ic, vertexCount: vc, positionStride: ps, uvStride: us }, o5];
    }
    case 2: return [{ type: 'Raw' }, o];
    default: throw new Error(`unknown ChunkMeta variant: ${variant}`);
  }
}

function decodeChunkInfo(bytes: Uint8Array, off: number): [ChunkInfo, number] {
  const [offset, o1] = decodeU64(bytes, off);
  const [compressedSize, o2] = decodeU64(bytes, o1);
  const [uncompressedSize, o3] = decodeU64(bytes, o2);
  const [lodLevel, o4] = decodeU8(bytes, o3);
  const [mipLevel, o5] = decodeU8(bytes, o4);
  const [compression, o6] = decodeCompression(bytes, o5);
  const [meta, o7] = decodeChunkMeta(bytes, o6);
  return [{
    offset, compressedSize, uncompressedSize,
    lodLevel, mipLevel, compression, meta,
  }, o7];
}

function decodeVTMipDirectory(bytes: Uint8Array, off: number): [VirtualTextureMipDirectory, number] {
  const [mip, o1] = decodeU8(bytes, off);
  const [pagesX, o2] = decodeU32(bytes, o1);
  const [pagesY, o3] = decodeU32(bytes, o2);
  const [offset, o4] = decodeU64(bytes, o3);
  const [pageSizes, o5] = decodeVec(bytes, o4, decodeU32);
  return [{ mip, pagesX, pagesY, offset, pageSizes }, o5];
}

function decodeVTTailDirectory(bytes: Uint8Array, off: number): [VirtualTextureTailDirectory, number] {
  const [firstMip, o1] = decodeU8(bytes, off);
  const [offset, o2] = decodeU64(bytes, o1);
  const [size, o3] = decodeU32(bytes, o2);
  return [{ firstMip, offset, size }, o3];
}

function decodeVTDirectory(bytes: Uint8Array, off: number): [VirtualTextureDirectory, number] {
  const [width, o1] = decodeU32(bytes, off);
  const [height, o2] = decodeU32(bytes, o1);
  const [encoding, o3] = decodeTextureEncoding(bytes, o2);
  const [mips, o4] = decodeVec(bytes, o3, decodeVTMipDirectory);
  const [hasTail, o5] = decodeBool(bytes, o4);
  if (!hasTail) return [{ width, height, encoding, mips, tail: null }, o5];
  const [tail, o6] = decodeVTTailDirectory(bytes, o5);
  return [{ width, height, encoding, mips, tail }, o6];
}

function decodeAssetEntry(bytes: Uint8Array, off: number): [AssetEntry, number] {
  const [name, o1] = decodeString(bytes, off);
  const [assetType, o2] = decodeAssetType(bytes, o1);
  const [chunks, o3] = decodeVec(bytes, o2, decodeChunkInfo);
  const [hasVirtualTexture, o4] = decodeBool(bytes, o3);
  if (!hasVirtualTexture) return [{ name, assetType, chunks, virtualTexture: null }, o4];
  const [virtualTexture, o5] = decodeVTDirectory(bytes, o4);
  return [{ name, assetType, chunks, virtualTexture }, o5];
}

// ============================================================================
// Public API
// ============================================================================

export const BIG_MAGIC = 0x31474942; // "BIG1" as u32 LE
/** Current `.big` version written by the pipeline. */
export const BIG_VERSION = 6;
/** Oldest readable `.big` version. v5 files predate resident `Texture` assets
 *  (they contain no `AssetType::Texture` chunks, so the `ChunkMeta::Texture`
 *  encoding carrying a `format` field is unambiguous). */
export const BIG_MIN_READABLE_VERSION = 5;

/**
 * Parse a .big file header from raw bytes.
 *
 * Layout: MAGIC(4) + VERSION(4) + DATA_OFFSET(8) + POSTCARD_HEADER + DATA
 *
 * @param data The raw .big file bytes (at least the header portion)
 * @returns { header, dataOffset } where dataOffset is where chunk data starts
 */
export function parseBigHeader(data: Uint8Array): { header: BigHeader; dataOffset: number } {
  if (data.length < 16) throw new Error('.big: file too small');

  // MAGIC (4 bytes, LE u32)
  const magic = new DataView(data.buffer, data.byteOffset, 4).getUint32(0, true);
  if (magic !== BIG_MAGIC) throw new Error('.big: bad magic');

  // VERSION (4 bytes, LE u32)
  const version = new DataView(data.buffer, data.byteOffset + 4, 4).getUint32(0, true);
  if (version < BIG_MIN_READABLE_VERSION || version > BIG_VERSION) {
    throw new Error(`.big: version ${version} not in [${BIG_MIN_READABLE_VERSION},${BIG_VERSION}]`);
  }

  // DATA_OFFSET (8 bytes, LE u64)
  const dataOffset = Number(new DataView(data.buffer, data.byteOffset + 8, 8).getBigUint64(0, true));

  // Postcard-encoded BigHeader (from offset 16 to dataOffset)
  const headerBytes = data.subarray(16, dataOffset);

  // Decode postcard: BigHeader { version: u32, data_offset: u64, assets: Vec<AssetEntry> }
  let off = 0;
  const [hdrVersion, o1] = decodeU32(headerBytes, off); off = o1;
  const [hdrDataOffset, o2] = decodeU64(headerBytes, off); off = o2;
  const [assets, o3] = decodeVec(headerBytes, off, decodeAssetEntry); off = o3;

  return {
    header: { version: hdrVersion, dataOffset: hdrDataOffset, assets },
    dataOffset,
  };
}

/**
 * Find a chunk for a virtual texture page by mip + page coordinates.
 *
 * @param header The parsed .big header
 * @param assetName The virtual texture asset name
 * @param mip The mip level
 * @param pageX Page X coordinate
 * @param pageY Page Y coordinate
 * @returns The ChunkInfo for the matching page, or null
 */
export function getVirtualTextureDimensions(header: BigHeader, assetName: string): { width: number; height: number } {
  const directory = header.assets.find(asset => asset.name === assetName)?.virtualTexture;
  if (!directory) throw new Error(`VT dimensions unavailable: ${assetName}`);
  return { width: directory.width, height: directory.height };
}

export function findVTMipTailChunk(header: BigHeader, assetName: string): ChunkInfo | null {
  const directory = header.assets.find(asset => asset.name === assetName)?.virtualTexture;
  const tail = directory?.tail;
  if (!directory || !tail) return null;
  return {
    offset: tail.offset, compressedSize: BigInt(tail.size), uncompressedSize: BigInt(tail.size),
    lodLevel: 0, mipLevel: tail.firstMip, compression: 'None',
    meta: { type: 'VirtualTextureMipTail', mip: tail.firstMip, width: directory.width,
      height: directory.height, encoding: directory.encoding },
  };
}

export function findVTPageChunk(
  header: BigHeader,
  assetName: string,
  mip: number,
  pageX: number,
  pageY: number,
): ChunkInfo | null {
  const directory = header.assets.find(asset => asset.name === assetName)?.virtualTexture;
  const mipDirectory = directory?.mips.find(candidate => candidate.mip === mip);
  if (!directory || !mipDirectory || pageX < 0 || pageY < 0 ||
      pageX >= mipDirectory.pagesX || pageY >= mipDirectory.pagesY) return null;
  const page = pageY * mipDirectory.pagesX + pageX;
  let offset = mipDirectory.offset;
  for (let index = 0; index < page; index++) offset += BigInt(mipDirectory.pageSizes[index]);
  const size = mipDirectory.pageSizes[page];
  return {
    offset, compressedSize: BigInt(size), uncompressedSize: BigInt(size),
    lodLevel: 0, mipLevel: mip, compression: 'None',
    meta: { type: 'VirtualTexturePage', mip, pageX, pageY, encoding: directory.encoding },
  };
}

