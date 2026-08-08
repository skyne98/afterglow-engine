import { describe, expect, test } from 'bun:test';
import {
  findResidentTextureChunk,
  loadResidentTexture,
  residentTextureBytesPerTexel,
} from './resident-texture.ts';
import type { BigHeader, ChunkInfo, TextureFormat } from './big-format.ts';

class FakeDataTexture {
  image: { data: Uint8Array; width: number; height: number };
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
  dispose(): void {}

  constructor(data: Uint8Array, width: number, height: number, format: number, type: number) {
    this.image = { data, width, height };
    this.format = format;
    this.type = type;
  }
}

const three = {
  DataTexture: FakeDataTexture,
  RedFormat: 1028,
  RGBAFormat: 1021,
  UnsignedByteType: 1009,
  RepeatWrapping: 1000,
  LinearFilter: 1006,
  NoColorSpace: '',
};

function residentChunk(
  name: string,
  width: number,
  height: number,
  format: TextureFormat,
  bytes: Uint8Array,
  offset: number,
): { asset: BigHeader['assets'][number]; chunk: ChunkInfo } {
  const chunk: ChunkInfo = {
    offset: BigInt(offset),
    compressedSize: BigInt(bytes.length),
    uncompressedSize: BigInt(bytes.length),
    lodLevel: 0,
    mipLevel: 0,
    compression: 'None',
    meta: { type: 'Texture', width, height, format },
  };
  return { asset: { name, assetType: 'Texture', chunks: [chunk], virtualTexture: null }, chunk };
}

function header(assets: BigHeader['assets']): BigHeader {
  return { version: 6, dataOffset: 64n, assets };
}

describe('resident texture lookup', () => {
  test('bytes per texel matches format', () => {
    expect(residentTextureBytesPerTexel('R8')).toBe(1);
    expect(residentTextureBytesPerTexel('Rgba8')).toBe(4);
  });

  test('finds and validates an R8 resident texture chunk', () => {
    const { asset } = residentChunk('Rock_Height', 4, 4, 'R8', new Uint8Array(16), 100);
    const chunk = findResidentTextureChunk(header([asset]), 'Rock_Height');
    expect(chunk.meta.format).toBe('R8');
    expect(chunk.meta.width).toBe(4);
    expect(chunk.meta.height).toBe(4);
    expect(chunk.compression).toBe('None');
  });

  test('rejects compressed, missing, wrong-type, and byte-mismatch chunks', () => {
    const missing = header([]);
    expect(() => findResidentTextureChunk(missing, 'nope')).toThrow('not found');

    const compressed: BigHeader['assets'][number] = {
      name: 'c', assetType: 'Texture',
      chunks: [{
        offset: 0n, compressedSize: 10n, uncompressedSize: 10n, lodLevel: 0, mipLevel: 0,
        compression: 'Meshopt', meta: { type: 'Texture', width: 4, height: 4, format: 'R8' },
      }],
      virtualTexture: null,
    };
    expect(() => findResidentTextureChunk(header([compressed]), 'c')).toThrow('uncompressed');

    const mesh: BigHeader['assets'][number] = {
      name: 'm', assetType: 'Mesh',
      chunks: [{
        offset: 0n, compressedSize: 4n, uncompressedSize: 4n, lodLevel: 0, mipLevel: 0,
        compression: 'None', meta: { type: 'Raw' },
      }],
      virtualTexture: null,
    };
    expect(() => findResidentTextureChunk(header([mesh]), 'm')).toThrow('not a resident Texture');

    const badLen = residentChunk('bad', 4, 4, 'R8', new Uint8Array(15), 0).asset;
    expect(() => findResidentTextureChunk(header([badLen]), 'bad')).toThrow('byte length');
  });
});

describe('resident texture loading', () => {
  test('loads an R8 height into a RedFormat+UnsignedByte DataTexture', async () => {
    const payload = new Uint8Array([0, 64, 128, 255, 1, 2, 3, 4]);
    const { asset } = residentChunk('Rock_Height', 4, 2, 'R8', payload, 200);
    const reads: Array<{ offset: number; length: number }> = [];
    const source = {
      read: async (offset: number, length: number): Promise<Uint8Array> => {
        reads.push({ offset, length });
        return payload.slice();
      },
    };
    const result = await loadResidentTexture(three, source, header([asset]), 'Rock_Height');
    expect(result.format).toBe('R8');
    expect(result.width).toBe(4);
    expect(result.height).toBe(2);
    expect(result.texture.format).toBe(three.RedFormat);
    expect(result.texture.type).toBe(three.UnsignedByteType);
    expect([...result.texture.image.data]).toEqual([0, 64, 128, 255, 1, 2, 3, 4]);
    expect(result.texture.generateMipmaps).toBe(false);
    expect(result.texture.flipY).toBe(false);
    expect(result.texture.colorSpace).toBe(three.NoColorSpace);
    expect(result.texture.needsUpdate).toBe(true);
    expect(result.texture.unpackAlignment).toBe(1);
    expect(reads).toEqual([{ offset: 200, length: 8 }]);
  });

  test('loads an RGBA8 resident texture into RGBAFormat', async () => {
    const payload = new Uint8Array(2 * 2 * 4);
    for (let i = 0; i < payload.length; i++) payload[i] = i;
    const { asset } = residentChunk('albedo', 2, 2, 'Rgba8', payload, 0);
    const source = { read: async () => payload.slice() };
    const result = await loadResidentTexture(three, source, header([asset]), 'albedo');
    expect(result.texture.format).toBe(three.RGBAFormat);
    expect(result.texture.type).toBe(three.UnsignedByteType);
    expect([...result.texture.image.data]).toEqual([...payload]);
  });

  test('rejects a short read', async () => {
    const { asset } = residentChunk('h', 4, 1, 'R8', new Uint8Array(4), 0);
    const source = { read: async () => new Uint8Array(2) };
    await expect(loadResidentTexture(three, source, header([asset]), 'h')).rejects.toThrow('read 2 bytes');
  });
});
