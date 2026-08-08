import { BlobStorageClient } from '../../workers/blobstorage.client.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import type {
  PersistentBlobBackend,
  PersistentBlobEntry,
} from './persistent-blob-store.ts';

const RPC_CHUNK_BYTES = 512 * 1024;

function crc32(bytes: Uint8Array): number {
  let crc = 0xffff_ffff;
  for (let index = 0; index < bytes.byteLength; index++) {
    crc ^= bytes[index] ?? 0;
    for (let bit = 0; bit < 8; bit++)
      crc = (crc >>> 1) ^ ((crc & 1) !== 0 ? 0xedb8_8320 : 0);
  }
  return (crc ^ 0xffff_ffff) >>> 0;
}

/** Public-web generated-RPC adapter. OPFS is visible only to its Worker. */
export class WebPersistentBlobBackend implements PersistentBlobBackend {
  private closed = false;

  private constructor(
    private readonly namespace: string,
    private readonly maxItems: number,
    private readonly maxValueBytes: number,
    private readonly client: BlobStorageClient,
  ) {}

  static async open(
    namespace: string,
    maxItems: number,
    maxValueBytes: number,
    _telemetry?: EngineTelemetry,
  ): Promise<WebPersistentBlobBackend> {
    if (!/^[A-Za-z0-9._-]{1,64}$/.test(namespace) ||
        !Number.isInteger(maxItems) || maxItems < 1 ||
        !Number.isSafeInteger(maxValueBytes) || maxValueBytes < 1)
      throw new RangeError('invalid web blob backend configuration');
    const client = await BlobStorageClient.spawnThreaded({
      mainWasmUrl: 'afterglow_rpc.wasm',
      workerJsUrl: 'storage-worker.js',
      workerWasmUrl: '',
    });
    return new WebPersistentBlobBackend(namespace, maxItems, maxValueBytes, client);
  }

  private assertOpen(): void {
    if (this.closed) throw new Error('web blob backend is closed');
  }

  async list(maxValueBytes: number): Promise<readonly PersistentBlobEntry[]> {
    this.assertOpen();
    const encoded = await this.client.list(
      this.namespace, this.maxItems, Math.min(maxValueBytes, this.maxValueBytes),
    );
    if (encoded.byteLength < 4) throw new Error('web blob index is truncated');
    const view = new DataView(encoded.buffer, encoded.byteOffset, encoded.byteLength);
    const count = view.getUint32(0, true);
    if (count > this.maxItems) throw new Error('web blob index exceeds item capacity');
    const entries: PersistentBlobEntry[] = new Array(count);
    const decoder = new TextDecoder('utf-8', { fatal: true });
    let cursor = 4;
    for (let index = 0; index < count; index++) {
      if (cursor >= encoded.byteLength) throw new Error('web blob index is truncated');
      const keyLength = encoded[cursor++] ?? 0;
      if (cursor + keyLength + 8 > encoded.byteLength) throw new Error('web blob index is truncated');
      const key = decoder.decode(encoded.subarray(cursor, cursor + keyLength));
      cursor += keyLength;
      const size = Number(view.getBigUint64(cursor, true));
      cursor += 8;
      entries[index] = { key, size };
    }
    if (cursor !== encoded.byteLength) throw new Error('web blob index has trailing bytes');
    return entries;
  }

  async read(key: string, maxBytes: number): Promise<Uint8Array | null> {
    this.assertOpen();
    const size = await this.client.size(this.namespace, key, Math.min(maxBytes, this.maxValueBytes));
    if (size > maxBytes || size > this.maxValueBytes)
      throw new RangeError('web blob exceeds caller capacity');
    const output = new Uint8Array(size);
    let offset = 0;
    while (offset < size) {
      const length = Math.min(RPC_CHUNK_BYTES, size - offset);
      const chunk = await this.client.read(this.namespace, key, offset, length, this.maxValueBytes);
      if (chunk.byteLength !== length) throw new Error('web blob read returned a short chunk');
      output.set(chunk, offset);
      offset += length;
    }
    return output;
  }

  async writeAtomic(key: string, bytes: Uint8Array): Promise<void> {
    this.assertOpen();
    if (bytes.byteLength > this.maxValueBytes) throw new RangeError('web blob exceeds value capacity');
    const transaction = await this.client.beginPut(
      this.namespace, key, bytes.byteLength, crc32(bytes), this.maxValueBytes,
    );
    try {
      let offset = 0;
      while (offset < bytes.byteLength) {
        const length = Math.min(RPC_CHUNK_BYTES, bytes.byteLength - offset);
        const written = await this.client.writeChunk(
          transaction, offset, bytes.subarray(offset, offset + length),
        );
        if (written !== length) throw new Error('web blob write returned a short chunk');
        offset += length;
      }
      if (!await this.client.commitPut(transaction)) throw new Error('web blob commit was rejected');
    } catch (error) {
      await this.client.abortPut(transaction).catch(() => false);
      throw error;
    }
  }

  async remove(key: string): Promise<boolean> {
    this.assertOpen();
    return this.client.remove(this.namespace, key);
  }

  async clear(): Promise<void> {
    this.assertOpen();
    if (!await this.client.clear(this.namespace)) throw new Error('web blob clear was rejected');
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.client.close();
  }
}
