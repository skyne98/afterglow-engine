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

import {
  BULK_IN_FLIGHT_MAX_BYTES,
  BULK_RANGE_CAPACITY,
  BULK_RESPONSE_MAX_BYTES,
  estimatedBulkResponseBytes,
  fetchByteRanges,
  type AssetByteRange,
} from './bulk-range.ts';

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
  /** One bounded multipart response for non-contiguous source spans. */
  readBulk?(path: string, ranges: readonly AssetByteRange[]): Promise<Uint8Array[]>;
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
      return (await fetchByteRanges(url(path), [{ offset, length: len }]))[0];
    },
    async readBulk(path: string, ranges: readonly AssetByteRange[]): Promise<Uint8Array[]> {
      return fetchByteRanges(url(path), ranges);
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
  bulkQueued: number;
  bulkInFlight: number;
  bulkInFlightBytes: number;
  urgentBatches: number;
  qualityBatches: number;
  bulkRejected: number;
  bulkCanceled: number;
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

export type PageLoadTier = 'urgent' | 'quality';

export type VirtualTexturePageProvider = ((
  path: string,
  req: { mip: number; x: number; y: number; tail?: boolean; batchTier?: PageLoadTier },
  signal?: AbortSignal,
) => Promise<Uint8Array>) & {
  getStats(): Readonly<PageProviderStats>;
  close(): void;
};

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

/** Expand the compact on-disk per-mip size vectors once into direct-indexed
 *  typed arrays (offset + size per page). Shared by the single-page provider and
 *  the batch range reader so page lookup is `y * pagesX + x` with no per-page
 *  objects or hash entries. */
function expandVtDirectories(header: BigHeader): Map<string, RuntimeVTDirectory> {
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
  return directories;
}

/** A page location to read (matches the scheduler's `PageRequest`). */
export interface PageReadRequest {
  path: string;
  mip: number;
  x: number;
  y: number;
  /** Selects the packed mip tail instead of a regular page. */
  tail?: boolean;
}

export interface PageRangeReaderStats {
  reads: number;
  averageReadMs: number;
  maxReadMs: number;
  batches: number;
  pagesRequested: number;
  /** Pages served by a multi-page coalesced run (not a singleton read). */
  pagesCoalesced: number;
  runs: number;
}

/** Batch page-range reader: the client-side primitive for reading many pages in
 * one call. It coalesces contiguous pages (same mip row, adjacent x, uniform
 * size, contiguous offset) into single range reads, so one fetch amortizes the
 * per-request connection floor across K pages. Returns raw (pre-transcode)
 * bytes in request order; transcode/OPFS are the caller's policy.
 *
 * This is the infrastructure the scheduler will call; it is decoupled from the
 * single-page provider's transcode/cache policy. */
export interface PageRangeReader {
  readBatch(requests: readonly PageReadRequest[], signal?: AbortSignal): Promise<Uint8Array[]>;
  getStats(): Readonly<PageRangeReaderStats>;
}

interface ResolvedPage {
  index: number;
  path: string;
  mip: number;
  x: number;
  y: number;
  tail: boolean;
  offset: number;
  size: number;
}

/** Create a batch page-range reader over a `.big` container. `readConcurrency`
 * bounds in-flight range reads per `readBatch` call. */
export function createPageRangeReader(
  loader: {
    read(path: string, offset: number, len: number): Promise<Uint8Array>;
    readBulk?(path: string, ranges: readonly AssetByteRange[]): Promise<Uint8Array[]>;
  },
  header: BigHeader,
  readConcurrency = 16,
): PageRangeReader {
  const directories = expandVtDirectories(header);
  let reads = 0, totalReadMs = 0, maxReadMs = 0;
  let batches = 0, pagesRequested = 0, pagesCoalesced = 0, runs = 0;
  const stats: PageRangeReaderStats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0,
    batches: 0, pagesRequested: 0, pagesCoalesced: 0, runs: 0,
  };

  const resolve = (req: PageReadRequest, index: number): ResolvedPage => {
    const dir = directories.get(req.path);
    if (!dir) throw new Error(`VT directory not found: ${req.path}`);
    let offset = 0, size = 0;
    if (req.tail) {
      offset = dir.tailOffset;
      size = dir.tailSize;
    } else {
      const mip = dir.mips[req.mip];
      if (!mip || req.x < 0 || req.y < 0 || req.x >= mip.pagesX || req.y >= mip.pagesY)
        throw new Error(`VT page out of range: ${req.path} mip=${req.mip} (${req.x},${req.y})`);
      const page = req.y * mip.pagesX + req.x;
      offset = mip.offsets[page];
      size = mip.sizes[page];
    }
    if (size === 0) throw new Error(`VT page not found: ${req.path} mip=${req.mip} (${req.x},${req.y})`);
    return { index, path: req.path, mip: req.mip, x: req.x, y: req.y, tail: !!req.tail, offset, size };
  };

  /** Split a same-(path,mip,y) group (sorted by x) into maximal contiguous
   *  uniform-size runs; each run is one range read. */
  const coalesce = (group: ResolvedPage[]): ResolvedPage[][] => {
    const out: ResolvedPage[][] = [];
    let runStart = 0;
    for (let i = 1; i <= group.length; i++) {
      const prev = group[i - 1];
      const cur = i < group.length ? group[i] : null;
      const contiguous = cur !== null
        && cur.x === prev.x + 1 && cur.size === prev.size
        && cur.offset === prev.offset + prev.size;
      if (!contiguous) {
        out.push(group.slice(runStart, i));
        runStart = i;
      }
    }
    return out;
  };

  const readBatch = async (
    requests: readonly PageReadRequest[],
    signal?: AbortSignal,
  ): Promise<Uint8Array[]> => {
    if (signal?.aborted) throw new Error('batch read canceled');
    batches++;
    pagesRequested += requests.length;
    const results = new Array<Uint8Array>(requests.length);
    const resolved = requests.map(resolve);
    if (loader.readBulk) {
      // Bulk transports may reorder independent requests. Source order turns
      // adjacent pages into long sequential reads while `page.index` restores
      // the caller's original order without copying payload bytes.
      const ordered = resolved.slice().sort((left, right) =>
        left.path === right.path
          ? left.offset - right.offset
          : left.path.localeCompare(right.path));
      const groups: ResolvedPage[][] = [];
      let group: ResolvedPage[] = [];
      let ranges: AssetByteRange[] = [];
      for (const page of ordered) {
        const candidate = { offset: page.offset, length: page.size };
        if (group.length !== 0 && group[0].path !== page.path) {
          groups.push(group);
          group = [];
          ranges = [];
        }
        ranges.push(candidate);
        if (ranges.length > BULK_RANGE_CAPACITY ||
            estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES) {
          ranges.pop();
          if (group.length === 0) throw new RangeError('one page exceeds bulk response capacity');
          groups.push(group);
          group = [];
          ranges = [candidate];
        }
        group.push(page);
      }
      if (group.length !== 0) groups.push(group);
      const readGroup = async (pages: ResolvedPage[]): Promise<void> => {
        if (signal?.aborted) throw new Error('batch read canceled');
        const spans = pages.map(page => ({ offset: page.offset, length: page.size }));
        const readStartedAt = performance.now();
        const parts = await loader.readBulk!(pages[0].path + '.big', spans);
        const readMs = performance.now() - readStartedAt;
        if (parts.length !== pages.length) throw new Error('bulk page response count mismatch');
        reads++;
        totalReadMs += readMs;
        maxReadMs = Math.max(maxReadMs, readMs);
        runs++;
        if (pages.length > 1) pagesCoalesced += pages.length;
        for (let index = 0; index < pages.length; index++) {
          if (parts[index].byteLength !== pages[index].size)
            throw new Error('bulk page response length mismatch');
          results[pages[index].index] = parts[index];
        }
      };
      let nextGroup = 0;
      const concurrency = Math.min(2, readConcurrency);
      await Promise.all(Array.from({ length: concurrency }, async () => {
        while (true) {
          const index = nextGroup++;
          if (index >= groups.length) return;
          await readGroup(groups[index]);
        }
      }));
      return results;
    }
    // Group by row (path, mip, y) so adjacent-x pages land together; tails apart.
    const groups = new Map<string, ResolvedPage[]>();
    for (const r of resolved) {
      const key = r.tail ? `${r.path}:tail:${r.mip}` : `${r.path}:${r.mip}:${r.y}`;
      let g = groups.get(key);
      if (!g) { g = []; groups.set(key, g); }
      g.push(r);
    }
    const allRuns: ResolvedPage[][] = [];
    for (const group of groups.values()) {
      if (group[0].tail) {
        for (const r of group) allRuns.push([r]);
      } else {
        group.sort((a, b) => a.x - b.x);
        for (const run of coalesce(group)) allRuns.push(run);
      }
    }
    const readRun = async (run: ResolvedPage[]): Promise<void> => {
      const runOffset = run[0].offset;
      let runSize = 0;
      for (const p of run) runSize += p.size;
      const readStartedAt = performance.now();
      const batchData = await loader.read(run[0].path + '.big', runOffset, runSize);
      const readMs = performance.now() - readStartedAt;
      reads++;
      totalReadMs += readMs;
      maxReadMs = Math.max(maxReadMs, readMs);
      runs++;
      if (run.length > 1) pagesCoalesced += run.length;
      let rel = 0;
      for (const p of run) {
        results[p.index] = batchData.subarray(rel, rel + p.size);
        rel += p.size;
      }
    };
    let next = 0;
    await Promise.all(Array.from({ length: readConcurrency }, async () => {
      while (true) {
        const i = next++;
        if (i >= allRuns.length) return;
        await readRun(allRuns[i]);
      }
    }));
    return results;
  };

  return {
    readBatch,
    getStats() {
      stats.reads = reads;
      stats.averageReadMs = reads === 0 ? 0 : totalReadMs / reads;
      stats.maxReadMs = maxReadMs;
      stats.batches = batches;
      stats.pagesRequested = pagesRequested;
      stats.pagesCoalesced = pagesCoalesced;
      stats.runs = runs;
      return stats;
    },
  };
}

interface BulkReadLoader {
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
  /** Runtime sessions bind this directly to their one cooked container. */
  readBulk?(ranges: readonly AssetByteRange[]): Promise<Uint8Array[]>;
}

interface BulkReadSlot {
  path: string;
  offset: number;
  length: number;
  signal: AbortSignal | undefined;
  resolve: ((bytes: Uint8Array) => void) | null;
  reject: ((error: unknown) => void) | null;
}

/** Fixed-capacity two-deadline raw-byte queue. Timers are opened by the first
 * miss and never reset, so continuous arrivals cannot postpone a lane's ready
 * deadline. Dispatch still gives ready urgent work strict priority; sustained
 * urgent demand can defer the quality lane. */
class BoundedBulkReadQueue {
  private readonly slots: BulkReadSlot[] = new Array(BULK_RANGE_CAPACITY);
  private readonly free = new Uint16Array(BULK_RANGE_CAPACITY);
  private freeTop = 0;
  private readonly queued = [
    new Uint16Array(BULK_RANGE_CAPACITY),
    new Uint16Array(BULK_RANGE_CAPACITY),
  ];
  private readonly heads = new Uint16Array(2);
  private readonly tails = new Uint16Array(2);
  private readonly counts = new Uint16Array(2);
  private readonly ready = new Uint8Array(2);
  private readonly timers: Array<ReturnType<typeof setTimeout> | null> = [null, null];
  private inFlight = 0;
  private inFlightBytes = 0;
  private closed = false;
  private reads = 0;
  private totalReadMs = 0;
  private maxReadMs = 0;
  private urgentBatches = 0;
  private qualityBatches = 0;
  private rejected = 0;
  private canceled = 0;
  private readonly stats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0, queued: 0,
    inFlight: 0, inFlightBytes: 0, urgentBatches: 0, qualityBatches: 0,
    rejected: 0, canceled: 0,
  };

  constructor(private readonly loader: BulkReadLoader) {
    for (let index = BULK_RANGE_CAPACITY - 1; index >= 0; index--) {
      this.slots[index] = {
        path: '', offset: 0, length: 0, signal: undefined, resolve: null, reject: null,
      };
      this.free[this.freeTop++] = index;
    }
  }

  private tierIndex(tier: PageLoadTier): number { return tier === 'urgent' ? 0 : 1; }
  private deadlineMs(tier: number): number { return tier === 0 ? 1 : 100; }

  read(
    path: string,
    offset: number,
    length: number,
    tier: PageLoadTier,
    signal?: AbortSignal,
  ): Promise<Uint8Array> {
    return new Promise<Uint8Array>((resolve, reject) => {
      if (this.closed) { this.rejected++; reject(new Error('bulk page reader is closed')); return; }
      if (signal?.aborted) {
        this.canceled++;
        reject(new Error('VT page load canceled before batching'));
        return;
      }
      if (this.freeTop === 0) {
        this.rejected++;
        reject(new Error('bulk page queue capacity exceeded'));
        return;
      }
      const slotIndex = this.free[--this.freeTop];
      const slot = this.slots[slotIndex];
      slot.path = path;
      slot.offset = offset;
      slot.length = length;
      slot.signal = signal;
      slot.resolve = resolve;
      slot.reject = reject;
      const lane = this.tierIndex(tier);
      this.queued[lane][this.tails[lane]] = slotIndex;
      this.tails[lane] = (this.tails[lane] + 1) % BULK_RANGE_CAPACITY;
      this.counts[lane]++;
      if (this.timers[lane] === null) {
        this.timers[lane] = setTimeout(() => {
          this.timers[lane] = null;
          this.ready[lane] = 1;
          this.pump();
        }, this.deadlineMs(lane));
      }
      if (this.counts[lane] === BULK_RANGE_CAPACITY) {
        this.ready[lane] = 1;
        this.pump();
      }
    });
  }

  private release(slotIndex: number): void {
    const slot = this.slots[slotIndex];
    slot.path = '';
    slot.signal = undefined;
    slot.resolve = null;
    slot.reject = null;
    this.free[this.freeTop++] = slotIndex;
  }

  private pop(lane: number): number {
    const index = this.queued[lane][this.heads[lane]];
    this.heads[lane] = (this.heads[lane] + 1) % BULK_RANGE_CAPACITY;
    this.counts[lane]--;
    return index;
  }

  private clearLaneTimer(lane: number): void {
    const timer = this.timers[lane];
    if (timer !== null) clearTimeout(timer);
    this.timers[lane] = null;
  }

  private pump(): void {
    while (this.inFlight < 2 && this.inFlightBytes < BULK_IN_FLIGHT_MAX_BYTES) {
      const lane = this.ready[0] !== 0 && this.counts[0] !== 0
        ? 0
        : this.ready[1] !== 0 && this.counts[1] !== 0 ? 1 : -1;
      if (lane < 0) return;
      const indices: number[] = [];
      const ranges: AssetByteRange[] = [];
      while (this.counts[lane] !== 0 && indices.length < BULK_RANGE_CAPACITY) {
        const slotIndex = this.queued[lane][this.heads[lane]];
        const slot = this.slots[slotIndex];
        if (slot.signal?.aborted) {
          this.pop(lane);
          this.canceled++;
          slot.reject?.(new Error('VT page load canceled while batched'));
          this.release(slotIndex);
          continue;
        }
        const candidate = { offset: slot.offset, length: slot.length };
        ranges.push(candidate);
        if (estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES) {
          ranges.pop();
          if (indices.length === 0) {
            this.pop(lane);
            this.rejected++;
            slot.reject?.(new RangeError('one VT page exceeds bulk response capacity'));
            this.release(slotIndex);
            continue;
          }
          break;
        }
        indices.push(this.pop(lane));
      }
      if (this.counts[lane] === 0) {
        this.ready[lane] = 0;
        this.clearLaneTimer(lane);
      }
      if (indices.length === 0) continue;
      const expectedBytes = estimatedBulkResponseBytes(ranges);
      if (this.inFlightBytes + expectedBytes > BULK_IN_FLIGHT_MAX_BYTES) return;
      this.dispatch(indices, ranges, expectedBytes, lane);
    }
  }

  private dispatch(
    indices: number[],
    ranges: AssetByteRange[],
    expectedBytes: number,
    lane: number,
  ): void {
    this.inFlight++;
    this.inFlightBytes += expectedBytes;
    if (lane === 0) this.urgentBatches++;
    else this.qualityBatches++;
    const startedAt = performance.now();
    const request = this.loader.readBulk
      ? this.loader.readBulk(ranges)
      : Promise.all(indices.map((slotIndex, index) => {
          const slot = this.slots[slotIndex];
          const range = ranges[index];
          return this.loader.read(`${slot.path}.big`, range.offset, range.length);
        }));
    request.then(parts => {
      if (parts.length !== indices.length)
        throw new Error(`bulk response returned ${parts.length} parts; expected ${indices.length}`);
      const readMs = performance.now() - startedAt;
      this.reads++;
      this.totalReadMs += readMs;
      this.maxReadMs = Math.max(this.maxReadMs, readMs);
      for (let index = 0; index < indices.length; index++) {
        const slotIndex = indices[index];
        const slot = this.slots[slotIndex];
        const bytes = parts[index];
        if (bytes.byteLength !== slot.length)
          slot.reject?.(new Error(`bulk page returned ${bytes.byteLength} bytes; expected ${slot.length}`));
        else if (this.closed || slot.signal?.aborted) {
          this.canceled++;
          slot.reject?.(new Error('VT page load canceled after bulk read'));
        } else slot.resolve?.(bytes);
        this.release(slotIndex);
      }
    }).catch(error => {
      for (const slotIndex of indices) {
        this.slots[slotIndex].reject?.(error);
        this.release(slotIndex);
      }
    }).finally(() => {
      this.inFlight--;
      this.inFlightBytes -= expectedBytes;
      this.pump();
    });
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (let lane = 0; lane < 2; lane++) {
      this.clearLaneTimer(lane);
      while (this.counts[lane] !== 0) {
        const slotIndex = this.pop(lane);
        this.canceled++;
        this.slots[slotIndex].reject?.(new Error('bulk page reader closed'));
        this.release(slotIndex);
      }
      this.ready[lane] = 0;
    }
  }

  getStats(): Readonly<{
    reads: number;
    averageReadMs: number;
    maxReadMs: number;
    queued: number;
    inFlight: number;
    inFlightBytes: number;
    urgentBatches: number;
    qualityBatches: number;
    rejected: number;
    canceled: number;
  }> {
    const stats = this.stats;
    stats.reads = this.reads;
    stats.averageReadMs = this.reads === 0 ? 0 : this.totalReadMs / this.reads;
    stats.maxReadMs = this.maxReadMs;
    stats.queued = this.counts[0] + this.counts[1];
    stats.inFlight = this.inFlight;
    stats.inFlightBytes = this.inFlightBytes;
    stats.urgentBatches = this.urgentBatches;
    stats.qualityBatches = this.qualityBatches;
    stats.rejected = this.rejected;
    stats.canceled = this.canceled;
    return stats;
  }
}

export function createPageDataProvider(
  loader: BulkReadLoader & { load(path: string): Promise<Uint8Array>; size(path: string): Promise<number> },
  header: BigHeader,
  textureWorkers: readonly { transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array> }[],
  format: number,
  cache?: PersistentBlobCache,
  transcodeQueueCapacity = 64,
): VirtualTexturePageProvider {
  const directories = expandVtDirectories(header);

  const transcoder = new BoundedTranscoderPool(textureWorkers, transcodeQueueCapacity);
  const bulkReads = new BoundedBulkReadQueue(loader);
  const stats: PageProviderStats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0,
    bulkQueued: 0, bulkInFlight: 0, bulkInFlightBytes: 0,
    urgentBatches: 0, qualityBatches: 0, bulkRejected: 0, bulkCanceled: 0,
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
    req: { mip: number; x: number; y: number; tail?: boolean; batchTier?: PageLoadTier },
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

    const pageData = await bulkReads.read(
      path,
      offset,
      size,
      req.batchTier ?? 'urgent',
      signal,
    );
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
  provider.close = () => bulkReads.close();
  provider.getStats = () => {
    const transcode = transcoder.getStats();
    const read = bulkReads.getStats();
    stats.reads = read.reads;
    stats.averageReadMs = read.averageReadMs;
    stats.maxReadMs = read.maxReadMs;
    stats.bulkQueued = read.queued;
    stats.bulkInFlight = read.inFlight;
    stats.bulkInFlightBytes = read.inFlightBytes;
    stats.urgentBatches = read.urgentBatches;
    stats.qualityBatches = read.qualityBatches;
    stats.bulkRejected = read.rejected;
    stats.bulkCanceled = read.canceled;
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
