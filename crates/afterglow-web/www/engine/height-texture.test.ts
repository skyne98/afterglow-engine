import { describe, expect, test } from 'bun:test';
import { assertHeightTextureGpuFormat, assertHeightTextureSupport, loadHeightTextureR16, parseHeightR16 } from './height-texture.ts';

function encode(width: number, height: number, pixels: readonly number[]): ArrayBuffer {
  const buffer = new ArrayBuffer(16 + pixels.length * 2);
  const bytes = new Uint8Array(buffer);
  bytes.set([0x41, 0x47, 0x52, 0x31, 0x36, 0x4c, 0x45, 0x01]);
  const view = new DataView(buffer);
  view.setUint32(8, width, true);
  view.setUint32(12, height, true);
  for (let index = 0; index < pixels.length; index++) view.setUint16(16 + index * 2, pixels[index], true);
  return buffer;
}

class FakeDataTexture {
  image: { data: Float32Array; width: number; height: number };
  format: number;
  type: number;
  wrapS = 0;
  wrapT = 0;
  minFilter = 0;
  magFilter = 0;
  generateMipmaps = true;
  flipY = true;
  colorSpace = 'unexpected';
  unpackAlignment = 4;
  needsUpdate = false;
  name = '';

  constructor(data: Float32Array, width: number, height: number, format: number, type: number) {
    this.image = { data, width, height };
    this.format = format;
    this.type = type;
  }
}

const three = {
  DataTexture: FakeDataTexture,
  RedFormat: 1028,
  FloatType: 1015,
  RepeatWrapping: 1000,
  LinearFilter: 1006,
  NoColorSpace: '',
};
const supportedDevice = { features: new Set(['float32-filterable']) };

describe('R16 displacement assets', () => {
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

  test('fails closed without filterable float32 texture support', () => {
    expect(() => assertHeightTextureSupport({ features: new Set() })).toThrow('float32-filterable');
    expect(() => assertHeightTextureSupport(supportedDevice)).not.toThrow();
  });

  test('asserts the actual post-warm-up Three GPU format', () => {
    const texture = new FakeDataTexture(new Float32Array(1), 1, 1, three.RedFormat, three.FloatType);
    expect(() => assertHeightTextureGpuFormat({ utils: { getTextureFormatGPU: () => 'r32float' } }, texture)).not.toThrow();
    expect(() => assertHeightTextureGpuFormat({ utils: { getTextureFormatGPU: () => 'r16unorm' } }, texture)).toThrow('expected r32float');
    expect(() => assertHeightTextureGpuFormat({}, texture)).toThrow('unavailable');
  });

  test('constructs a single-channel filterable float texture without losing u16 levels', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => new Response(encode(2, 1, [1, 65535]))) as typeof fetch;
    try {
      const texture = await loadHeightTextureR16(three, supportedDevice, 'height.r16');
      expect(texture.format).toBe(three.RedFormat);
      expect(texture.type).toBe(three.FloatType);
      expect(texture.image.data).toBeInstanceOf(Float32Array);
      expect([...texture.image.data].map(value => Math.round(value * 65535))).toEqual([1, 65535]);
      expect(texture.image.width).toBe(2);
      expect(texture.image.height).toBe(1);
      expect(texture.minFilter).toBe(three.LinearFilter);
      expect(texture.magFilter).toBe(three.LinearFilter);
      expect(texture.generateMipmaps).toBe(false);
      expect(texture.flipY).toBe(false);
      expect(texture.unpackAlignment).toBe(4);
      expect(texture.needsUpdate).toBe(true);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
