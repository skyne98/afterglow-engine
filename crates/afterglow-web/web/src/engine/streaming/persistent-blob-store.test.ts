import { describe, expect, test } from 'bun:test';
import {
  MemoryPersistentBlobBackend,
  PersistentBlobStatus,
  PersistentBlobStore,
  type PersistentBlobBackend,
  type PersistentBlobEntry,
} from './persistent-blob-store.ts';

const capacities = {
  maxItems: 2,
  maxBytes: 8,
  maxValueBytes: 6,
  maxInFlightOperations: 1,
  maxInFlightBytes: 6,
};

describe('PersistentBlobStore', () => {
  test('enforces item/value/byte capacities and atomically replaces values', async () => {
    const store = await PersistentBlobStore.open(new MemoryPersistentBlobBackend(), capacities);
    expect((await store.putAtomic('a', new Uint8Array([1, 2, 3]))).status)
      .toBe(PersistentBlobStatus.Ok);
    expect((await store.putAtomic('b', new Uint8Array([4, 5, 6, 7, 8]))).status)
      .toBe(PersistentBlobStatus.Ok);
    expect((await store.putAtomic('c', new Uint8Array([1]))).status)
      .toBe(PersistentBlobStatus.ItemCapacityExceeded);
    expect((await store.putAtomic('a', new Uint8Array(7))).status)
      .toBe(PersistentBlobStatus.ValueCapacityExceeded);
    expect((await store.putAtomic('a', new Uint8Array([9, 9, 9, 9]))).status)
      .toBe(PersistentBlobStatus.ByteCapacityExceeded);
    const read = await store.get('a', 6);
    expect(read.status).toBe(PersistentBlobStatus.Ok);
    expect(Array.from(read.bytes ?? [])).toEqual([1, 2, 3]);
    expect(store.getStats()).toBe(store.getStats());
    expect(store.getStats().items).toBe(2);
    expect(store.getStats().bytes).toBe(8);
  });

  test('rejects a concurrent operation and leaves the admitted operation valid', async () => {
    let release!: () => void;
    const blocked = new Promise<void>(resolve => { release = resolve; });
    const backend: PersistentBlobBackend = {
      async list(): Promise<readonly PersistentBlobEntry[]> {
        return [{ key: 'a', size: 1 }, { key: 'b', size: 1 }];
      },
      async read(): Promise<Uint8Array | null> { await blocked; return new Uint8Array([5]); },
      async writeAtomic(): Promise<void> {},
      async remove(): Promise<boolean> { return false; },
      async clear(): Promise<void> {},
      async close(): Promise<void> {},
    };
    const store = await PersistentBlobStore.open(backend, capacities);
    const first = store.get('a', 1);
    expect((await store.get('b', 1)).status).toBe(PersistentBlobStatus.OperationCapacityExceeded);
    release();
    expect((await first).status).toBe(PersistentBlobStatus.Ok);
    expect(store.getStats().inFlightOperations).toBe(0);
  });

  test('rejects corrupt persisted indexes before publication', async () => {
    const backend: PersistentBlobBackend = {
      async list() { return [{ key: '../bad', size: 1 }]; },
      async read() { return null; }, async writeAtomic() {},
      async remove() { return false; }, async clear() {}, async close() {},
    };
    await expect(PersistentBlobStore.open(backend, capacities)).rejects.toThrow('corrupt index');
  });
});
