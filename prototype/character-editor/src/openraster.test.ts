import { describe, expect, test } from 'bun:test';
import { decodeZip, encodeStoredZip, text, utf8 } from './openraster.ts';

describe('OpenRaster ZIP codec', () => {
  test('rejects excess expanded sizes before allocation', async () => {
    const archive = encodeStoredZip([{ name: 'x', data: utf8('a') }]);
    const view = new DataView(archive.buffer);
    const central = view.getUint32(archive.length - 6, true);
    view.setUint32(central + 24, 256 * 1024 * 1024 + 1, true);
    await expect(decodeZip(archive.buffer)).rejects.toThrow('exceed 256 MiB');
  });
  test('rejects corrupted bytes and duplicate entry names', async () => {
    const archive = encodeStoredZip([{ name: 'x', data: utf8('a') }]);
    archive[31] ^= 1;
    await expect(decodeZip(archive.buffer)).rejects.toThrow('checksum');
    const duplicate = encodeStoredZip([{ name: 'x', data: utf8('a') }, { name: 'x', data: utf8('b') }]);
    await expect(decodeZip(duplicate.buffer)).rejects.toThrow('duplicate');
  });
  test('round-trips stored entries', async () => {
    const archive = encodeStoredZip([
      { name: 'mimetype', data: utf8('image/openraster') },
      { name: 'stack.xml', data: utf8('<image w="64" h="64" />') },
    ]);
    const entries = await decodeZip(archive.buffer);
    expect(text(entries.get('mimetype')!)).toBe('image/openraster');
    expect(text(entries.get('stack.xml')!)).toContain('w="64"');
  });
});
