// .big container format parser (JavaScript side).
//
// Binary layout:
//   MAGIC (4 bytes: "BIG1")
//   VERSION (4 bytes: u32 LE = 5)
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

import type { PersistentBlobCache } from './persistent-blob-cache.ts';

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

interface TranscodeJob {
  data: Uint8Array;
  format: number;
  signal?: AbortSignal;
  queuedAt: number;
  resolve(value: Uint8Array): void;
  reject(error: unknown): void;
}

/** Fixed-capacity dispatcher over independent one-in-flight SPSC workers. */
export class BoundedTranscoderPool {
  private readonly jobs: (TranscodeJob | null)[];
  private readonly workerBusy: Uint8Array;
  private head = 0;
  private tail = 0;
  private count = 0;
  private active = 0;
  private completed = 0;
  private totalQueueMs = 0;
  private maxQueueMs = 0;
  private totalTranscodeMs = 0;
  private maxTranscodeMs = 0;
  private readonly stats = {
    workerCount: 0, active: 0, queued: 0, completed: 0,
    averageQueueMs: 0, maxQueueMs: 0,
    averageTranscodeMs: 0, maxTranscodeMs: 0,
  };

  constructor(
    private readonly workers: readonly {
      transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
    }[],
    capacity: number,
  ) {
    if (workers.length === 0 || !Number.isInteger(capacity) || capacity < 1)
      throw new RangeError('VT transcoder pool requires workers and positive capacity');
    this.jobs = new Array(capacity).fill(null);
    this.workerBusy = new Uint8Array(workers.length);
  }

  submit(data: Uint8Array, format: number, signal?: AbortSignal): Promise<Uint8Array> {
    if (this.count === this.jobs.length) return Promise.reject(new Error('VT transcode queue capacity exceeded'));
    return new Promise((resolve, reject) => {
      this.jobs[this.tail] = { data, format, signal, queuedAt: performance.now(), resolve, reject };
      this.tail = (this.tail + 1) % this.jobs.length;
      this.count++;
      this.pump();
    });
  }

  private pump(): void {
    for (let workerIndex = 0; workerIndex < this.workers.length && this.count !== 0; workerIndex++) {
      if (this.workerBusy[workerIndex] !== 0) continue;
      const job = this.jobs[this.head]!;
      this.jobs[this.head] = null;
      this.head = (this.head + 1) % this.jobs.length;
      this.count--;
      if (job.signal?.aborted) {
        job.reject(new Error('VT transcode canceled before dispatch'));
        workerIndex--;
        continue;
      }
      const queueMs = performance.now() - job.queuedAt;
      this.totalQueueMs += queueMs;
      this.maxQueueMs = Math.max(this.maxQueueMs, queueMs);
      this.workerBusy[workerIndex] = 1;
      this.active++;
      void this.run(workerIndex, job);
    }
  }

  private async run(workerIndex: number, job: TranscodeJob): Promise<void> {
    const startedAt = performance.now();
    try {
      const result = await this.workers[workerIndex].transcode(job.data, job.format);
      if (job.signal?.aborted) job.reject(new Error('VT transcode canceled after dispatch'));
      // Each worker owns one reusable response scratch. Copy before that same
      // worker receives its next call; other workers may complete independently.
      else job.resolve(result.slice());
    } catch (error) {
      job.reject(error);
    } finally {
      const elapsed = performance.now() - startedAt;
      this.completed++;
      this.totalTranscodeMs += elapsed;
      this.maxTranscodeMs = Math.max(this.maxTranscodeMs, elapsed);
      this.workerBusy[workerIndex] = 0;
      this.active--;
      this.pump();
    }
  }

  getStats() {
    const stats = this.stats;
    stats.workerCount = this.workers.length;
    stats.active = this.active;
    stats.queued = this.count;
    stats.completed = this.completed;
    stats.averageQueueMs = this.completed === 0 ? 0 : this.totalQueueMs / this.completed;
    stats.maxQueueMs = this.maxQueueMs;
    stats.averageTranscodeMs = this.completed === 0 ? 0 : this.totalTranscodeMs / this.completed;
    stats.maxTranscodeMs = this.maxTranscodeMs;
    return stats;
  }
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
export interface AssetIdentity {
  size: number;
  etag: string | null;
  lastModified: string | null;
}

export interface FetchRangeLoader {
  load(path: string): Promise<Uint8Array>;
  size(path: string): Promise<number>;
  identity(path: string): Promise<AssetIdentity>;
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
}

/** Read and validate one bounded BIG header without loading payload chunks. */
export async function readBigHeader(
  source: FetchRangeLoader,
  path: string,
  maxHeaderBytes: number,
): Promise<BigHeader> {
  if (!Number.isSafeInteger(maxHeaderBytes) || maxHeaderBytes < 16)
    throw new RangeError('BIG maxHeaderBytes must be at least 16');
  const prefix = await source.read(path, 0, 16);
  if (prefix.byteLength !== 16) throw new Error('BIG container prefix is truncated');
  const view = new DataView(prefix.buffer, prefix.byteOffset, prefix.byteLength);
  if (view.getUint32(0, true) !== BIG_MAGIC) throw new Error('BIG container has invalid magic');
  const version = view.getUint32(4, true);
  if (version < BIG_MIN_READABLE_VERSION || version > BIG_VERSION) {
    throw new Error(`BIG container version ${version} is unsupported`);
  }
  const dataOffset = Number(view.getBigUint64(8, true));
  if (!Number.isSafeInteger(dataOffset) || dataOffset < 16 || dataOffset > maxHeaderBytes)
    throw new RangeError(`BIG header size ${dataOffset} exceeds configured capacity ${maxHeaderBytes}`);
  const bytes = await source.read(path, 0, dataOffset);
  if (bytes.byteLength !== dataOffset) throw new Error('BIG container header is truncated');
  return parseBigHeader(bytes).header;
}

/** Browser/CEF serving-layer loader. Large assets use exact HTTP-style ranges. */
export function createFetchRangeLoader(baseUrl = ''): FetchRangeLoader {
  const url = (path: string) => baseUrl + path;
  const identity = async (path: string): Promise<AssetIdentity> => {
    const response = await fetch(url(path), { headers: { Range: 'bytes=0-0' } });
    if (response.status !== 206)
      throw new Error(`asset identity range expected 206, got ${response.status}: ${path}`);
    const contentRange = response.headers.get('content-range') ?? '';
    const separator = contentRange.lastIndexOf('/');
    const size = Number(separator < 0 ? '' : contentRange.slice(separator + 1));
    if (!Number.isSafeInteger(size) || size < 1)
      throw new Error(`asset identity has invalid content-range: ${path}`);
    return {
      size,
      etag: response.headers.get('etag'),
      lastModified: response.headers.get('last-modified'),
    };
  };
  return {
    async load(path: string): Promise<Uint8Array> {
      const response = await fetch(url(path));
      if (!response.ok) throw new Error(`asset fetch ${response.status}: ${path}`);
      return new Uint8Array(await response.arrayBuffer());
    },
    async size(path: string): Promise<number> { return (await identity(path)).size; },
    identity,
    async read(path: string, offset: number, len: number): Promise<Uint8Array> {
      if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(len) || len < 0)
        throw new RangeError('asset range must use non-negative safe integers');
      if (len === 0) return new Uint8Array(0);
      const response = await fetch(url(path), {
        headers: { Range: `bytes=${offset}-${offset + len - 1}` },
      });
      if (response.status !== 206)
        throw new Error(`asset range fetch expected 206, got ${response.status}: ${path}`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (bytes.byteLength !== len)
        throw new Error(`asset range returned ${bytes.byteLength} bytes; expected ${len}: ${path}`);
      return bytes;
    },
  };
}

/**
 * Asset-loader view over raw, uncompressed payloads packed in a `.big` file.
 * The path is the container asset name, not a deployment URL. Indexing occurs
 * once at bootstrap; runtime reads are direct bounded ranges.
 */
export class BigContainerAssetLoader {
  private readonly assets = new Map<string, ChunkInfo>();

  constructor(
    private readonly source: FetchRangeLoader,
    private readonly containerPath: string,
    header: BigHeader,
  ) {
    for (const asset of header.assets) {
      if (asset.chunks.length !== 1 || asset.chunks[0].meta.type !== 'Raw') continue;
      const chunk = asset.chunks[0];
      if (chunk.compression !== 'None' || chunk.compressedSize !== chunk.uncompressedSize)
        throw new Error(`raw BIG asset must be uncompressed: ${asset.name}`);
      if (chunk.uncompressedSize > BigInt(Number.MAX_SAFE_INTEGER))
        throw new RangeError(`raw BIG asset exceeds browser safe size: ${asset.name}`);
      this.assets.set(asset.name, chunk);
    }
  }

  private chunk(path: string): ChunkInfo {
    const chunk = this.assets.get(path);
    if (!chunk) throw new Error(`raw BIG asset not found: ${path}`);
    return chunk;
  }

  load(path: string): Promise<Uint8Array> {
    const chunk = this.chunk(path);
    return this.source.read(this.containerPath, Number(chunk.offset), Number(chunk.uncompressedSize));
  }

  async size(path: string): Promise<number> { return Number(this.chunk(path).uncompressedSize); }

  read(path: string, offset: number, length: number): Promise<Uint8Array> {
    const chunk = this.chunk(path);
    const size = Number(chunk.uncompressedSize);
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 0 || offset + length > size)
      throw new RangeError(`raw BIG asset range exceeds ${path}: ${offset}+${length} > ${size}`);
    return this.source.read(this.containerPath, Number(chunk.offset) + offset, length);
  }

  poll(): void {}
}

export interface PageProviderStats {
  reads: number;
  averageReadMs: number;
  maxReadMs: number;
  workerCount: number;
  activeTranscodes: number;
  queuedTranscodes: number;
  completedTranscodes: number;
  averageTranscodeQueueMs: number;
  maxTranscodeQueueMs: number;
  averageTranscodeMs: number;
  maxTranscodeMs: number;
  cacheEnabled: boolean;
  cacheBackend: string;
  cacheEntries: number;
  cacheBytes: number;
  cacheLiveBytes: number;
  cacheQueuedWrites: number;
  cacheEvictions: number;
  cacheCompactions: number;
  cacheReclaimedBytes: number;
  cacheMaintenance: boolean;
  cacheHits: number;
  cacheMisses: number;
  cacheWrites: number;
  cacheRejected: number;
  cacheErrors: number;
  averageCacheReadMs: number;
  maxCacheReadMs: number;
  averageCacheWriteMs: number;
  maxCacheWriteMs: number;
}

export type VirtualTexturePageProvider = ((
  path: string,
  req: { mip: number; x: number; y: number; tail?: boolean },
  signal?: AbortSignal,
) => Promise<Uint8Array>) & { getStats(): Readonly<PageProviderStats> };

export function createPageDataProvider(
  loader: { read(path: string, offset: number, len: number): Promise<Uint8Array>; load(path: string): Promise<Uint8Array>; size(path: string): Promise<number> },
  header: BigHeader,
  textureWorkers: readonly { transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array> }[],
  format: number,
  cache?: PersistentBlobCache,
  transcodeQueueCapacity = 64,
): VirtualTexturePageProvider {
  interface RuntimeMipDirectory {
    pagesX: number;
    pagesY: number;
    offsets: Float64Array;
    sizes: Uint32Array;
  }
  interface RuntimeVTDirectory {
    assetId: number;
    encoding: TextureEncoding;
    mips: (RuntimeMipDirectory | null)[];
    tailOffset: number;
    tailSize: number;
  }
  // Expand compact on-disk size vectors once at header admission. Runtime page
  // lookup is direct typed-array indexing with no page objects or hash entries.
  const directories = new Map<string, RuntimeVTDirectory>();
  for (let assetId = 0; assetId < header.assets.length; assetId++) {
    const asset = header.assets[assetId];
    const source = asset.virtualTexture;
    if (!source) continue;
    let maxMip = 0;
    for (const mip of source.mips) maxMip = Math.max(maxMip, mip.mip);
    const mips: (RuntimeMipDirectory | null)[] = new Array(maxMip + 1).fill(null);
    for (const mip of source.mips) {
      const sizes = Uint32Array.from(mip.pageSizes);
      const offsets = new Float64Array(sizes.length);
      let offset = Number(mip.offset);
      for (let page = 0; page < sizes.length; page++) {
        offsets[page] = offset;
        offset += sizes[page];
      }
      mips[mip.mip] = { pagesX: mip.pagesX, pagesY: mip.pagesY, offsets, sizes };
    }
    directories.set(asset.name, {
      assetId, encoding: source.encoding, mips,
      tailOffset: source.tail ? Number(source.tail.offset) : 0,
      tailSize: source.tail?.size ?? 0,
    });
  }

  const transcoder = new BoundedTranscoderPool(textureWorkers, transcodeQueueCapacity);
  let reads = 0;
  let totalReadMs = 0;
  let maxReadMs = 0;
  const stats: PageProviderStats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0,
    workerCount: textureWorkers.length, activeTranscodes: 0, queuedTranscodes: 0,
    completedTranscodes: 0, averageTranscodeQueueMs: 0, maxTranscodeQueueMs: 0,
    averageTranscodeMs: 0, maxTranscodeMs: 0,
    cacheEnabled: cache !== undefined, cacheBackend: '', cacheEntries: 0, cacheBytes: 0,
    cacheLiveBytes: 0, cacheQueuedWrites: 0, cacheEvictions: 0, cacheCompactions: 0,
    cacheReclaimedBytes: 0, cacheMaintenance: false,
    cacheHits: 0, cacheMisses: 0, cacheWrites: 0,
    cacheRejected: 0, cacheErrors: 0,
    averageCacheReadMs: 0, maxCacheReadMs: 0,
    averageCacheWriteMs: 0, maxCacheWriteMs: 0,
  };

  const provider = (async (
    path: string,
    req: { mip: number; x: number; y: number; tail?: boolean },
    signal?: AbortSignal,
  ) => {
    if (signal?.aborted) throw new Error('VT page load canceled before read');
    const directory = directories.get(path);
    let offset = 0;
    let size = 0;
    if (req.tail) {
      offset = directory?.tailOffset ?? 0;
      size = directory?.tailSize ?? 0;
    } else {
      const mip = directory?.mips[req.mip];
      if (mip && req.x >= 0 && req.y >= 0 && req.x < mip.pagesX && req.y < mip.pagesY) {
        const page = req.y * mip.pagesX + req.x;
        offset = mip.offsets[page];
        size = mip.sizes[page];
      }
    }
    if (!directory || size === 0)
      throw new Error(`VT page not found: ${path} mip=${req.mip} (${req.x},${req.y})`);

    const cacheKey = `${directory.assetId}:${req.tail ? 't' : req.mip}:${req.x}:${req.y}`;
    const expectedBytes = format === 4 ? 136 * 136 * 4 : 34 * 34 * 16;
    if (cache) {
      const cached = await cache.get(cacheKey);
      if (signal?.aborted) throw new Error('VT page load canceled after cache read');
      if (cached && cached.byteLength === expectedBytes) return cached;
    }

    const readStartedAt = performance.now();
    const pageData = await loader.read(path + '.big', offset, size);
    const readMs = performance.now() - readStartedAt;
    reads++;
    totalReadMs += readMs;
    maxReadMs = Math.max(maxReadMs, readMs);
    if (signal?.aborted) throw new Error('VT page load canceled after read');

    if (directory.encoding === 'RawRgba8') {
      if (format !== 4) {
        throw new Error(`VT page ${path} is raw RGBA8 but GPU format ${format} requires Basis encoding`);
      }
      if (cache) void cache.put(cacheKey, pageData);
      return pageData;
    }

    // The worker returns [count][width][height][length][data]...; a VT page
    // consumes only the first image payload, never the serialization header.
    if (pageData.byteLength < 2 || pageData[0] !== 0x73 || pageData[1] !== 0x42)
      throw new Error(`invalid Basis page range for ${path}: bytes=${pageData.byteLength}, magic=${pageData[0]},${pageData[1]}`);
    // Each SPSC worker permits one in-flight transcode. The fixed pool dispatches
    // independently without an unbounded Promise chain and drops canceled jobs
    // before they reach a worker.
    const transcoded = await transcoder.submit(pageData, format, signal);
    if (signal?.aborted) throw new Error('VT page load canceled after transcode');
    if (transcoded.byteLength < 16) throw new Error('truncated transcoded VT page');
    const view = new DataView(transcoded.buffer, transcoded.byteOffset, transcoded.byteLength);
    const count = view.getUint32(0, true);
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const length = view.getUint32(12, true);
    if (count < 1 || width !== 136 || height !== 136 || 16 + length > transcoded.byteLength)
      throw new Error(`invalid transcoded VT page header: count=${count}, size=${width}x${height}, bytes=${length}`);
    const payload = transcoded.slice(16, 16 + length);
    if (cache) void cache.put(cacheKey, payload);
    return payload;
  }) as VirtualTexturePageProvider;
  provider.getStats = () => {
    const transcode = transcoder.getStats();
    stats.reads = reads;
    stats.averageReadMs = reads === 0 ? 0 : totalReadMs / reads;
    stats.maxReadMs = maxReadMs;
    stats.workerCount = transcode.workerCount;
    stats.activeTranscodes = transcode.active;
    stats.queuedTranscodes = transcode.queued;
    stats.completedTranscodes = transcode.completed;
    stats.averageTranscodeQueueMs = transcode.averageQueueMs;
    stats.maxTranscodeQueueMs = transcode.maxQueueMs;
    stats.averageTranscodeMs = transcode.averageTranscodeMs;
    stats.maxTranscodeMs = transcode.maxTranscodeMs;
    const persistent = cache?.getStats();
    if (persistent) {
      stats.cacheBackend = persistent.backend;
      stats.cacheEntries = persistent.entries;
      stats.cacheBytes = persistent.bytes;
      stats.cacheLiveBytes = persistent.liveBytes;
      stats.cacheQueuedWrites = persistent.queuedWrites;
      stats.cacheEvictions = persistent.evictions;
      stats.cacheCompactions = persistent.compactions;
      stats.cacheReclaimedBytes = persistent.reclaimedBytes;
      stats.cacheMaintenance = persistent.maintenance;
      stats.cacheHits = persistent.hits;
      stats.cacheMisses = persistent.misses;
      stats.cacheWrites = persistent.writes;
      stats.cacheRejected = persistent.rejectedCapacity + persistent.rejectedQueue;
      stats.cacheErrors = persistent.corruptEntries + persistent.readErrors + persistent.writeErrors;
      stats.averageCacheReadMs = persistent.averageReadMs;
      stats.maxCacheReadMs = persistent.maxReadMs;
      stats.averageCacheWriteMs = persistent.averageWriteMs;
      stats.maxCacheWriteMs = persistent.maxWriteMs;
    }
    return stats;
  };
  return provider;
}
