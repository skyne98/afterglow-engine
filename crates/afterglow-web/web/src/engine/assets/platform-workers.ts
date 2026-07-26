import { MeshoptClient } from '../../workers/meshopt.client.ts';
import { NativeRpcTransport } from '../workers/native-transport.ts';
import { TextureClient } from '../../workers/texture.client.ts';
import type { OwnedMeshOptimizer, OwnedTextureTranscoder } from './service-types.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

type NativeOps = {
  op_afterglow_rpc_call_async?: unknown;
  op_afterglow_worker_ids?: (service: string) => number[];
};
type NativeDeno = { core?: { ops?: NativeOps } };

function nativeOps(): NativeOps | undefined {
  return (globalThis as typeof globalThis & { Deno?: NativeDeno }).Deno?.core?.ops;
}

export function hasNativeWorkerTransport(): boolean {
  return typeof nativeOps()?.op_afterglow_rpc_call_async === 'function';
}

function nativeWorkerIds(service: string): number[] {
  const resolve = nativeOps()?.op_afterglow_worker_ids;
  if (typeof resolve !== 'function')
    throw new Error('native worker manifest op is unavailable');
  const ids = resolve(service);
  if (!Array.isArray(ids) || ids.some(id => !Number.isInteger(id) || id < 0))
    throw new Error(`native worker manifest is invalid for ${service}`);
  return ids;
}

/** Select the bounded platform profile without exposing worker ids to games. */
export function platformTextureWorkerCount(maxWorkers: number): number {
  if (!Number.isInteger(maxWorkers) || maxWorkers <= 0)
    throw new RangeError('texture worker limit must be positive');
  if (hasNativeWorkerTransport()) {
    const available = nativeWorkerIds('texture').length;
    if (available === 0) throw new Error('native texture worker manifest is empty');
    return Math.min(available, maxWorkers);
  }
  const hardwareThreads = globalThis.navigator?.hardwareConcurrency || 4;
  return Math.min(maxWorkers, Math.max(2, Math.min(4, Math.floor(hardwareThreads / 2))));
}

export async function createPlatformTextureTranscoder(
  index: number,
  sourcePath: string,
  telemetry?: EngineTelemetry,
): Promise<OwnedTextureTranscoder> {
  if (!hasNativeWorkerTransport())
    return TextureClient.spawnThreaded({ workerWasmUrl: 'texture.wasm', timeoutMs: 10_000 });
  const ids = nativeWorkerIds('texture');
  if (!Number.isInteger(index) || index < 0 || index >= ids.length)
    throw new RangeError(`native texture worker index must be 0..${ids.length - 1}`);
  const client = new TextureClient(
    new NativeRpcTransport(ids[index]!, telemetry),
  );
  const source = await client.openSource(sourcePath);
  return {
    responseIsOwned: true,
    transcode(data, targetFormat) {
      return client.transcode(data, targetFormat);
    },
    transcodeSourceRange(offset, length, targetFormat) {
      return client.transcodeRange(source, offset, length, targetFormat);
    },
    close() {
      client.close();
    },
  };
}

export async function createPlatformMeshOptimizer(telemetry?: EngineTelemetry): Promise<OwnedMeshOptimizer> {
  if (!hasNativeWorkerTransport())
    return MeshoptClient.spawnThreaded({ workerWasmUrl: 'meshopt.wasm', timeoutMs: 10_000 });
  const ids = nativeWorkerIds('meshopt');
  if (ids.length !== 1)
    throw new Error(`native meshopt service requires exactly one worker; found ${ids.length}`);
  return new MeshoptClient(new NativeRpcTransport(ids[0]!, telemetry));
}
