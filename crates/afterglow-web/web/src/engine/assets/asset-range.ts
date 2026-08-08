import { fetchByteRanges, type AssetByteRange } from './bulk-range.ts';
import {
  BIG_MAGIC,
  BIG_MIN_READABLE_VERSION,
  BIG_VERSION,
  parseBigHeader,
  type BigHeader,
} from './big-format.ts';

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
  readBulk?: ((path: string, ranges: readonly AssetByteRange[]) => Promise<Uint8Array[]>) | undefined;
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

/** Public-web serving-layer loader. Large assets use exact HTTP ranges. */
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
