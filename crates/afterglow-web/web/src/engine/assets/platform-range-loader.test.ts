import { afterEach, expect, test } from 'bun:test';
import { createPlatformRangeLoader } from './platform-range-loader.ts';

afterEach(() => { delete (globalThis as typeof globalThis & { Deno?: unknown }).Deno; });

test('native source returns zero-copy arena views for single and scatter reads', async () => {
  const backing = new Uint8Array([10, 11, 12, 13, 20, 21, 22]);
  const handles: Array<{ words: number[]; bytes: Uint8Array }> = [
    { words: [2, 0, 4, 7], bytes: backing.subarray(0, 4) },
    { words: [2, 1, 7, 9, 4, 3], bytes: backing },
  ];
  let nextHandle = 0;
  const seenSpans: Uint8Array[] = [];
  (globalThis as typeof globalThis & { Deno?: unknown }).Deno = { core: { ops: {
    op_native_asset_size: async () => 727,
    op_native_asset_read_copy: async () => { throw new Error('copy path was not expected'); },
    op_native_asset_read_handle: async () => handles[nextHandle++]!.words,
    op_native_asset_read_many_handle: async (_path: string, spans: Uint8Array) => {
      seenSpans.push(spans.slice());
      return handles[nextHandle++]!.words;
    },
    op_afterglow_arena_view: (handle: { slot: number }) => handles[handle.slot]!.bytes,
  } } };

  const source = createPlatformRangeLoader();
  expect((await source.identity('dungeon.big')).size).toBe(727);
  const single = await source.read('dungeon.big', 5, 4);
  expect([...single]).toEqual([10, 11, 12, 13]);
  expect(single.buffer).toBe(backing.buffer);

  const parts = await source.readBulk!('dungeon.big', [
    { offset: 4, length: 4 },
    { offset: 20, length: 3 },
  ]);
  expect(parts.map(part => [...part])).toEqual([[10, 11, 12, 13], [20, 21, 22]]);
  expect(parts[0]!.buffer).toBe(backing.buffer);
  expect(parts[1]!.buffer).toBe(backing.buffer);
  expect(new DataView(seenSpans[0]!.buffer).getBigUint64(0, true)).toBe(4n);
});
