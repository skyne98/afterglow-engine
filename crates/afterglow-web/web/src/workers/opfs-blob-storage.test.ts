import { describe, expect, test } from 'bun:test';
import { OpfsBlobStorageService } from './opfs-blob-storage.ts';

class FakeFileHandle {
  constructor(private readonly owner: FakeDirectory, private readonly name: string) {}
  async getFile() {
    const bytes = this.owner.files.get(this.name);
    if (!bytes) throw new DOMException('missing', 'NotFoundError');
    return { size: bytes.byteLength, async arrayBuffer() { return bytes.slice().buffer; } };
  }
  async createWritable() {
    const chunks: Uint8Array[] = [];
    return {
      write: async (bytes: Uint8Array) => {
        if (this.owner.failWrite === this.name) throw new Error('injected write failure');
        chunks.push(bytes.slice());
      },
      close: async () => {
        let length = 0;
        for (const chunk of chunks) length += chunk.byteLength;
        const bytes = new Uint8Array(length);
        let offset = 0;
        for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
        this.owner.files.set(this.name, bytes);
      },
      abort: async () => {},
    };
  }
}

class FakeDirectory {
  readonly files = new Map<string, Uint8Array>();
  readonly directories = new Map<string, FakeDirectory>();
  failWrite: string | null = null;
  async getFileHandle(name: string, options?: { create?: boolean }) {
    if (!options?.create && !this.files.has(name)) throw new DOMException('missing', 'NotFoundError');
    return new FakeFileHandle(this, name);
  }
  async getDirectoryHandle(name: string, options?: { create?: boolean }) {
    let directory = this.directories.get(name);
    if (!directory && options?.create) {
      directory = new FakeDirectory();
      this.directories.set(name, directory);
    }
    if (!directory) throw new DOMException('missing', 'NotFoundError');
    return directory;
  }
  async removeEntry(name: string) {
    if (!this.files.delete(name) && !this.directories.delete(name))
      throw new DOMException('missing', 'NotFoundError');
  }
  async *entries(): AsyncIterableIterator<[string, unknown]> {
    for (const entry of this.files) yield entry;
    for (const entry of this.directories) yield entry;
  }
}

function checksum(bytes: Uint8Array): number {
  let crc = 0xffff_ffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb8_8320 : 0);
  }
  return (crc ^ 0xffff_ffff) >>> 0;
}

async function put(
  service: OpfsBlobStorageService, key: string, bytes: Uint8Array,
): Promise<void> {
  const transaction = await service.beginPut('game', key, bytes.byteLength, checksum(bytes), 64);
  await service.writeChunk(transaction, 0, bytes);
  await service.commitPut(transaction);
}

describe('OPFS RingBuffer worker service', () => {
  test('retains the prior generation when pointer publication fails', async () => {
    const root = new FakeDirectory();
    const service = new OpfsBlobStorageService({ async getDirectory() { return root; } });
    await put(service, 'paint', new Uint8Array([1, 2, 3]));
    const directory = root.directories.get('game')!;
    directory.failWrite = 'paint.ptr';
    await expect(put(service, 'paint', new Uint8Array([9, 8, 7]))).rejects.toThrow('injected');
    directory.failWrite = null;
    expect(Array.from(await service.read('game', 'paint', 0, 3, 64))).toEqual([1, 2, 3]);

    directory.files.set('paint.ptr', new Uint8Array([99]));
    expect(Array.from(await service.read('game', 'paint', 0, 3, 64))).toEqual([9, 8, 7]);
    expect(await service.list('game', 4, 64)).toEqual([{ key: 'paint', size: 3 }]);
  });

  test('bounds transactions and rejects stale chunk handles', async () => {
    const root = new FakeDirectory();
    const service = new OpfsBlobStorageService({ async getDirectory() { return root; } });
    const handles: number[] = [];
    for (let index = 0; index < 8; index++)
      handles.push(await service.beginPut('game', `key${index}`, 1, checksum(new Uint8Array([index])), 64));
    await expect(service.beginPut('game', 'overflow', 1, 0, 64)).rejects.toThrow('capacity');
    const first = handles[0]!;
    await service.abortPut(first);
    await expect(service.writeChunk(first, 0, new Uint8Array([0]))).rejects.toThrow('stale');
  });
});
