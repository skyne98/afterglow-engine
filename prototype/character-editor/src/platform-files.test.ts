import { afterEach, expect, test } from 'bun:test';
import { openFile, saveFile } from '../../../crates/afterglow-web/web/src/engine/workers/platform-files.ts';

const original = (globalThis as any).Deno;
afterEach(() => { (globalThis as any).Deno = original; });

test('native file dialogs return bytes without a filesystem path', async () => {
  let filters: string[] = [];
  let saved: unknown;
  (globalThis as any).Deno = { core: { ops: {
    async op_open_file(extensions: string[]) { filters = extensions; return new Uint8Array([1, 2, 3]); },
    async op_save_file(name: string, bytes: Uint8Array) { saved = [name, [...bytes]]; return true; },
  } } };
  const file = await openFile(['ora']);
  expect(filters).toEqual(['ora']);
  expect([...new Uint8Array(await file!.arrayBuffer())]).toEqual([1, 2, 3]);
  expect(await saveFile(new Uint8Array([4]), 'paint.png', 'image/png')).toBe(true);
  expect(saved).toEqual(['paint.png', [4]]);
});

test('native cancellation is not a file error', async () => {
  (globalThis as any).Deno = { core: { ops: {
    async op_open_file() { return null; }, async op_save_file() { return false; },
  } } };
  expect(await openFile(['ora'])).toBeNull();
  expect(await saveFile(new Uint8Array(), 'paint.ora', 'image/openraster')).toBe(false);
});

test('native files never use a browser fallback when the host API is missing', async () => {
  (globalThis as any).Deno = { core: { ops: {} } };
  expect(openFile(['ora'])).rejects.toThrow('unavailable');
});
