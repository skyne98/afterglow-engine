// Generic bounded persistent byte cache.
//
// The public API knows nothing about textures. Values are arbitrary byte blobs;
// callers own namespace and key policy. OPFS stores one append-only pack plus a
// fixed-record index, avoiding one filesystem file per cached value.

const INDEX_MAGIC = 0x43424741; // "AGBC" LE
const MANIFEST_MAGIC = 0x4d424741; // "AGBM" LE
const INDEX_VERSION = 2;
const INDEX_HEADER_BYTES = 16;
const HASH_WORDS = 8; // complete SHA-256 key; collisions are cryptographically negligible
const INDEX_RECORD_BYTES = 48;
const EMPTY = 0;
const OCCUPIED = 1;
const TOMBSTONE = 2;

export interface PersistentBlobCacheOptions {
  namespace: string;
  maxBytes: number;
  maxEntries: number;
  writeQueueCapacity?: number;
}

export interface PersistentBlobCacheStats {
  enabled: boolean;
  backend: string;
  entries: number;
  bytes: number;
  liveBytes: number;
  maxBytes: number;
  maxEntries: number;
  queuedWrites: number;
  hits: number;
  misses: number;
  writes: number;
  writeBytes: number;
  rejectedCapacity: number;
  rejectedQueue: number;
  corruptEntries: number;
  readErrors: number;
  writeErrors: number;
  evictions: number;
  compactions: number;
  reclaimedBytes: number;
  maintenance: boolean;
  averageReadMs: number;
  maxReadMs: number;
  averageWriteMs: number;
  maxWriteMs: number;
}

export interface PersistentBlobBackend {
  readonly kind?: string;
  size(name: string): Promise<number>;
  read(name: string, offset: number, length: number): Promise<Uint8Array>;
  append(name: string, data: Uint8Array): Promise<void>;
  replace(name: string, data: Uint8Array): Promise<void>;
}

interface WriteJob {
  hash: Uint32Array;
  data: Uint8Array;
  resolve(value: boolean): void;
}

function checksum(bytes: Uint8Array): number {
  let value = 0x811c9dc5;
  for (let index = 0; index < bytes.length; index++) {
    value ^= bytes[index];
    value = Math.imul(value, 0x01000193);
  }
  return value >>> 0;
}

async function hashKey(key: string): Promise<Uint32Array> {
  const encoded = new TextEncoder().encode(key);
  const digest = await crypto.subtle.digest('SHA-256', encoded);
  return new Uint32Array(digest);
}

export async function persistentCacheNamespace(parts: readonly string[]): Promise<string> {
  // Length-prefixing prevents ambiguous concatenations such as ["ab","c"] and ["a","bc"].
  let value = '';
  for (const part of parts) value += `${part.length}:${part};`;
  const words = await hashKey(value);
  let output = '';
  for (let index = 0; index < words.length; index++)
    output += words[index].toString(16).padStart(8, '0');
  return output;
}

/** OPFS backend used by CEF and supporting browsers. */
export class OpfsBlobBackend implements PersistentBlobBackend {
  readonly kind = 'opfs';
  private constructor(private readonly directory: FileSystemDirectoryHandle) {}

  static async open(namespace: string): Promise<OpfsBlobBackend> {
    const storage = navigator.storage;
    if (!storage?.getDirectory) throw new Error('OPFS is unavailable');
    const root = await storage.getDirectory();
    const cacheRoot = await root.getDirectoryHandle('afterglow-cache', { create: true });
    const directory = await cacheRoot.getDirectoryHandle(namespace, { create: true });
    return new OpfsBlobBackend(directory);
  }

  private async file(name: string): Promise<FileSystemFileHandle> {
    return this.directory.getFileHandle(name, { create: true });
  }

  async size(name: string): Promise<number> {
    return (await (await this.file(name)).getFile()).size;
  }

  async read(name: string, offset: number, length: number): Promise<Uint8Array> {
    const file = await (await this.file(name)).getFile();
    return new Uint8Array(await file.slice(offset, offset + length).arrayBuffer());
  }

  async append(name: string, data: Uint8Array): Promise<void> {
    const handle = await this.file(name);
    const size = (await handle.getFile()).size;
    const writable = await handle.createWritable({ keepExistingData: true });
    try {
      await writable.seek(size);
      await writable.write(data as unknown as FileSystemWriteChunkType);
    } finally {
      await writable.close();
    }
  }

  async replace(name: string, data: Uint8Array): Promise<void> {
    const writable = await (await this.file(name)).createWritable();
    try {
      await writable.write(data as unknown as FileSystemWriteChunkType);
    } finally {
      await writable.close();
    }
  }
}

interface IndexedChunk {
  file: string;
  offset: number;
  data: Uint8Array;
}

function idbRequest<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('IndexedDB request failed'));
  });
}

function idbTransaction(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error ?? new Error('IndexedDB transaction aborted'));
    transaction.onerror = () => reject(transaction.error ?? new Error('IndexedDB transaction failed'));
  });
}

/** IndexedDB fallback for secure custom schemes where Chromium denies OPFS. */
export class IndexedDbBlobBackend implements PersistentBlobBackend {
  readonly kind = 'indexeddb';
  private constructor(private readonly database: IDBDatabase) {}

  static async open(namespace: string): Promise<IndexedDbBlobBackend> {
    if (!globalThis.indexedDB) throw new Error('IndexedDB is unavailable');
    const request = indexedDB.open(`afterglow-cache-${namespace}`, 1);
    request.onupgradeneeded = () => {
      const database = request.result;
      if (!database.objectStoreNames.contains('chunks'))
        database.createObjectStore('chunks', { keyPath: ['file', 'offset'] });
      if (!database.objectStoreNames.contains('meta'))
        database.createObjectStore('meta', { keyPath: 'file' });
    };
    return new IndexedDbBlobBackend(await idbRequest(request));
  }

  async size(name: string): Promise<number> {
    const transaction = this.database.transaction('meta', 'readonly');
    const result = await idbRequest(transaction.objectStore('meta').get(name)) as { file: string; size: number } | undefined;
    return result?.size ?? 0;
  }

  private async predecessor(name: string, offset: number): Promise<IndexedChunk | null> {
    const transaction = this.database.transaction('chunks', 'readonly');
    const range = IDBKeyRange.bound([name, 0], [name, offset]);
    const cursor = await idbRequest(transaction.objectStore('chunks').openCursor(range, 'prev'));
    return cursor ? cursor.value as IndexedChunk : null;
  }

  async read(name: string, offset: number, length: number): Promise<Uint8Array> {
    if (length === 0) return new Uint8Array(0);
    // Append-only cache values are normally read at their exact chunk boundary.
    // One direct key lookup avoids two cursor transactions in that common case.
    const exactTransaction = this.database.transaction('chunks', 'readonly');
    const exact = await idbRequest(exactTransaction.objectStore('chunks').get([name, offset])) as IndexedChunk | undefined;
    if (exact && exact.data.byteLength === length) return exact.data;
    const predecessor = await this.predecessor(name, offset);
    const firstOffset = predecessor && predecessor.offset + predecessor.data.byteLength > offset
      ? predecessor.offset : offset;
    const output = new Uint8Array(length);
    const end = offset + length;
    const transaction = this.database.transaction('chunks', 'readonly');
    const store = transaction.objectStore('chunks');
    const range = IDBKeyRange.bound([name, firstOffset], [name, end - 1]);
    await new Promise<void>((resolve, reject) => {
      const request = store.openCursor(range);
      request.onerror = () => reject(request.error ?? new Error('IndexedDB cursor failed'));
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor) { resolve(); return; }
        const chunk = cursor.value as IndexedChunk;
        const copyStart = Math.max(offset, chunk.offset);
        const copyEnd = Math.min(end, chunk.offset + chunk.data.byteLength);
        if (copyEnd > copyStart)
          output.set(chunk.data.subarray(copyStart - chunk.offset, copyEnd - chunk.offset), copyStart - offset);
        cursor.continue();
      };
    });
    return output;
  }

  async append(name: string, data: Uint8Array): Promise<void> {
    const transaction = this.database.transaction(['chunks', 'meta'], 'readwrite');
    const done = idbTransaction(transaction);
    const meta = transaction.objectStore('meta');
    const previous = await idbRequest(meta.get(name)) as { file: string; size: number } | undefined;
    const offset = previous?.size ?? 0;
    transaction.objectStore('chunks').put({ file: name, offset, data: data.slice() });
    meta.put({ file: name, size: offset + data.byteLength });
    await done;
  }

  async replace(name: string, data: Uint8Array): Promise<void> {
    const transaction = this.database.transaction(['chunks', 'meta'], 'readwrite');
    const done = idbTransaction(transaction);
    const chunks = transaction.objectStore('chunks');
    const range = IDBKeyRange.bound([name, 0], [name, Number.MAX_SAFE_INTEGER]);
    await new Promise<void>((resolve, reject) => {
      const request = chunks.openCursor(range);
      request.onerror = () => reject(request.error ?? new Error('IndexedDB cursor failed'));
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor) { resolve(); return; }
        cursor.delete();
        cursor.continue();
      };
    });
    if (data.byteLength !== 0) chunks.put({ file: name, offset: 0, data: data.slice() });
    transaction.objectStore('meta').put({ file: name, size: data.byteLength });
    await done;
  }
}

/**
 * Fixed-capacity generic persistent cache for arbitrary byte blobs.
 *
 * Reads are cache-first and writes are serialized through a bounded queue.
 * Payload bytes are appended before the corresponding index record, so a crash
 * can leave only unreachable pack suffix bytes, never a published partial value.
 */
export class PersistentBlobCache {
  private readonly states: Uint8Array;
  private readonly hashes: Uint32Array;
  private readonly offsets: Float64Array;
  private readonly lengths: Uint32Array;
  private readonly checksums: Uint32Array;
  private readonly lruPrevious: Int32Array;
  private readonly lruNext: Int32Array;
  private readonly compactionOffsets: Float64Array;
  private readonly jobs: (WriteJob | null)[];
  private readonly stats: PersistentBlobCacheStats;
  private head = 0;
  private tail = 0;
  private queued = 0;
  private queuedBytes = 0;
  private writingBytes = 0;
  private writing = false;
  private entries = 0;
  private packBytes = 0;
  private liveBytes = 0;
  private lruHead = -1;
  private lruTail = -1;
  private activeGeneration = 0;
  private maintenancePromise: Promise<void> | null = null;
  private readonly idleResolvers: Array<() => void> = [];
  private totalReadMs = 0;
  private maxReadMs = 0;
  private totalWriteMs = 0;
  private maxWriteMs = 0;

  private constructor(
    private readonly backend: PersistentBlobBackend,
    private readonly maxBytes: number,
    private readonly maxEntries: number,
    writeQueueCapacity: number,
  ) {
    this.states = new Uint8Array(maxEntries * 2);
    this.hashes = new Uint32Array(this.states.length * HASH_WORDS);
    this.offsets = new Float64Array(this.states.length);
    this.lengths = new Uint32Array(this.states.length);
    this.checksums = new Uint32Array(this.states.length);
    this.lruPrevious = new Int32Array(this.states.length);
    this.lruNext = new Int32Array(this.states.length);
    this.lruPrevious.fill(-1);
    this.lruNext.fill(-1);
    this.compactionOffsets = new Float64Array(this.states.length);
    this.jobs = new Array(writeQueueCapacity).fill(null);
    this.stats = {
      enabled: true, backend: backend.kind ?? 'custom',
      entries: 0, bytes: 0, liveBytes: 0, maxBytes, maxEntries, queuedWrites: 0,
      hits: 0, misses: 0, writes: 0, writeBytes: 0,
      rejectedCapacity: 0, rejectedQueue: 0, corruptEntries: 0,
      readErrors: 0, writeErrors: 0, evictions: 0, compactions: 0,
      reclaimedBytes: 0, maintenance: false,
      averageReadMs: 0, maxReadMs: 0, averageWriteMs: 0, maxWriteMs: 0,
    };
  }

  static async open(
    options: Readonly<PersistentBlobCacheOptions>,
    backend?: PersistentBlobBackend,
  ): Promise<PersistentBlobCache> {
    if (!options.namespace || !Number.isSafeInteger(options.maxBytes) || options.maxBytes < 1 ||
        !Number.isInteger(options.maxEntries) || options.maxEntries < 1)
      throw new RangeError('invalid persistent blob cache options');
    const queueCapacity = options.writeQueueCapacity ?? 64;
    if (!Number.isInteger(queueCapacity) || queueCapacity < 1)
      throw new RangeError('cache write queue capacity must be positive');
    let store = backend;
    if (!store) {
      try {
        store = await OpfsBlobBackend.open(options.namespace);
      } catch {
        store = await IndexedDbBlobBackend.open(options.namespace);
      }
    }
    const cache = new PersistentBlobCache(store, options.maxBytes, options.maxEntries, queueCapacity);
    await cache.loadIndex();
    return cache;
  }

  private hashStart(slot: number): number { return slot * HASH_WORDS; }

  private hashesEqual(slot: number, hash: Uint32Array): boolean {
    const start = this.hashStart(slot);
    for (let word = 0; word < HASH_WORDS; word++)
      if (this.hashes[start + word] !== hash[word]) return false;
    return true;
  }

  private hashSlot(hash: Uint32Array): number {
    let value = 0x9e3779b9;
    for (let word = 0; word < HASH_WORDS; word++)
      value = Math.imul(value ^ hash[word], 0x85ebca6b);
    return (value >>> 0) % this.states.length;
  }

  private find(hash: Uint32Array): number {
    let slot = this.hashSlot(hash);
    for (let probe = 0; probe < this.states.length; probe++) {
      const state = this.states[slot];
      if (state === EMPTY) return -1;
      if (state === OCCUPIED && this.hashesEqual(slot, hash)) return slot;
      slot = (slot + 1) % this.states.length;
    }
    return -1;
  }

  private insert(hash: Uint32Array, offset: number, length: number, valueChecksum: number): boolean {
    let slot = this.hashSlot(hash);
    let tombstone = -1;
    for (let probe = 0; probe < this.states.length; probe++) {
      const state = this.states[slot];
      if (state === OCCUPIED && this.hashesEqual(slot, hash)) {
        this.liveBytes += length - this.lengths[slot];
        this.offsets[slot] = offset;
        this.lengths[slot] = length;
        this.checksums[slot] = valueChecksum;
        this.touch(slot);
        return true;
      }
      if (state === TOMBSTONE && tombstone < 0) tombstone = slot;
      if (state === EMPTY) {
        const target = tombstone < 0 ? slot : tombstone;
        this.states[target] = OCCUPIED;
        this.hashes.set(hash, this.hashStart(target));
        this.offsets[target] = offset;
        this.lengths[target] = length;
        this.checksums[target] = valueChecksum;
        this.entries++;
        this.liveBytes += length;
        this.linkLruTail(target);
        return true;
      }
      slot = (slot + 1) % this.states.length;
    }
    return false;
  }

  private linkLruTail(slot: number): void {
    this.lruPrevious[slot] = this.lruTail;
    this.lruNext[slot] = -1;
    if (this.lruTail < 0) this.lruHead = slot;
    else this.lruNext[this.lruTail] = slot;
    this.lruTail = slot;
  }

  private unlinkLru(slot: number): void {
    const previous = this.lruPrevious[slot];
    const next = this.lruNext[slot];
    if (previous < 0) this.lruHead = next;
    else this.lruNext[previous] = next;
    if (next < 0) this.lruTail = previous;
    else this.lruPrevious[next] = previous;
    this.lruPrevious[slot] = -1;
    this.lruNext[slot] = -1;
  }

  private touch(slot: number): void {
    if (slot === this.lruTail) return;
    this.unlinkLru(slot);
    this.linkLruTail(slot);
  }

  private remove(slot: number): void {
    if (slot < 0 || this.states[slot] !== OCCUPIED) return;
    this.unlinkLru(slot);
    this.liveBytes -= this.lengths[slot];
    this.states[slot] = TOMBSTONE;
    this.entries--;
  }

  private record(hash: Uint32Array, offset: number, length: number, valueChecksum: number): Uint8Array {
    const bytes = new Uint8Array(INDEX_RECORD_BYTES);
    const view = new DataView(bytes.buffer);
    for (let word = 0; word < HASH_WORDS; word++) view.setUint32(word * 4, hash[word], true);
    view.setBigUint64(32, BigInt(offset), true);
    view.setUint32(40, length, true);
    view.setUint32(44, valueChecksum, true);
    return bytes;
  }

  private packName(generation = this.activeGeneration): string { return `values-${generation}.pack`; }
  private indexName(generation = this.activeGeneration): string { return `values-${generation}.index`; }

  private manifest(generation: number): Uint8Array {
    const bytes = new Uint8Array(8);
    const view = new DataView(bytes.buffer);
    view.setUint32(0, MANIFEST_MAGIC, true);
    view.setUint32(4, generation, true);
    return bytes;
  }

  private async loadIndex(): Promise<void> {
    const manifestSize = await this.backend.size('manifest');
    if (manifestSize >= 8) {
      const manifest = await this.backend.read('manifest', 0, 8);
      const view = new DataView(manifest.buffer, manifest.byteOffset, manifest.byteLength);
      if (view.getUint32(0, true) === MANIFEST_MAGIC) this.activeGeneration = view.getUint32(4, true) & 1;
    } else {
      await this.backend.replace('manifest', this.manifest(0));
    }
    this.packBytes = await this.backend.size(this.packName());
    let indexSize = await this.backend.size(this.indexName());
    if (this.packBytes > this.maxBytes) {
      await this.clear();
      return;
    }
    if (indexSize < INDEX_HEADER_BYTES) {
      await this.backend.replace(this.indexName(), this.indexHeader());
      return;
    }
    const bytes = await this.backend.read(this.indexName(), 0, indexSize);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (view.getUint32(0, true) !== INDEX_MAGIC || view.getUint32(4, true) !== INDEX_VERSION) {
      await this.clear();
      return;
    }
    indexSize = INDEX_HEADER_BYTES + Math.floor((indexSize - INDEX_HEADER_BYTES) / INDEX_RECORD_BYTES) * INDEX_RECORD_BYTES;
    const hash = new Uint32Array(HASH_WORDS);
    for (let offset = INDEX_HEADER_BYTES; offset < indexSize; offset += INDEX_RECORD_BYTES) {
      for (let word = 0; word < HASH_WORDS; word++) hash[word] = view.getUint32(offset + word * 4, true);
      const packOffset = Number(view.getBigUint64(offset + 32, true));
      const length = view.getUint32(offset + 40, true);
      const valueChecksum = view.getUint32(offset + 44, true);
      if (length === 0) {
        this.remove(this.find(hash));
      } else if (packOffset + length <= this.packBytes && this.entries < this.maxEntries) {
        this.insert(hash, packOffset, length, valueChecksum);
      }
    }
  }

  private indexHeader(): Uint8Array {
    const bytes = new Uint8Array(INDEX_HEADER_BYTES);
    const view = new DataView(bytes.buffer);
    view.setUint32(0, INDEX_MAGIC, true);
    view.setUint32(4, INDEX_VERSION, true);
    view.setUint32(8, INDEX_RECORD_BYTES, true);
    view.setUint32(12, HASH_WORDS, true);
    return bytes;
  }

  async get(key: string): Promise<Uint8Array | null> {
    const startedAt = performance.now();
    try {
      if (this.maintenancePromise) await this.maintenancePromise;
      const hash = await hashKey(key);
      const slot = this.find(hash);
      if (slot < 0) {
        this.stats.misses++;
        return null;
      }
      const bytes = await this.backend.read(this.packName(), this.offsets[slot], this.lengths[slot]);
      if (bytes.byteLength !== this.lengths[slot] || checksum(bytes) !== this.checksums[slot]) {
        this.remove(slot);
        this.stats.corruptEntries++;
        this.stats.misses++;
        return null;
      }
      this.touch(slot);
      this.stats.hits++;
      return bytes;
    } catch {
      this.stats.readErrors++;
      this.stats.misses++;
      return null;
    } finally {
      const elapsed = performance.now() - startedAt;
      this.totalReadMs += elapsed;
      this.maxReadMs = Math.max(this.maxReadMs, elapsed);
    }
  }

  async put(key: string, data: Uint8Array): Promise<boolean> {
    if (data.byteLength === 0 || data.byteLength > this.maxBytes) {
      this.stats.rejectedCapacity++;
      return false;
    }
    if (this.maintenancePromise) await this.maintenancePromise;
    const hash = await hashKey(key);
    if (this.find(hash) >= 0) return true;
    if (this.hasQueued(hash)) return true;
    if (this.entries + this.queued + (this.writing ? 1 : 0) >= this.maxEntries ||
        this.packBytes + this.queuedBytes + this.writingBytes + data.byteLength > this.maxBytes) {
      try {
        await this.ensureCapacity(data.byteLength);
      } catch {
        this.stats.writeErrors++;
        return false;
      }
      if (this.find(hash) >= 0) return true;
    }
    if (this.entries + this.queued + (this.writing ? 1 : 0) >= this.maxEntries ||
        this.packBytes + this.queuedBytes + this.writingBytes + data.byteLength > this.maxBytes) {
      this.stats.rejectedCapacity++;
      return false;
    }
    if (this.queued >= this.jobs.length) {
      this.stats.rejectedQueue++;
      return false;
    }
    return new Promise(resolve => {
      this.jobs[this.tail] = { hash, data, resolve };
      this.tail = (this.tail + 1) % this.jobs.length;
      this.queued++;
      this.queuedBytes += data.byteLength;
      this.pump();
    });
  }

  private hasQueued(hash: Uint32Array): boolean {
    for (let count = 0, index = this.head; count < this.queued; count++, index = (index + 1) % this.jobs.length) {
      const job = this.jobs[index];
      if (job) {
        let equal = true;
        for (let word = 0; word < HASH_WORDS; word++)
          if (job.hash[word] !== hash[word]) { equal = false; break; }
        if (equal) return true;
      }
    }
    return false;
  }

  private pump(): void {
    if (this.writing || this.queued === 0) return;
    const job = this.jobs[this.head]!;
    this.jobs[this.head] = null;
    this.head = (this.head + 1) % this.jobs.length;
    this.queued--;
    this.queuedBytes -= job.data.byteLength;
    this.writing = true;
    this.writingBytes = job.data.byteLength;
    void this.write(job);
  }

  private async write(job: WriteJob): Promise<void> {
    const startedAt = performance.now();
    let success = false;
    try {
      if (this.find(job.hash) >= 0) {
        success = true;
      } else if (this.entries < this.maxEntries && this.packBytes + job.data.byteLength <= this.maxBytes) {
        const offset = this.packBytes;
        const valueChecksum = checksum(job.data);
        await this.backend.append(this.packName(), job.data);
        await this.backend.append(this.indexName(), this.record(job.hash, offset, job.data.byteLength, valueChecksum));
        this.packBytes += job.data.byteLength;
        success = this.insert(job.hash, offset, job.data.byteLength, valueChecksum);
        if (success) {
          this.stats.writes++;
          this.stats.writeBytes += job.data.byteLength;
        }
      } else {
        this.stats.rejectedCapacity++;
      }
    } catch {
      this.stats.writeErrors++;
      // Payload append may have succeeded before index publication failed.
      // Resynchronize so the next append never publishes an orphan's offset.
      try { this.packBytes = await this.backend.size(this.packName()); } catch { /* retain last known bound */ }
    } finally {
      const elapsed = performance.now() - startedAt;
      this.totalWriteMs += elapsed;
      this.maxWriteMs = Math.max(this.maxWriteMs, elapsed);
      this.writing = false;
      this.writingBytes = 0;
      job.resolve(success);
      this.pump();
      this.resolveIdle();
    }
  }

  private waitForIdle(): Promise<void> {
    if (!this.writing && this.queued === 0) return Promise.resolve();
    return new Promise(resolve => this.idleResolvers.push(resolve));
  }

  private resolveIdle(): void {
    if (this.writing || this.queued !== 0) return;
    while (this.idleResolvers.length !== 0) this.idleResolvers.pop()!();
  }

  private resetIndexState(): void {
    this.states.fill(EMPTY);
    this.lruPrevious.fill(-1);
    this.lruNext.fill(-1);
    this.entries = 0;
    this.liveBytes = 0;
    this.lruHead = -1;
    this.lruTail = -1;
  }

  private async ensureCapacity(incomingBytes: number): Promise<void> {
    if (this.maintenancePromise) {
      await this.maintenancePromise;
      return;
    }
    const maintenance = this.compact(incomingBytes).finally(() => {
      if (this.maintenancePromise === maintenance) this.maintenancePromise = null;
    });
    this.maintenancePromise = maintenance;
    await maintenance;
  }

  private async compact(incomingBytes: number): Promise<void> {
    await this.waitForIdle();
    const oldGeneration = this.activeGeneration;
    const nextGeneration = oldGeneration ^ 1;
    const oldPackBytes = this.packBytes;
    const targetBytes = Math.min(
      Math.floor(this.maxBytes * 0.75),
      Math.max(0, this.maxBytes - incomingBytes),
    );
    const targetEntries = Math.min(
      Math.floor(this.maxEntries * 0.75),
      Math.max(0, this.maxEntries - 1),
    );
    let evicted = 0;
    while (this.lruHead >= 0 && (this.liveBytes > targetBytes || this.entries > targetEntries)) {
      this.remove(this.lruHead);
      evicted++;
    }

    let published = false;
    try {
      await this.backend.replace(this.packName(nextGeneration), new Uint8Array(0));
      await this.backend.replace(this.indexName(nextGeneration), this.indexHeader());
      let nextOffset = 0;
      let slot = this.lruHead;
      while (slot >= 0) {
        const following = this.lruNext[slot];
        const bytes = await this.backend.read(this.packName(oldGeneration), this.offsets[slot], this.lengths[slot]);
        if (bytes.byteLength !== this.lengths[slot] || checksum(bytes) !== this.checksums[slot]) {
          this.remove(slot);
          this.stats.corruptEntries++;
        } else {
          const hash = this.hashes.subarray(this.hashStart(slot), this.hashStart(slot) + HASH_WORDS);
          await this.backend.append(this.packName(nextGeneration), bytes);
          await this.backend.append(
            this.indexName(nextGeneration),
            this.record(hash, nextOffset, bytes.byteLength, this.checksums[slot]),
          );
          this.compactionOffsets[slot] = nextOffset;
          nextOffset += bytes.byteLength;
        }
        slot = following;
      }
      await this.backend.replace('manifest', this.manifest(nextGeneration));
      this.activeGeneration = nextGeneration;
      published = true;
      for (let index = 0; index < this.states.length; index++)
        if (this.states[index] === OCCUPIED) this.offsets[index] = this.compactionOffsets[index];
      this.packBytes = nextOffset;
      this.stats.evictions += evicted;
      this.stats.compactions++;
      this.stats.reclaimedBytes += Math.max(0, oldPackBytes - nextOffset);
      // Old generation is unreachable after manifest publication. Cleanup is
      // best-effort and cannot invalidate the newly active cache.
      try {
        await this.backend.replace(this.packName(oldGeneration), new Uint8Array(0));
        await this.backend.replace(this.indexName(oldGeneration), this.indexHeader());
      } catch { /* quota cleanup will retry on a later compaction */ }
    } catch (error) {
      if (!published) {
        this.activeGeneration = oldGeneration;
        this.resetIndexState();
        await this.loadIndex();
      }
      throw error;
    }
  }

  async clear(): Promise<void> {
    if (this.writing || this.queued !== 0 || this.maintenancePromise)
      throw new Error('cannot clear persistent cache while writes or maintenance are pending');
    await this.backend.replace(this.packName(0), new Uint8Array(0));
    await this.backend.replace(this.indexName(0), this.indexHeader());
    await this.backend.replace(this.packName(1), new Uint8Array(0));
    await this.backend.replace(this.indexName(1), this.indexHeader());
    await this.backend.replace('manifest', this.manifest(0));
    this.activeGeneration = 0;
    this.resetIndexState();
    this.packBytes = 0;
  }

  getStats(): Readonly<PersistentBlobCacheStats> {
    const stats = this.stats;
    stats.entries = this.entries;
    stats.bytes = this.packBytes;
    stats.liveBytes = this.liveBytes;
    stats.queuedWrites = this.queued + (this.writing ? 1 : 0);
    stats.maintenance = this.maintenancePromise !== null;
    const reads = stats.hits + stats.misses;
    stats.averageReadMs = reads === 0 ? 0 : this.totalReadMs / reads;
    stats.maxReadMs = this.maxReadMs;
    const writes = stats.writes + stats.writeErrors;
    stats.averageWriteMs = writes === 0 ? 0 : this.totalWriteMs / writes;
    stats.maxWriteMs = this.maxWriteMs;
    return stats;
  }
}
