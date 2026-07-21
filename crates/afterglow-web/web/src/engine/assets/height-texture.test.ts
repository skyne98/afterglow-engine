import { describe, expect, test } from 'bun:test';
import { parseHeightR16 } from './height-texture.ts';

function encode(width: number, height: number, pixels: readonly number[]): ArrayBuffer {
  const buffer = new ArrayBuffer(16 + pixels.length * 2);
  const bytes = new Uint8Array(buffer);
  bytes.set([0x41, 0x47, 0x52, 0x31, 0x36, 0x4c, 0x45, 0x01]);
  const view = new DataView(buffer);
  view.setUint32(8, width, true);
  view.setUint32(12, height, true);
  for (let index = 0; index < pixels.length; index++) view.setUint16(16 + index * 2, pixels[index] ?? 0, true);
  return buffer;
}

describe('R16 displacement interchange', () => {
  test('preserves every u16 sample without browser image decoding', () => {
    const asset = parseHeightR16(encode(7, 1, [0, 1, 255, 256, 32768, 65534, 65535]));
    expect(asset.width).toBe(7);
    expect(asset.height).toBe(1);
    expect([...asset.pixels]).toEqual([0, 1, 255, 256, 32768, 65534, 65535]);
  });

  test('rejects truncation, unknown versions, invalid dimensions, and trailing bytes', () => {
    expect(() => parseHeightR16(new ArrayBuffer(15))).toThrow('truncated');
    const unknown = encode(1, 1, [0]);
    new Uint8Array(unknown)[7] = 2;
    expect(() => parseHeightR16(unknown)).toThrow('magic/version');
    expect(() => parseHeightR16(encode(0, 1, []))).toThrow('non-zero');
    expect(() => parseHeightR16(encode(1, 1, [0, 1]))).toThrow('byte length');
  });
});
