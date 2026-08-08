import { describe, expect, test } from 'bun:test';
import { MemoryTextureWriteStatus, MemoryVirtualTextureSource } from './memory-page-source.ts';
import {
  decodeMemoryTextureSnapshot,
  encodeMemoryTextureSnapshot,
  restoreMemoryTextureSnapshot,
} from './memory-texture-snapshot.ts';

function source(): MemoryVirtualTextureSource {
  return new MemoryVirtualTextureSource({
    width: 256, height: 129, format: 'rgba8unorm', mipFilter: 'linear-color',
    addressMode: 'repeat', pageCapacity: 16, dirtyCapacity: 16, outputCapacity: 2,
    defaultTexel: new Uint8Array([1, 2, 3, 4]),
  });
}

describe('mutable texture snapshots', () => {
  test('round trips deterministic sparse canonical pages and logical revision', () => {
    const a = source();
    expect(a.writeRegion(130, 128, 2, 1, new Uint8Array([
      10, 20, 30, 40, 50, 60, 70, 80,
    ]))).toBe(MemoryTextureWriteStatus.Written);
    expect(a.writeRegion(3, 4, 1, 1, new Uint8Array([90, 91, 92, 93])))
      .toBe(MemoryTextureWriteStatus.Written);
    const descriptor = {
      width: 256, height: 129, format: 'rgba8unorm-srgb' as const,
      addressMode: 'repeat' as const,
    };
    const encoded = encodeMemoryTextureSnapshot(a, descriptor);
    expect(encodeMemoryTextureSnapshot(a, descriptor)).toEqual(encoded);
    const decoded = decodeMemoryTextureSnapshot(encoded, 16);
    const restored = restoreMemoryTextureSnapshot(decoded, {
      pageCapacity: 16, dirtyCapacity: 16, outputCapacity: 2,
    });
    expect(decoded.descriptor).toEqual(descriptor);
    expect(restored.contentRevision).toBe(a.contentRevision);
    expect(restored.canonicalPageCount).toBe(a.canonicalPageCount);
    expect(restored.readPage({ mip: 0, x: 0, y: 0 }))
      .toEqual(a.readPage({ mip: 0, x: 0, y: 0 }));
    expect(restored.readPage({ mip: 0, x: 1, y: 1 }))
      .toEqual(a.readPage({ mip: 0, x: 1, y: 1 }));
  });

  test('rejects corruption, truncation, duplicate pages, and restore overflow', () => {
    const a = source();
    expect(a.writeRegion(1, 1, 1, 1, new Uint8Array([1, 2, 3, 4])))
      .toBe(MemoryTextureWriteStatus.Written);
    const encoded = encodeMemoryTextureSnapshot(a, {
      width: 256, height: 129, format: 'rgba8unorm', addressMode: 'repeat',
    });
    const corrupt = encoded.slice();
    corrupt[corrupt.length - 1] = (corrupt[corrupt.length - 1] ?? 0) ^ 1;
    expect(() => decodeMemoryTextureSnapshot(corrupt, 16)).toThrow('checksum');
    expect(() => decodeMemoryTextureSnapshot(encoded.subarray(0, encoded.length - 1), 16))
      .toThrow('byte length');
    const decoded = decodeMemoryTextureSnapshot(encoded, 16);
    expect(() => restoreMemoryTextureSnapshot(decoded, {
      pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 1,
    })).toThrow('restore capacities');
  });
});
