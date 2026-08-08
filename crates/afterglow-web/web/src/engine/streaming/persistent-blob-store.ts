import {
  EngineMetric,
  EngineTelemetryCategory,
  EngineTraceDescriptor,
} from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

export interface PersistentBlobStoreCapacities {
  readonly maxItems: number;
  readonly maxBytes: number;
  readonly maxValueBytes: number;
  readonly maxInFlightOperations: number;
  readonly maxInFlightBytes: number;
}

export interface PersistentBlobEntry {
  readonly key: string;
  readonly size: number;
}

/** Policy-free persistence mechanism. Backends own crash-safe atomic replace. */
export interface PersistentBlobBackend {
  list(maxValueBytes: number): Promise<readonly PersistentBlobEntry[]>;
  read(key: string, maxBytes: number): Promise<Uint8Array | null>;
  writeAtomic(key: string, bytes: Uint8Array): Promise<void>;
  remove(key: string): Promise<boolean>;
  clear(): Promise<void>;
  close(): Promise<void>;
}

export const enum PersistentBlobStatus {
  Ok = 0,
  NotFound = 1,
  InvalidKey = 2,
  ItemCapacityExceeded = 3,
  ByteCapacityExceeded = 4,
  ValueCapacityExceeded = 5,
  OperationCapacityExceeded = 6,
  InFlightByteCapacityExceeded = 7,
  KeyBusy = 8,
  CorruptBackendIndex = 9,
  IoError = 10,
  Closed = 11,
}

export interface PersistentBlobReadResult {
  readonly status: PersistentBlobStatus;
  readonly bytes: Uint8Array | null;
}

export interface PersistentBlobWriteResult {
  readonly status: PersistentBlobStatus;
}

export interface PersistentBlobStoreStats {
  items: number;
  bytes: number;
  inFlightOperations: number;
  inFlightBytes: number;
  operationHighWater: number;
  inFlightByteHighWater: number;
  rejectedOperations: number;
  readOperations: number;
  writeOperations: number;
  removeOperations: number;
  ioErrors: number;
}

function validKey(key: string): boolean {
  return key.length > 0 && key.length <= 128 && /^[A-Za-z0-9._-]+$/.test(key);
}

/**
 * Generic bounded byte persistence. Texture/model/cache semantics remain in
 * consumers; this class only admits byte items and delegates atomic storage.
 */
export class PersistentBlobStore {
  private readonly sizes = new Map<string, number>();
  private readonly busyKeys = new Set<string>();
  private reservedBytes = 0;
  private reservedItems = 0;
  private closed = false;
  private readonly stats: PersistentBlobStoreStats = {
    items: 0, bytes: 0, inFlightOperations: 0, inFlightBytes: 0,
    operationHighWater: 0, inFlightByteHighWater: 0, rejectedOperations: 0,
    readOperations: 0, writeOperations: 0, removeOperations: 0, ioErrors: 0,
  };

  private constructor(
    private readonly backend: PersistentBlobBackend,
    private readonly capacities: Readonly<PersistentBlobStoreCapacities>,
    private readonly telemetry?: EngineTelemetry,
  ) {}

  static async open(
    backend: PersistentBlobBackend,
    capacities: Readonly<PersistentBlobStoreCapacities>,
    telemetry?: EngineTelemetry,
  ): Promise<PersistentBlobStore> {
    if (!Number.isInteger(capacities.maxItems) || capacities.maxItems < 1 ||
        !Number.isInteger(capacities.maxBytes) || capacities.maxBytes < 1 ||
        !Number.isInteger(capacities.maxValueBytes) || capacities.maxValueBytes < 1 ||
        capacities.maxValueBytes > capacities.maxBytes ||
        !Number.isInteger(capacities.maxInFlightOperations) || capacities.maxInFlightOperations < 1 ||
        !Number.isInteger(capacities.maxInFlightBytes) || capacities.maxInFlightBytes < 1)
      throw new RangeError('invalid persistent blob-store capacities');
    const store = new PersistentBlobStore(backend, capacities, telemetry);
    const entries = await backend.list(capacities.maxValueBytes);
    let bytes = 0;
    for (const entry of entries) {
      if (!validKey(entry.key) || !Number.isSafeInteger(entry.size) || entry.size < 0 ||
          entry.size > capacities.maxValueBytes || store.sizes.has(entry.key)) {
        await backend.close();
        throw new Error('persistent blob backend returned a corrupt index');
      }
      bytes += entry.size;
      if (store.sizes.size >= capacities.maxItems || bytes > capacities.maxBytes) {
        await backend.close();
        throw new Error('persistent blob backend exceeds configured capacity');
      }
      store.sizes.set(entry.key, entry.size);
    }
    store.refreshStats(bytes);
    return store;
  }

  private refreshStats(bytes?: number): void {
    this.stats.items = this.sizes.size;
    if (bytes !== undefined) this.stats.bytes = bytes;
  }

  private begin(key: string, bytes: number): PersistentBlobStatus {
    if (this.closed) return PersistentBlobStatus.Closed;
    if (!validKey(key)) return PersistentBlobStatus.InvalidKey;
    if (this.busyKeys.has(key)) return PersistentBlobStatus.KeyBusy;
    if (this.stats.inFlightOperations >= this.capacities.maxInFlightOperations) {
      this.stats.rejectedOperations++;
      return PersistentBlobStatus.OperationCapacityExceeded;
    }
    if (this.stats.inFlightBytes + bytes > this.capacities.maxInFlightBytes) {
      this.stats.rejectedOperations++;
      return PersistentBlobStatus.InFlightByteCapacityExceeded;
    }
    this.busyKeys.add(key);
    this.stats.inFlightOperations++;
    this.stats.inFlightBytes += bytes;
    if (this.stats.inFlightOperations > this.stats.operationHighWater)
      this.stats.operationHighWater = this.stats.inFlightOperations;
    if (this.stats.inFlightBytes > this.stats.inFlightByteHighWater)
      this.stats.inFlightByteHighWater = this.stats.inFlightBytes;
    return PersistentBlobStatus.Ok;
  }

  private end(key: string, bytes: number): void {
    this.busyKeys.delete(key);
    this.stats.inFlightOperations--;
    this.stats.inFlightBytes -= bytes;
  }

  async get(key: string, maxBytes: number): Promise<PersistentBlobReadResult> {
    if (!validKey(key)) return { status: PersistentBlobStatus.InvalidKey, bytes: null };
    if (!this.sizes.has(key)) return { status: PersistentBlobStatus.NotFound, bytes: null };
    if (!Number.isSafeInteger(maxBytes) || maxBytes < 0 || maxBytes > this.capacities.maxValueBytes)
      return { status: PersistentBlobStatus.ValueCapacityExceeded, bytes: null };
    const admitted = this.begin(key, maxBytes);
    if (admitted !== PersistentBlobStatus.Ok) return { status: admitted, bytes: null };
    const correlation = this.telemetry?.nextCorrelation(EngineTelemetryCategory.Storage) ?? 0;
    this.telemetry?.trace.asyncBegin(EngineTraceDescriptor.BlobRead, correlation, maxBytes, 0);
    try {
      const bytes = await this.backend.read(key, maxBytes);
      if (bytes === null) {
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.BlobRead, correlation, 0, 1);
        return { status: PersistentBlobStatus.NotFound, bytes: null };
      }
      if (bytes.byteLength > maxBytes || bytes.byteLength > this.capacities.maxValueBytes) {
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.BlobRead, correlation, bytes.byteLength, 2);
        return { status: PersistentBlobStatus.ValueCapacityExceeded, bytes: null };
      }
      this.stats.readOperations++;
      this.telemetry?.metrics.counterAdd(EngineMetric.BlobReadBytes, bytes.byteLength);
      this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.BlobRead, correlation, bytes.byteLength, 0);
      return { status: PersistentBlobStatus.Ok, bytes };
    } catch {
      this.stats.ioErrors++;
      this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.BlobRead, correlation, 0, 3);
      return { status: PersistentBlobStatus.IoError, bytes: null };
    } finally {
      this.end(key, maxBytes);
    }
  }

  async putAtomic(key: string, bytes: Uint8Array): Promise<PersistentBlobWriteResult> {
    if (bytes.byteLength > this.capacities.maxValueBytes)
      return { status: PersistentBlobStatus.ValueCapacityExceeded };
    const previous = this.sizes.get(key);
    const newItem = previous === undefined;
    const delta = Math.max(0, bytes.byteLength - (previous ?? 0));
    if (newItem && this.sizes.size + this.reservedItems >= this.capacities.maxItems)
      return { status: PersistentBlobStatus.ItemCapacityExceeded };
    if (this.stats.bytes + this.reservedBytes + delta > this.capacities.maxBytes)
      return { status: PersistentBlobStatus.ByteCapacityExceeded };
    const admitted = this.begin(key, bytes.byteLength);
    if (admitted !== PersistentBlobStatus.Ok) return { status: admitted };
    this.reservedBytes += delta;
    if (newItem) this.reservedItems++;
    const correlation = this.telemetry?.nextCorrelation(EngineTelemetryCategory.Storage) ?? 0;
    this.telemetry?.trace.asyncBegin(EngineTraceDescriptor.BlobWrite, correlation, bytes.byteLength, 0);
    try {
      await this.backend.writeAtomic(key, bytes);
      this.sizes.set(key, bytes.byteLength);
      this.stats.bytes += bytes.byteLength - (previous ?? 0);
      this.stats.writeOperations++;
      this.refreshStats();
      this.telemetry?.metrics.counterAdd(EngineMetric.BlobWriteBytes, bytes.byteLength);
      this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.BlobWrite, correlation, bytes.byteLength, 0);
      return { status: PersistentBlobStatus.Ok };
    } catch {
      this.stats.ioErrors++;
      this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.BlobWrite, correlation, 0, 1);
      return { status: PersistentBlobStatus.IoError };
    } finally {
      this.reservedBytes -= delta;
      if (newItem) this.reservedItems--;
      this.end(key, bytes.byteLength);
    }
  }

  async remove(key: string): Promise<PersistentBlobWriteResult> {
    if (!validKey(key)) return { status: PersistentBlobStatus.InvalidKey };
    if (!this.sizes.has(key)) return { status: PersistentBlobStatus.NotFound };
    const admitted = this.begin(key, 0);
    if (admitted !== PersistentBlobStatus.Ok) return { status: admitted };
    try {
      const removed = await this.backend.remove(key);
      if (!removed) return { status: PersistentBlobStatus.NotFound };
      const previous = this.sizes.get(key) ?? 0;
      this.sizes.delete(key);
      this.stats.bytes -= previous;
      this.stats.removeOperations++;
      this.refreshStats();
      return { status: PersistentBlobStatus.Ok };
    } catch {
      this.stats.ioErrors++;
      return { status: PersistentBlobStatus.IoError };
    } finally {
      this.end(key, 0);
    }
  }

  async clear(): Promise<PersistentBlobWriteResult> {
    if (this.closed) return { status: PersistentBlobStatus.Closed };
    if (this.stats.inFlightOperations !== 0)
      return { status: PersistentBlobStatus.OperationCapacityExceeded };
    try {
      await this.backend.clear();
      this.sizes.clear();
      this.stats.bytes = 0;
      this.refreshStats();
      return { status: PersistentBlobStatus.Ok };
    } catch {
      this.stats.ioErrors++;
      return { status: PersistentBlobStatus.IoError };
    }
  }

  getStats(): Readonly<PersistentBlobStoreStats> {
    return this.stats;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    await this.backend.close();
  }
}

/** Deterministic test/in-memory adapter; production policy uses platform backends. */
export class MemoryPersistentBlobBackend implements PersistentBlobBackend {
  private readonly values = new Map<string, Uint8Array>();
  private closed = false;

  async list(_maxValueBytes: number): Promise<readonly PersistentBlobEntry[]> {
    return Array.from(this.values, ([key, value]) => ({ key, size: value.byteLength }));
  }
  async read(key: string, maxBytes: number): Promise<Uint8Array | null> {
    const value = this.values.get(key);
    if (!value) return null;
    if (value.byteLength > maxBytes) throw new RangeError('stored value exceeds read capacity');
    return value.slice();
  }
  async writeAtomic(key: string, bytes: Uint8Array): Promise<void> {
    if (this.closed) throw new Error('backend is closed');
    this.values.set(key, bytes.slice());
  }
  async remove(key: string): Promise<boolean> { return this.values.delete(key); }
  async clear(): Promise<void> { this.values.clear(); }
  async close(): Promise<void> { this.closed = true; }
}
