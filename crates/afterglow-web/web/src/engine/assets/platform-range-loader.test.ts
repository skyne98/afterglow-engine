import { afterEach, expect, test } from 'bun:test';
import { createPlatformRangeLoader } from './platform-range-loader.ts';
import {
  EngineMetric, ENGINE_METRIC_DESCRIPTORS, ENGINE_TRACE_DESCRIPTORS,
} from '../telemetry/catalog.ts';
import { EngineTelemetry } from '../telemetry/telemetry.ts';

afterEach(() => { delete (globalThis as typeof globalThis & { Deno?: unknown }).Deno; });

test('native JS-visible reads use bounded RPC-owned byte responses', async () => {
  const source = new Uint8Array([10, 11, 12, 13, 20, 21, 22]);
  const reads: Array<[string, bigint, number]> = [];
  (globalThis as typeof globalThis & { Deno?: unknown }).Deno = { core: { ops: {
    op_native_asset_size: async () => 727,
    op_native_asset_read_copy: async (path: string, offset: bigint, length: number) => {
      reads.push([path, offset, length]);
      return source.slice(Number(offset), Number(offset) + length);
    },
  } } };

  const telemetry = new EngineTelemetry(
    ENGINE_TRACE_DESCRIPTORS,
    ENGINE_METRIC_DESCRIPTORS,
    new ArrayBuffer(40 * 32),
    new Float64Array(256),
    () => 1,
  );
  telemetry.trace.arm(4);
  const loader = createPlatformRangeLoader('', telemetry);
  expect((await loader.identity('dungeon.big')).size).toBe(727);
  expect(await loader.read('dungeon.big', 0, 4)).toEqual(new Uint8Array([10, 11, 12, 13]));
  expect(await loader.readBulk!('dungeon.big', [
    { offset: 4, length: 2 },
    { offset: 6, length: 1 },
  ])).toEqual([new Uint8Array([20, 21]), new Uint8Array([22])]);
  expect(reads).toEqual([
    ['dungeon.big', 0n, 4],
    ['dungeon.big', 4n, 2],
    ['dungeon.big', 6n, 1],
  ]);
  expect(telemetry.metrics.readCell(EngineMetric.AssetBytesRead)).toBe(7);
  telemetry.trace.stop();
  expect(telemetry.trace.snapshot()?.count).toBe(6);
});

test('large native reads are split below the RPC payload ceiling', async () => {
  const calls: Array<[bigint, number]> = [];
  (globalThis as typeof globalThis & { Deno?: unknown }).Deno = { core: { ops: {
    op_native_asset_size: async () => 600_000,
    op_native_asset_read_copy: async (_path: string, offset: bigint, length: number) => {
      calls.push([offset, length]);
      return new Uint8Array(length).fill(calls.length);
    },
  } } };
  const bytes = await createPlatformRangeLoader().read('model.glb', 7, 600_000);
  expect(bytes.byteLength).toBe(600_000);
  expect(calls).toEqual([[7n, 524_288], [524_295n, 75_712]]);
  expect(bytes[0]).toBe(1);
  expect(bytes[524_288]).toBe(2);
});
