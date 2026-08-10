import { describe, expect, test } from 'bun:test';
import { decodeZip, encodeStoredZip, text, utf8 } from './openraster.ts';

describe('OpenRaster ZIP codec', () => {
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
