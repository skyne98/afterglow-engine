import { describe, expect, test } from 'bun:test';
import {
  BoundedTranscoderPool, createFetchRangeLoader, createPageDataProvider, findVTPageChunk,
  getVirtualTextureDimensions, type BigHeader,
} from './big-parser.ts';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe('browser range loader', () => {
  test('issues an exact Range request and rejects non-partial responses', async () => {
    const original = globalThis.fetch;
    let range = '';
    globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
      range = new Headers(init?.headers).get('range') ?? '';
      if (range === 'bytes=0-0') return new Response(new Uint8Array([1]), {
        status: 206, headers: { 'Content-Range': 'bytes 0-0/1234', ETag: '"build-7"' },
      });
      return new Response(new Uint8Array([1, 2, 3, 4]), { status: 206 });
    }) as typeof fetch;
    try {
      const loader = createFetchRangeLoader('afterglow://local/');
      expect(await loader.read('data.big', 10, 4)).toEqual(new Uint8Array([1, 2, 3, 4]));
      expect(range).toBe('bytes=10-13');
      expect(await loader.identity('data.big')).toEqual({ size: 1234, etag: '"build-7"', lastModified: null });
    } finally {
      globalThis.fetch = original;
    }
  });
});

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

  test('returns a persistent GPU-block cache hit without source read or transcode', async () => {
    const payload = new Uint8Array(34 * 34 * 16).fill(7);
    let reads = 0;
    let transcodes = 0;
    const loader = {
      async read() { reads++; throw new Error('cache hit must skip source'); },
      async load() { return new Uint8Array(); }, async size() { return 0; },
    };
    const cache = {
      async get(key: string) { expect(key).toBe('0:0:1:0'); return payload; },
      async put() { throw new Error('cache hit must not write'); },
      getStats() { return {
        entries: 1, bytes: payload.byteLength, queuedWrites: 0, hits: 1, misses: 0, writes: 0,
        rejectedCapacity: 0, rejectedQueue: 0, corruptEntries: 0, readErrors: 0, writeErrors: 0,
        averageReadMs: 0, maxReadMs: 0, averageWriteMs: 0, maxWriteMs: 0,
      }; },
    };
    const provider = createPageDataProvider(loader, header, [{
      async transcode() { transcodes++; return new Uint8Array(); },
    }], 0, cache as never);
    expect(await provider('terrain', { mip: 0, x: 1, y: 0 })).toBe(payload);
    expect(reads).toBe(0);
    expect(transcodes).toBe(0);
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
    const provider = createPageDataProvider(loader, header, [{
      async transcode() { throw new Error('raw pages do not transcode'); },
    }], 4);
    expect((await provider('terrain', { mip: 0, x: 1, y: 0 })).byteLength).toBe(20);
    expect((await provider('terrain', { mip: 1, x: 0, y: 0, tail: true })).byteLength).toBe(30);
    expect(reads).toEqual([
      ['terrain.big', 110, 20],
      ['terrain.big', 130, 30],
    ]);
  });
});

describe('BoundedTranscoderPool', () => {
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
    const queue = new BoundedTranscoderPool([worker], 2);
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

  test('dispatches concurrently across independent workers', async () => {
    const resolvers: Array<(value: Uint8Array) => void> = [];
    let active = 0;
    let maxActive = 0;
    const workers = [0, 1].map(() => ({
      transcode() {
        active++;
        maxActive = Math.max(maxActive, active);
        return new Promise<Uint8Array>(resolve => resolvers.push(value => {
          active--;
          resolve(value);
        }));
      },
    }));
    const pool = new BoundedTranscoderPool(workers, 4);
    const first = pool.submit(new Uint8Array([1]), 0);
    const second = pool.submit(new Uint8Array([2]), 0);
    expect(maxActive).toBe(2);
    resolvers.shift()!(new Uint8Array([1]));
    resolvers.shift()!(new Uint8Array([2]));
    expect(await first).toEqual(new Uint8Array([1]));
    expect(await second).toEqual(new Uint8Array([2]));
    expect(pool.getStats()).toMatchObject({ workerCount: 2, active: 0, queued: 0, completed: 2 });
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
    const queue = new BoundedTranscoderPool([worker], 2);
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
    const queue = new BoundedTranscoderPool([worker], 1);
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
