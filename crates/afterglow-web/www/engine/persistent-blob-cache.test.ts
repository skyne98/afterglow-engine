import { describe, expect, test } from 'bun:test';
import {
  PersistentBlobCache,
  persistentCacheNamespace,
  type PersistentBlobBackend,
} from './persistent-blob-cache.ts';

class MemoryBackend implements PersistentBlobBackend {
  readonly files = new Map<string, Uint8Array>();
  failNextIndexAppend = false;

  async size(name: string): Promise<number> { return this.files.get(name)?.byteLength ?? 0; }
  async read(name: string, offset: number, length: number): Promise<Uint8Array> {
    return (this.files.get(name) ?? new Uint8Array()).slice(offset, offset + length);
  }
  async append(name: string, data: Uint8Array): Promise<void> {
    if (name.endsWith('.index') && this.failNextIndexAppend) {
      this.failNextIndexAppend = false;
      throw new Error('injected index append failure');
    }
    const previous = this.files.get(name) ?? new Uint8Array();
    const next = new Uint8Array(previous.byteLength + data.byteLength);
    next.set(previous); next.set(data, previous.byteLength);
    this.files.set(name, next);
  }
  async replace(name: string, data: Uint8Array): Promise<void> { this.files.set(name, data.slice()); }
}

const open = (backend: MemoryBackend, maxBytes = 1024, maxEntries = 16) =>
  PersistentBlobCache.open({ namespace: 'test', maxBytes, maxEntries, writeQueueCapacity: 4 }, backend);

describe('PersistentBlobCache', () => {
  test('persists generic blobs across cache instances', async () => {
    const backend = new MemoryBackend();
    const first = await open(backend);
    expect(await first.get('missing')).toBeNull();
    expect(await first.put('mesh:lod0', new Uint8Array([1, 2, 3]))).toBe(true);
    expect(await first.get('mesh:lod0')).toEqual(new Uint8Array([1, 2, 3]));

    const reopened = await open(backend);
    expect(await reopened.get('mesh:lod0')).toEqual(new Uint8Array([1, 2, 3]));
    expect(reopened.getStats()).toMatchObject({ entries: 1, bytes: 3, hits: 1 });
  });

  test('publishes index only after payload and ignores orphan pack suffixes', async () => {
    const backend = new MemoryBackend();
    const cache = await open(backend);
    await cache.put('valid', new Uint8Array([7, 8]));
    await backend.append('values-0.pack', new Uint8Array([99, 100, 101]));

    const reopened = await open(backend);
    expect(await reopened.get('valid')).toEqual(new Uint8Array([7, 8]));
    expect(await reopened.get('orphan')).toBeNull();
  });

  test('resynchronizes pack offsets after index publication failure', async () => {
    const backend = new MemoryBackend();
    const cache = await open(backend);
    backend.failNextIndexAppend = true;
    expect(await cache.put('orphan', new Uint8Array([1, 2]))).toBe(false);
    expect(await cache.put('next', new Uint8Array([3, 4]))).toBe(true);
    const reopened = await open(backend);
    expect(await reopened.get('orphan')).toBeNull();
    expect(await reopened.get('next')).toEqual(new Uint8Array([3, 4]));
  });

  test('treats checksum corruption as a miss', async () => {
    const backend = new MemoryBackend();
    const cache = await open(backend);
    await cache.put('page', new Uint8Array([4, 5, 6]));
    backend.files.get('values-0.pack')![1] ^= 0xff;
    expect(await cache.get('page')).toBeNull();
    expect(cache.getStats().corruptEntries).toBe(1);
  });

  test('evicts least-recently-used values and compacts at hard limits', async () => {
    const backend = new MemoryBackend();
    const cache = await open(backend, 5, 2);
    expect(await cache.put('a', new Uint8Array([1, 2, 3]))).toBe(true);
    expect(await cache.put('b', new Uint8Array([4, 5]))).toBe(true);
    expect(await cache.get('a')).toEqual(new Uint8Array([1, 2, 3])); // a becomes MRU
    expect(await cache.put('c', new Uint8Array([6, 7]))).toBe(true);
    expect(await cache.get('a')).toEqual(new Uint8Array([1, 2, 3]));
    expect(await cache.get('b')).toBeNull();
    expect(await cache.get('c')).toEqual(new Uint8Array([6, 7]));
    expect(cache.getStats()).toMatchObject({
      entries: 2, bytes: 5, liveBytes: 5, evictions: 1, compactions: 1, reclaimedBytes: 2,
    });
    const reopened = await open(backend, 5, 2);
    expect(await reopened.get('a')).toEqual(new Uint8Array([1, 2, 3]));
    expect(await reopened.get('b')).toBeNull();
    expect(await reopened.get('c')).toEqual(new Uint8Array([6, 7]));
  });

  test('keeps the active generation valid when compaction publication fails', async () => {
    const backend = new MemoryBackend();
    const cache = await open(backend, 5, 2);
    await cache.put('a', new Uint8Array([1, 2, 3]));
    await cache.put('b', new Uint8Array([4, 5]));
    backend.failNextIndexAppend = true;
    expect(await cache.put('c', new Uint8Array([6, 7]))).toBe(false);
    expect(await cache.get('a')).toEqual(new Uint8Array([1, 2, 3]));
    expect(await cache.get('b')).toEqual(new Uint8Array([4, 5]));
    const reopened = await open(backend, 5, 2);
    expect(await reopened.get('a')).toEqual(new Uint8Array([1, 2, 3]));
    expect(await reopened.get('b')).toEqual(new Uint8Array([4, 5]));
    expect(await reopened.get('c')).toBeNull();
  });

  test('rejects a single value larger than the hard byte capacity', async () => {
    const cache = await open(new MemoryBackend(), 2, 2);
    expect(await cache.put('large', new Uint8Array([1, 2, 3]))).toBe(false);
    expect(cache.getStats().rejectedCapacity).toBe(1);
  });

  test('namespaces length-prefixed identity parts', async () => {
    expect(await persistentCacheNamespace(['ab', 'c'])).not.toBe(
      await persistentCacheNamespace(['a', 'bc']),
    );
    expect(await persistentCacheNamespace(['same'])).toBe(await persistentCacheNamespace(['same']));
  });
});
