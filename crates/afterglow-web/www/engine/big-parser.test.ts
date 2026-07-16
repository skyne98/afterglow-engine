import { describe, expect, test } from 'bun:test';
import {
  BoundedSerialTranscoder, createPageDataProvider, findVTPageChunk,
  getVirtualTextureDimensions, type BigHeader,
} from './big-parser.ts';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe('.big v5 compact VT directories', () => {
  const header: BigHeader = {
    version: 5, dataOffset: 64n,
    assets: [{
      name: 'terrain', assetType: 'VirtualTexture', chunks: [],
      virtualTexture: {
        width: 256, height: 128, encoding: 'RawRgba8',
        mips: [{ mip: 0, pagesX: 2, pagesY: 1, offset: 100n, pageSizes: [10, 20] }],
        tail: { firstMip: 1, offset: 130n, size: 30 },
      },
    }],
  };

  test('calculates direct page ranges from one mip offset plus compact sizes', () => {
    expect(getVirtualTextureDimensions(header, 'terrain')).toEqual({ width: 256, height: 128 });
    const second = findVTPageChunk(header, 'terrain', 0, 1, 0)!;
    expect(second.offset).toBe(110n);
    expect(second.compressedSize).toBe(20n);
    expect(findVTPageChunk(header, 'terrain', 0, 2, 0)).toBeNull();
  });

  test('expands direct typed-array offsets once for runtime range reads', async () => {
    const reads: Array<[string, number, number]> = [];
    const loader = {
      async read(path: string, offset: number, size: number) {
        reads.push([path, offset, size]);
        return new Uint8Array(size);
      },
      async load() { return new Uint8Array(); },
      async size() { return 0; },
    };
    const provider = createPageDataProvider(loader, header, {
      async transcode() { throw new Error('raw pages do not transcode'); },
    }, 4);
    expect((await provider('terrain', { mip: 0, x: 1, y: 0 })).byteLength).toBe(20);
    expect((await provider('terrain', { mip: 1, x: 0, y: 0, tail: true })).byteLength).toBe(30);
    expect(reads).toEqual([
      ['terrain.big', 110, 20],
      ['terrain.big', 130, 30],
    ]);
  });
});

describe('BoundedSerialTranscoder', () => {
  test('keeps one RPC in flight and rejects canceled queued work', async () => {
    const resolvers: Array<(value: Uint8Array) => void> = [];
    let active = 0;
    let maxActive = 0;
    let calls = 0;
    const worker = {
      transcode() {
        calls++;
        active++;
        maxActive = Math.max(maxActive, active);
        return new Promise<Uint8Array>(resolve => resolvers.push(value => {
          active--;
          resolve(value);
        }));
      },
    };
    const queue = new BoundedSerialTranscoder(worker, 2);
    const first = queue.submit(new Uint8Array([1]), 0);
    const controller = new AbortController();
    const second = queue.submit(new Uint8Array([2]), 0, controller.signal);
    controller.abort();

    expect(calls).toBe(1);
    resolvers.shift()!(new Uint8Array([9]));
    expect(await first).toEqual(new Uint8Array([9]));
    await expect(second).rejects.toThrow('canceled before dispatch');
    expect(calls).toBe(1);
    expect(maxActive).toBe(1);
  });

  test('owns reusable RPC scratch before dispatching the next call', async () => {
    const scratch = new Uint8Array(1);
    let call = 0;
    const worker = {
      async transcode() {
        scratch[0] = ++call;
        return scratch;
      },
    };
    const queue = new BoundedSerialTranscoder(worker, 2);
    const first = queue.submit(new Uint8Array(), 0);
    const second = queue.submit(new Uint8Array(), 0);
    expect((await first)[0]).toBe(1);
    expect((await second)[0]).toBe(2);
  });

  test('has a fixed waiting capacity', async () => {
    const resolvers: Array<(value: Uint8Array) => void> = [];
    const worker = {
      transcode() {
        return new Promise<Uint8Array>(resolve => resolvers.push(resolve));
      },
    };
    const queue = new BoundedSerialTranscoder(worker, 1);
    const running = queue.submit(new Uint8Array([1]), 0);
    const waiting = queue.submit(new Uint8Array([2]), 0);
    await expect(queue.submit(new Uint8Array([3]), 0)).rejects.toThrow('capacity exceeded');
    resolvers.shift()!(new Uint8Array([1]));
    await running;
    await flush();
    resolvers.shift()!(new Uint8Array([2]));
    await waiting;
  });
});
