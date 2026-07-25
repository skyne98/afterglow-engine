import {
  createFetchRangeLoader,
  type AssetIdentity,
  type FetchRangeLoader,
} from './big-parser.ts';
import type { AssetByteRange } from './bulk-range.ts';

const ARENA_SLOT_BYTES = 4 * 1024 * 1024;
const COPY_CHUNK_BYTES = 512 * 1024;
const MAX_BULK_SPANS = 256;

type HandleWords = readonly number[];
type NativeAssetOps = {
  op_native_asset_size(path: string): Promise<number>;
  op_native_asset_read_copy(path: string, offset: bigint, len: number): Promise<Uint8Array>;
  op_native_asset_read_handle(path: string, offset: bigint, len: number): Promise<number[]>;
  op_native_asset_read_many_handle(path: string, spans: Uint8Array): Promise<number[]>;
  op_afterglow_arena_view(handle: {
    region: number; slot: number; length: number; generation: number;
  }): Uint8Array;
};

declare const Deno: { core: { ops: Partial<NativeAssetOps> } } | undefined;

function nativeOps(): NativeAssetOps | null {
  if (typeof Deno !== 'object' || Deno === null) return null;
  const ops = Deno.core?.ops;
  return typeof ops?.op_native_asset_size === 'function' &&
    typeof ops.op_native_asset_read_handle === 'function' &&
    typeof ops.op_native_asset_read_many_handle === 'function' &&
    typeof ops.op_afterglow_arena_view === 'function'
    ? ops as NativeAssetOps
    : null;
}

function validateRead(offset: number, length: number): void {
  if (!Number.isSafeInteger(offset) || offset < 0 ||
      !Number.isSafeInteger(length) || length < 0)
    throw new RangeError('asset range must use non-negative safe integers');
}

function viewHandle(ops: NativeAssetOps, words: HandleWords): Uint8Array {
  if (words.length < 4) throw new Error('native asset worker returned truncated handle metadata');
  const region = words[0] ?? -1;
  const slot = words[1] ?? -1;
  const length = words[2] ?? -1;
  const generation = words[3] ?? -1;
  if (![region, slot, length, generation].every(Number.isInteger))
    throw new Error('native asset worker returned invalid handle metadata');
  const bytes = ops.op_afterglow_arena_view({ region, slot, length, generation });
  if (bytes.byteLength !== length) throw new Error('native asset arena view length mismatch');
  return bytes;
}

function packSpans(ranges: readonly AssetByteRange[]): Uint8Array {
  if (ranges.length === 0 || ranges.length > MAX_BULK_SPANS)
    throw new RangeError(`native bulk read requires 1..${MAX_BULK_SPANS} spans`);
  const packed = new Uint8Array(ranges.length * 12);
  const view = new DataView(packed.buffer);
  let total = 0;
  for (let index = 0; index < ranges.length; index++) {
    const range = ranges[index]!;
    validateRead(range.offset, range.length);
    total += range.length;
    if (!Number.isSafeInteger(total) || total > ARENA_SLOT_BYTES)
      throw new RangeError(`native bulk read exceeds ${ARENA_SLOT_BYTES} bytes`);
    view.setBigUint64(index * 12, BigInt(range.offset), true);
    view.setUint32(index * 12 + 8, range.length, true);
  }
  return packed;
}

function createNativeRangeLoader(ops: NativeAssetOps): FetchRangeLoader {
  const identity = async (path: string): Promise<AssetIdentity> => ({
    size: await ops.op_native_asset_size(path),
    etag: null,
    lastModified: null,
  });
  const read = async (path: string, offset: number, length: number): Promise<Uint8Array> => {
    validateRead(offset, length);
    if (length === 0) return new Uint8Array(0);
    if (length <= ARENA_SLOT_BYTES) {
      return viewHandle(ops, await ops.op_native_asset_read_handle(path, BigInt(offset), length));
    }

    // Compatibility path for large monolithic assets. Streaming BIG/VT reads
    // use the zero-copy arena path above; this bounded copy path never asks the
    // 1 MiB RPC ring to carry more than 512 KiB.
    const output = new Uint8Array(length);
    let written = 0;
    while (written < length) {
      const requested = Math.min(COPY_CHUNK_BYTES, length - written);
      const chunk = await ops.op_native_asset_read_copy(path, BigInt(offset + written), requested);
      output.set(chunk, written);
      written += chunk.byteLength;
      if (chunk.byteLength !== requested) return output.subarray(0, written);
    }
    return output;
  };
  return {
    async load(path: string): Promise<Uint8Array> {
      const size = await ops.op_native_asset_size(path);
      return read(path, 0, size);
    },
    async size(path: string): Promise<number> { return ops.op_native_asset_size(path); },
    identity,
    read,
    async readBulk(path: string, ranges: readonly AssetByteRange[]): Promise<Uint8Array[]> {
      const metadata = await ops.op_native_asset_read_many_handle(path, packSpans(ranges));
      if (metadata.length !== 4 + ranges.length)
        throw new Error('native bulk read returned invalid part metadata');
      const bytes = viewHandle(ops, metadata);
      const parts = new Array<Uint8Array>(ranges.length);
      let offset = 0;
      for (let index = 0; index < ranges.length; index++) {
        const length = metadata[4 + index] ?? -1;
        if (!Number.isInteger(length) || length < 0 || offset + length > bytes.byteLength)
          throw new Error('native bulk read returned invalid part length');
        parts[index] = bytes.subarray(offset, offset + length);
        offset += length;
      }
      if (offset !== bytes.byteLength)
        throw new Error('native bulk read metadata does not cover its arena view');
      return parts;
    },
  };
}

/** Shared platform selector: native arena worker in afterglow-shell, Fetch on web. */
export function createPlatformRangeLoader(baseUrl = ''): FetchRangeLoader {
  const ops = nativeOps();
  return ops === null ? createFetchRangeLoader(baseUrl) : createNativeRangeLoader(ops);
}
