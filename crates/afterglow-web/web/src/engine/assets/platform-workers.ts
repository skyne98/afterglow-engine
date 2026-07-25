import { MeshoptClient } from '../../workers/meshopt.client.ts';
import { NativeRpcTransport } from '../workers/native-transport.ts';
import { TextureClient } from '../../workers/texture.client.ts';
import type { OwnedMeshOptimizer, OwnedTextureTranscoder } from './big-asset-session.ts';

const NATIVE_TEXTURE_WORKER_FIRST = 1;
const NATIVE_TEXTURE_WORKER_COUNT = 4;
const NATIVE_MESHOPT_WORKER = 5;

type NativeDeno = { core?: { ops?: { op_afterglow_rpc_call_async?: unknown } } };

export function hasNativeWorkerTransport(): boolean {
  const deno = (globalThis as typeof globalThis & { Deno?: NativeDeno }).Deno;
  return typeof deno?.core?.ops?.op_afterglow_rpc_call_async === 'function';
}

export async function createPlatformTextureTranscoder(index: number): Promise<OwnedTextureTranscoder> {
  if (!hasNativeWorkerTransport())
    return TextureClient.spawnThreaded({ workerWasmUrl: 'texture.wasm', timeoutMs: 10_000 });
  if (!Number.isInteger(index) || index < 0 || index >= NATIVE_TEXTURE_WORKER_COUNT)
    throw new RangeError(`native texture worker index must be 0..${NATIVE_TEXTURE_WORKER_COUNT - 1}`);
  return new TextureClient(new NativeRpcTransport(NATIVE_TEXTURE_WORKER_FIRST + index));
}

export async function createPlatformMeshOptimizer(): Promise<OwnedMeshOptimizer> {
  if (!hasNativeWorkerTransport())
    return MeshoptClient.spawnThreaded({ workerWasmUrl: 'meshopt.wasm', timeoutMs: 10_000 });
  return new MeshoptClient(new NativeRpcTransport(NATIVE_MESHOPT_WORKER));
}
