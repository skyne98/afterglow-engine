import {
  createFetchRangeLoader,
  type AssetIdentity,
  type FetchRangeLoader,
} from './big-parser.ts';
import type { AssetByteRange } from './bulk-range.ts';
import { EngineMetric, EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

const COPY_CHUNK_BYTES = 512 * 1024;

type NativeAssetOps = {
  op_native_asset_size(path: string): Promise<number>;
  op_native_asset_read_copy(path: string, offset: bigint, len: number): Promise<Uint8Array>;
};

declare const Deno: { core: { ops: Partial<NativeAssetOps> } } | undefined;

function nativeOps(): NativeAssetOps | null {
  if (typeof Deno !== 'object' || Deno === null) return null;
  const ops = Deno.core?.ops;
  return typeof ops?.op_native_asset_size === 'function' &&
    typeof ops.op_native_asset_read_copy === 'function'
    ? ops as NativeAssetOps
    : null;
}

function validateRead(offset: number, length: number): void {
  if (!Number.isSafeInteger(offset) || offset < 0 ||
      !Number.isSafeInteger(length) || length < 0)
    throw new RangeError('asset range must use non-negative safe integers');
}

/** Native JS-visible byte source. Payloads use the generated asset worker's
 * bounded RPC ring and become V8-owned arrays; no reusable native slot remains
 * leased to garbage collection. Native VT Basis pages bypass this adapter and
 * are read directly by source-backed texture workers. */
function createNativeRangeLoader(ops: NativeAssetOps): FetchRangeLoader {
  const identity = async (path: string): Promise<AssetIdentity> => ({
    size: await ops.op_native_asset_size(path),
    etag: null,
    lastModified: null,
  });
  const read = async (path: string, offset: number, length: number): Promise<Uint8Array> => {
    validateRead(offset, length);
    if (length === 0) return new Uint8Array(0);
    if (length <= COPY_CHUNK_BYTES)
      return ops.op_native_asset_read_copy(path, BigInt(offset), length);

    // Large raw assets are bootstrap/slow-path traffic. Keep each ring payload
    // bounded and give the returned array unambiguous V8 ownership.
    const output = new Uint8Array(length);
    let written = 0;
    while (written < length) {
      const requested = Math.min(COPY_CHUNK_BYTES, length - written);
      const chunk = await ops.op_native_asset_read_copy(
        path, BigInt(offset + written), requested,
      );
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
      return Promise.all(ranges.map(range => read(path, range.offset, range.length)));
    },
  };
}

function instrumentRangeLoader(loader: FetchRangeLoader, telemetry: EngineTelemetry): FetchRangeLoader {
  const duration = (startedAt: number): void => {
    telemetry.metrics.histogramLog2(
      EngineMetric.AssetReadNs,
      Math.max(1, Math.floor((performance.now() - startedAt) * 1_000_000)),
    );
  };
  return {
    async load(path: string): Promise<Uint8Array> {
      const correlation = telemetry.nextCorrelation(EngineTelemetryCategory.Asset);
      const startedAt = performance.now();
      telemetry.trace.asyncBegin(EngineTraceDescriptor.AssetRead, correlation, 0, 0);
      try {
        const bytes = await loader.load(path);
        telemetry.metrics.counterAdd(EngineMetric.AssetBytesRead, bytes.byteLength);
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetRead, correlation, bytes.byteLength, 0);
        return bytes;
      } catch (error) {
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetRead, correlation, 0, 1);
        throw error;
      } finally { duration(startedAt); }
    },
    async size(path: string): Promise<number> {
      const correlation = telemetry.nextCorrelation(EngineTelemetryCategory.Asset);
      telemetry.trace.asyncBegin(EngineTraceDescriptor.AssetSize, correlation, 0, 0);
      try {
        const size = await loader.size(path);
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetSize, correlation, size, 0);
        return size;
      } catch (error) {
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetSize, correlation, 0, 1);
        throw error;
      }
    },
    async identity(path: string): Promise<AssetIdentity> {
      const correlation = telemetry.nextCorrelation(EngineTelemetryCategory.Asset);
      telemetry.trace.asyncBegin(EngineTraceDescriptor.AssetSize, correlation, 0, 0);
      try {
        const identity = await loader.identity(path);
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetSize, correlation, identity.size, 0);
        return identity;
      } catch (error) {
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetSize, correlation, 0, 1);
        throw error;
      }
    },
    async read(path: string, offset: number, length: number): Promise<Uint8Array> {
      const correlation = telemetry.nextCorrelation(EngineTelemetryCategory.Asset);
      const startedAt = performance.now();
      telemetry.trace.asyncBegin(EngineTraceDescriptor.AssetRead, correlation, length, offset);
      try {
        const bytes = await loader.read(path, offset, length);
        telemetry.metrics.counterAdd(EngineMetric.AssetBytesRead, bytes.byteLength);
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetRead, correlation, bytes.byteLength, 0);
        return bytes;
      } catch (error) {
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetRead, correlation, 0, 1);
        throw error;
      } finally { duration(startedAt); }
    },
    readBulk: loader.readBulk === undefined ? undefined : async (
      path: string,
      ranges: readonly AssetByteRange[],
    ): Promise<Uint8Array[]> => {
      const correlation = telemetry.nextCorrelation(EngineTelemetryCategory.Asset);
      const startedAt = performance.now();
      let requested = 0;
      for (let index = 0; index < ranges.length; index++) requested += ranges[index]?.length ?? 0;
      telemetry.trace.asyncBegin(EngineTraceDescriptor.AssetBulkRead, correlation, requested, ranges.length);
      try {
        const parts = await loader.readBulk!(path, ranges);
        let bytes = 0;
        for (let index = 0; index < parts.length; index++) bytes += parts[index]?.byteLength ?? 0;
        telemetry.metrics.counterAdd(EngineMetric.AssetBytesRead, bytes);
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetBulkRead, correlation, bytes, parts.length);
        return parts;
      } catch (error) {
        telemetry.trace.asyncEnd(EngineTraceDescriptor.AssetBulkRead, correlation, 0, 1);
        throw error;
      } finally { duration(startedAt); }
    },
  };
}

/** Shared platform selector: native RPC bytes for JS-visible assets, Fetch on web. */
export function createPlatformRangeLoader(baseUrl = '', telemetry?: EngineTelemetry): FetchRangeLoader {
  const ops = nativeOps();
  const loader = ops === null ? createFetchRangeLoader(baseUrl) : createNativeRangeLoader(ops);
  return telemetry === undefined ? loader : instrumentRangeLoader(loader, telemetry);
}
