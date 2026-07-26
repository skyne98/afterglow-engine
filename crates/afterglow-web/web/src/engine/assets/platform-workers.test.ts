import { afterEach, expect, test } from 'bun:test';
import {
  createPlatformMeshOptimizer,
  createPlatformTextureTranscoder,
  platformTextureWorkerCount,
} from './platform-workers.ts';

afterEach(() => { delete (globalThis as typeof globalThis & { Deno?: unknown }).Deno; });

test('native services resolve bootstrap manifests instead of hardcoded ids', async () => {
  const calls: Array<[number, number]> = [];
  (globalThis as typeof globalThis & { Deno?: unknown }).Deno = { core: { ops: {
    op_afterglow_worker_ids(service: string) {
      return service === 'texture' ? [41, 9] : service === 'meshopt' ? [77] : [];
    },
    async op_afterglow_rpc_call_async(worker: number, method: number) {
      calls.push([worker, method]);
      return new Uint8Array([7, 0, 0, 0]);
    },
  } } };

  expect(platformTextureWorkerCount(16)).toBe(2);
  expect(platformTextureWorkerCount(1)).toBe(1);
  const texture = await createPlatformTextureTranscoder(1, 'scene.big');
  expect(calls).toEqual([[9, 3]]); // openSource on the selected worker
  texture.close();
  const mesh = await createPlatformMeshOptimizer();
  mesh.close();
});

test('native service manifests fail explicitly when topology is missing', async () => {
  (globalThis as typeof globalThis & { Deno?: unknown }).Deno = { core: { ops: {
    op_afterglow_worker_ids: () => [],
    async op_afterglow_rpc_call_async() { return new Uint8Array(); },
  } } };
  expect(() => platformTextureWorkerCount(16)).toThrow('manifest is empty');
  await expect(createPlatformTextureTranscoder(0, 'scene.big')).rejects.toThrow('index');
  await expect(createPlatformMeshOptimizer()).rejects.toThrow('exactly one');
});

test('public web retains the bounded two-to-four-worker profile', () => {
  const count = platformTextureWorkerCount(16);
  expect(count).toBeGreaterThanOrEqual(2);
  expect(count).toBeLessThanOrEqual(4);
  expect(platformTextureWorkerCount(1)).toBe(1);
});
