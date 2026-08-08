const POINTER_SUFFIX = '.ptr';
const SLOT0_SUFFIX = '.0';
const SLOT1_SUFFIX = '.1';
const ENVELOPE_MAGIC = 0x42504741; // AGPB
const ENVELOPE_BYTES = 16;
const TRANSACTION_CAPACITY = 8;

interface WritableFile {
  write(data: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort(): Promise<void>;
}
interface StoredFile { readonly size: number; arrayBuffer(): Promise<ArrayBuffer> }
interface FileHandle {
  getFile(): Promise<StoredFile>;
  createWritable(): Promise<WritableFile>;
}
interface DirectoryHandle {
  getFileHandle(name: string, options?: { create?: boolean }): Promise<FileHandle>;
  getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<DirectoryHandle>;
  removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
  entries(): AsyncIterableIterator<[string, unknown]>;
}
interface OpfsStorageManager { getDirectory(): Promise<DirectoryHandle> }

interface StoredGeneration {
  readonly slot: 0 | 1;
  readonly generation: number;
  readonly bytes: Uint8Array;
}
interface PutTransaction {
  readonly generation: number;
  readonly namespace: string;
  readonly key: string;
  readonly slot: 0 | 1;
  readonly totalLength: number;
  readonly checksum: number;
  readonly writable: WritableFile;
  written: number;
  crc: number;
}
interface TransactionSlot {
  generation: number;
  transaction: PutTransaction | null;
}

export interface OpfsBlobEntry { readonly key: string; readonly size: number }

function validComponent(value: string, maximum: number): boolean {
  return value.length > 0 && value.length <= maximum && /^[A-Za-z0-9._-]+$/.test(value);
}

function crcUpdate(crc: number, bytes: Uint8Array, start = 0): number {
  for (let index = start; index < bytes.byteLength; index++) {
    crc ^= bytes[index] ?? 0;
    for (let bit = 0; bit < 8; bit++)
      crc = (crc >>> 1) ^ ((crc & 1) !== 0 ? 0xedb8_8320 : 0);
  }
  return crc >>> 0;
}

function crc32(bytes: Uint8Array, start = 0): number {
  return (crcUpdate(0xffff_ffff, bytes, start) ^ 0xffff_ffff) >>> 0;
}

async function readFile(directory: DirectoryHandle, name: string): Promise<StoredFile | null> {
  try { return await (await directory.getFileHandle(name)).getFile(); }
  catch (error) {
    if (error instanceof DOMException && error.name === 'NotFoundError') return null;
    throw error;
  }
}

async function writeFile(directory: DirectoryHandle, name: string, bytes: Uint8Array): Promise<void> {
  const writable = await (await directory.getFileHandle(name, { create: true })).createWritable();
  try {
    await writable.write(bytes);
    await writable.close();
  } catch (error) {
    await writable.abort();
    throw error;
  }
}

function envelopeHeader(generation: number, length: number, checksum: number): Uint8Array {
  const output = new Uint8Array(ENVELOPE_BYTES);
  const view = new DataView(output.buffer);
  view.setUint32(0, ENVELOPE_MAGIC, true);
  view.setUint32(4, generation, true);
  view.setUint32(8, length, true);
  view.setUint32(12, checksum, true);
  return output;
}

function transactionParts(handle: number): [number, number] {
  return [handle & 0xffff, handle >>> 16];
}
function transactionHandle(slot: number, generation: number): number {
  return ((generation & 0xffff) << 16) | slot;
}

/** OPFS implementation that lives only in the storage Worker. */
export class OpfsBlobStorageService {
  private readonly transactions: TransactionSlot[];
  private root: DirectoryHandle | null = null;

  constructor(private readonly storage: OpfsStorageManager) {
    this.transactions = new Array<TransactionSlot>(TRANSACTION_CAPACITY);
    for (let index = 0; index < TRANSACTION_CAPACITY; index++)
      this.transactions[index] = { generation: 0, transaction: null };
  }

  static fromNavigator(): OpfsBlobStorageService {
    const storage = globalThis.navigator?.storage as unknown as OpfsStorageManager | undefined;
    if (!storage || typeof storage.getDirectory !== 'function') throw new Error('OPFS is unavailable');
    return new OpfsBlobStorageService(storage);
  }

  private async directory(namespace: string): Promise<DirectoryHandle> {
    if (!validComponent(namespace, 64)) throw new RangeError('invalid storage namespace');
    this.root ??= await this.storage.getDirectory();
    return this.root.getDirectoryHandle(namespace, { create: true });
  }

  private async readGeneration(
    directory: DirectoryHandle, key: string, slot: 0 | 1, maxValueBytes: number,
  ): Promise<StoredGeneration | null> {
    const file = await readFile(directory, key + (slot === 0 ? SLOT0_SUFFIX : SLOT1_SUFFIX));
    if (!file || file.size < ENVELOPE_BYTES || file.size > ENVELOPE_BYTES + maxValueBytes) return null;
    const encoded = new Uint8Array(await file.arrayBuffer());
    const view = new DataView(encoded.buffer, encoded.byteOffset, encoded.byteLength);
    const length = view.getUint32(8, true);
    if (view.getUint32(0, true) !== ENVELOPE_MAGIC || length > maxValueBytes ||
        encoded.byteLength !== ENVELOPE_BYTES + length ||
        view.getUint32(12, true) !== crc32(encoded, ENVELOPE_BYTES)) return null;
    const bytes = new Uint8Array(length);
    bytes.set(encoded.subarray(ENVELOPE_BYTES));
    return { slot, generation: view.getUint32(4, true), bytes };
  }

  private async selectedGeneration(
    directory: DirectoryHandle, key: string, maxValueBytes: number,
  ): Promise<StoredGeneration | null> {
    let preferred: 0 | 1 | null = null;
    const pointer = await readFile(directory, key + POINTER_SUFFIX);
    if (pointer?.size === 1) {
      const value = new Uint8Array(await pointer.arrayBuffer())[0];
      if (value === 0 || value === 1) preferred = value;
    }
    if (preferred !== null) {
      const selected = await this.readGeneration(directory, key, preferred, maxValueBytes);
      if (selected) return selected;
    }
    const zero = await this.readGeneration(directory, key, 0, maxValueBytes);
    const one = await this.readGeneration(directory, key, 1, maxValueBytes);
    if (!zero) return one;
    if (!one) return zero;
    return one.generation > zero.generation ? one : zero;
  }

  async list(namespace: string, maxEntries: number, maxValueBytes: number): Promise<readonly OpfsBlobEntry[]> {
    if (!Number.isInteger(maxEntries) || maxEntries < 0 || maxEntries > 4096)
      throw new RangeError('invalid storage list capacity');
    const directory = await this.directory(namespace);
    const entries: OpfsBlobEntry[] = [];
    for await (const [name] of directory.entries()) {
      if (!name.endsWith(POINTER_SUFFIX)) continue;
      const key = name.slice(0, -POINTER_SUFFIX.length);
      if (!validComponent(key, 128)) throw new Error('invalid stored key');
      const selected = await this.selectedGeneration(directory, key, maxValueBytes);
      if (!selected) throw new Error('stored blob has no valid generation');
      if (entries.length === maxEntries) throw new Error('stored item capacity exceeded');
      entries.push({ key, size: selected.bytes.byteLength });
    }
    entries.sort((left, right) => left.key.localeCompare(right.key));
    return entries;
  }

  async size(namespace: string, key: string, maxValueBytes: number): Promise<number> {
    if (!validComponent(key, 128)) throw new RangeError('invalid storage key');
    const selected = await this.selectedGeneration(await this.directory(namespace), key, maxValueBytes);
    if (!selected) throw new Error('blob not found');
    return selected.bytes.byteLength;
  }

  async read(
    namespace: string, key: string, offset: number, length: number, maxValueBytes: number,
  ): Promise<Uint8Array> {
    if (!validComponent(key, 128)) throw new RangeError('invalid storage key');
    const selected = await this.selectedGeneration(await this.directory(namespace), key, maxValueBytes);
    if (!selected) throw new Error('blob not found');
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > selected.bytes.byteLength ||
        !Number.isInteger(length) || length < 0) throw new RangeError('invalid blob read range');
    return selected.bytes.slice(offset, Math.min(selected.bytes.byteLength, offset + length));
  }

  async beginPut(
    namespace: string, key: string, totalLength: number, checksum: number, maxValueBytes: number,
  ): Promise<number> {
    if (!validComponent(key, 128) || !Number.isSafeInteger(totalLength) || totalLength < 0 ||
        totalLength > maxValueBytes || totalLength > 0xffff_ffff)
      throw new RangeError('invalid storage transaction');
    const directory = await this.directory(namespace);
    const active = await this.selectedGeneration(directory, key, maxValueBytes);
    const targetSlot: 0 | 1 = active?.slot === 0 ? 1 : 0;
    const persistedGeneration = active ? ((active.generation + 1) >>> 0 || 1) : 1;
    let slotIndex = -1;
    for (let index = 0; index < this.transactions.length; index++) {
      const candidate = this.transactions[index];
      if (candidate?.transaction?.namespace === namespace && candidate.transaction.key === key)
        throw new Error('blob key already has a transaction');
      if (slotIndex < 0 && candidate?.transaction === null) slotIndex = index;
    }
    if (slotIndex < 0) throw new Error('storage transaction capacity exceeded');
    const slot = this.transactions[slotIndex]!;
    slot.generation = ((slot.generation + 1) & 0xffff) || 1;
    const writable = await (await directory.getFileHandle(
      key + (targetSlot === 0 ? SLOT0_SUFFIX : SLOT1_SUFFIX), { create: true },
    )).createWritable();
    try { await writable.write(envelopeHeader(persistedGeneration, totalLength, checksum)); }
    catch (error) { await writable.abort(); throw error; }
    slot.transaction = {
      generation: slot.generation, namespace, key, slot: targetSlot,
      totalLength, checksum: checksum >>> 0, writable, written: 0, crc: 0xffff_ffff,
    };
    return transactionHandle(slotIndex, slot.generation);
  }

  async writeChunk(transaction: number, offset: number, bytes: Uint8Array): Promise<number> {
    const [slotIndex, generation] = transactionParts(transaction);
    const slot = this.transactions[slotIndex];
    const tx = slot?.generation === generation ? slot.transaction : null;
    if (!tx) throw new Error('stale or closed storage transaction');
    if (offset !== tx.written || tx.written + bytes.byteLength > tx.totalLength)
      throw new RangeError('storage chunks must be sequential and in bounds');
    await tx.writable.write(bytes);
    tx.crc = crcUpdate(tx.crc, bytes);
    tx.written += bytes.byteLength;
    return bytes.byteLength;
  }

  async commitPut(transaction: number): Promise<boolean> {
    const [slotIndex, generation] = transactionParts(transaction);
    const slot = this.transactions[slotIndex];
    const tx = slot?.generation === generation ? slot.transaction : null;
    if (!slot || !tx) throw new Error('stale or closed storage transaction');
    slot.transaction = null;
    const checksum = (tx.crc ^ 0xffff_ffff) >>> 0;
    if (tx.written !== tx.totalLength || checksum !== tx.checksum) {
      await tx.writable.abort();
      throw new Error('storage transaction length or checksum mismatch');
    }
    await tx.writable.close();
    await writeFile(await this.directory(tx.namespace), tx.key + POINTER_SUFFIX, new Uint8Array([tx.slot]));
    return true;
  }

  async abortPut(transaction: number): Promise<boolean> {
    const [slotIndex, generation] = transactionParts(transaction);
    const slot = this.transactions[slotIndex];
    if (!slot || slot.generation !== generation || !slot.transaction) return false;
    const tx = slot.transaction;
    slot.transaction = null;
    await tx.writable.abort();
    return true;
  }

  async remove(namespace: string, key: string): Promise<boolean> {
    if (!validComponent(key, 128)) throw new RangeError('invalid storage key');
    const directory = await this.directory(namespace);
    const existed = (await readFile(directory, key + POINTER_SUFFIX)) !== null;
    for (const suffix of [POINTER_SUFFIX, SLOT0_SUFFIX, SLOT1_SUFFIX]) {
      try { await directory.removeEntry(key + suffix); }
      catch (error) {
        if (!(error instanceof DOMException && error.name === 'NotFoundError')) throw error;
      }
    }
    return existed;
  }

  async clear(namespace: string): Promise<boolean> {
    const directory = await this.directory(namespace);
    const names: string[] = [];
    for await (const [name] of directory.entries()) names.push(name);
    for (const name of names) await directory.removeEntry(name, { recursive: true });
    return true;
  }
}
