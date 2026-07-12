// .big container format parser (JavaScript side).
//
// Binary layout:
//   MAGIC (4 bytes: "BIG1")
//   VERSION (4 bytes: u32 LE = 2)
//   DATA_OFFSET (8 bytes: u64 LE)
//   HEADER (postcard-encoded BigHeader, from offset 16 to data_offset)
//   CHUNK DATA (raw bytes, from data_offset to end)
//
// Postcard encoding uses varint (LEB128) for u32/u64/i32/i64,
// and length-prefixed bytes for strings and vectors.
//
// BigHeader { version: u32, data_offset: u64, assets: Vec<AssetEntry> }
// AssetEntry { name: String, asset_type: AssetType (enum), chunks: Vec<ChunkInfo> }
// ChunkInfo { offset: u64, compressed_size: u64, uncompressed_size: u64,
//             lod_level: u8, mip_level: u8, compression: Compression (enum),
//             meta: ChunkMeta (enum) }

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
  const [val, newOff] = decodeVarint(bytes, off);
  return [BigInt(val), newOff];
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

export interface ChunkMeta {
  type: 'Texture' | 'Mesh' | 'VirtualTexturePage' | 'VirtualTextureMipTail' | 'Raw';
  width?: number;
  height?: number;
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

export interface AssetEntry {
  name: string;
  assetType: AssetType;
  chunks: ChunkInfo[];
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

function decodeChunkMeta(bytes: Uint8Array, off: number): [ChunkMeta, number] {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: { // Texture { width, height }
      const [w, o2] = decodeU32(bytes, o);
      const [h, o3] = decodeU32(bytes, o2);
      return [{ type: 'Texture', width: w, height: h }, o3];
    }
    case 1: { // Mesh { index_count, vertex_count, position_stride, uv_stride }
      const [ic, o2] = decodeU32(bytes, o);
      const [vc, o3] = decodeU32(bytes, o2);
      const [ps, o4] = decodeU32(bytes, o3);
      const [us, o5] = decodeU32(bytes, o4);
      return [{ type: 'Mesh', indexCount: ic, vertexCount: vc, positionStride: ps, uvStride: us }, o5];
    }
    case 2: { // VirtualTexturePage { mip, page_x, page_y, encoding }
      const [mip, o2] = decodeU8(bytes, o);
      const [px, o3] = decodeU32(bytes, o2);
      const [py, o4] = decodeU32(bytes, o3);
      const [encoding, o5] = decodeTextureEncoding(bytes, o4);
      return [{ type: 'VirtualTexturePage', mip, pageX: px, pageY: py, encoding }, o5];
    }
    case 3: { // Raw
      return [{ type: 'Raw' }, o];
    }
    case 4: { // VirtualTextureMipTail { first_mip, encoding }
      const [firstMip, o2] = decodeU8(bytes, o);
      const [encoding, o3] = decodeTextureEncoding(bytes, o2);
      return [{ type: 'VirtualTextureMipTail', mip: firstMip, encoding }, o3];
    }
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

function decodeAssetEntry(bytes: Uint8Array, off: number): [AssetEntry, number] {
  const [name, o1] = decodeString(bytes, off);
  const [assetType, o2] = decodeAssetType(bytes, o1);
  const [chunks, o3] = decodeVec(bytes, o2, decodeChunkInfo);
  return [{ name, assetType, chunks }, o3];
}

// ============================================================================
// Public API
// ============================================================================

export const BIG_MAGIC = 0x31474942; // "BIG1" as u32 LE
export const BIG_VERSION = 3;

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
  if (version !== BIG_VERSION) throw new Error(`.big: version ${version} != ${BIG_VERSION}`);

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
export function findVTMipTailChunk(header: BigHeader, assetName: string): ChunkInfo | null {
  const asset = header.assets.find(a => a.name === assetName);
  return asset?.chunks.find(c => c.meta.type === 'VirtualTextureMipTail') ?? null;
}

export function findVTPageChunk(
  header: BigHeader,
  assetName: string,
  mip: number,
  pageX: number,
  pageY: number,
): ChunkInfo | null {
  const asset = header.assets.find(a => a.name === assetName);
  if (!asset) return null;

  return asset.chunks.find(c =>
    c.meta.type === 'VirtualTexturePage' &&
    c.meta.mip === mip &&
    c.meta.pageX === pageX &&
    c.meta.pageY === pageY,
  ) ?? null;
}

/**
 * Create a page data provider that reads pages from a .big file
 * and transcodes them via the texture worker.
 *
 * @param loader The asset loader (provides read(offset, len))
 * @param header The parsed .big header
 * @param textureWorker The texture worker client (provides transcode())
 * @param format The target GPU format (FORMAT_BC7, FORMAT_ASTC, FORMAT_RGBA)
 * @returns A function (path, req) → Promise<Uint8Array> that returns transcoded page data
 */
export function createPageDataProvider(
  loader: { read(path: string, offset: number, len: number): Promise<Uint8Array>; load(path: string): Promise<Uint8Array>; size(path: string): Promise<number> },
  header: BigHeader,
  textureWorker: { transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>; poll(): void },
  format: number,
): (path: string, req: { mip: number; x: number; y: number; tail?: boolean }) => Promise<Uint8Array> {
  // Cache the .big file data (loaded once)
  let bigFileData: Uint8Array | null = null;

  return async (path: string, req: { mip: number; x: number; y: number; tail?: boolean }) => {
    const chunk = req.tail
      ? findVTMipTailChunk(header, path)
      : findVTPageChunk(header, path, req.mip, req.x, req.y);
    if (!chunk) {
      throw new Error(`VT page not found: ${path} mip=${req.mip} (${req.x},${req.y})`);
    }

    // Read the page data from the .big file
    const pageData = await loader.read(
      path + '.big',  // or however the .big file path is resolved
      Number(chunk.offset),
      Number(chunk.compressedSize),
    );

    if (chunk.meta.encoding === 'RawRgba8') {
      if (format !== 4) {
        throw new Error(`VT page ${path} is raw RGBA8 but GPU format ${format} requires Basis encoding`);
      }
      return pageData;
    }

    // The worker returns [count][width][height][length][data]...; a VT page
    // consumes only the first image payload, never the serialization header.
    const transcoded = await textureWorker.transcode(pageData, format);
    if (transcoded.byteLength < 16) throw new Error('truncated transcoded VT page');
    const view = new DataView(transcoded.buffer, transcoded.byteOffset, transcoded.byteLength);
    const count = view.getUint32(0, true);
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const length = view.getUint32(12, true);
    if (count < 1 || width !== 136 || height !== 136 || 16 + length > transcoded.byteLength)
      throw new Error(`invalid transcoded VT page header: count=${count}, size=${width}x${height}, bytes=${length}`);
    return transcoded.slice(16, 16 + length);
  };
}
