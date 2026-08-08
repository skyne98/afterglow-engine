// crates/afterglow-web/web/src/workers/codec.ts
function encodeVarint(n) {
  const b = [];
  do {
    let x = n & 127;
    n = Math.floor(n / 128);
    if (n)
      x |= 128;
    b.push(x);
  } while (n);
  return b;
}
function decodeVarint(bytes, off) {
  let r = 0;
  for (let shift = 0;shift < 56; shift += 7) {
    if (off >= bytes.length)
      throw new Error("postcard varint truncated");
    const b = bytes[off++];
    r += (b & 127) * 2 ** shift;
    if (!(b & 128))
      return [r, off];
  }
  throw new Error("postcard varint overflows");
}
function concat(...arrs) {
  const out = new Uint8Array(arrs.reduce((s, a) => s + a.length, 0));
  let o = 0;
  for (const a of arrs) {
    out.set(a, o);
    o += a.length;
  }
  return out;
}
function encodeU32(n) {
  return new Uint8Array(encodeVarint(n));
}
function decodeU32(bytes, off) {
  return decodeVarint(bytes, off);
}
function encodeU64(n) {
  return new Uint8Array(encodeVarint(n));
}
function decodeU64(bytes, off) {
  return decodeVarint(bytes, off);
}
function encodeBool(b) {
  return new Uint8Array([b ? 1 : 0]);
}
function encodeString(s) {
  const enc = new TextEncoder().encode(s);
  return concat(encodeVarint(enc.length), enc);
}
function decodeString(bytes, off) {
  const [len, o] = decodeVarint(bytes, off);
  const end = o + len;
  if (end > bytes.length)
    throw new Error("postcard string truncated");
  return [new TextDecoder().decode(Uint8Array.from(bytes.subarray(o, end))), end];
}
function encodeBytes(b) {
  return concat(encodeVarint(b.length), b);
}
function decodeBytes(bytes, off) {
  const [len, o] = decodeVarint(bytes, off);
  const end = o + len;
  if (end > bytes.length)
    throw new Error("postcard bytes truncated");
  return [bytes.subarray(o, end), end];
}

// crates/afterglow-web/web/src/workers/ring-buf.ts
var U32 = 4;
var HEADER = 12;
function rdU32(u8, off, cap) {
  return (u8[off % cap] | u8[(off + 1) % cap] << 8 | u8[(off + 2) % cap] << 16 | u8[(off + 3) % cap] << 24) >>> 0;
}
function wrU32(u8, off, cap, val) {
  u8[off % cap] = val & 255;
  u8[(off + 1) % cap] = val >>> 8 & 255;
  u8[(off + 2) % cap] = val >>> 16 & 255;
  u8[(off + 3) % cap] = val >>> 24 & 255;
}
function xfer(u8, off, cap, buf, len, mode) {
  const o = off % cap, first = Math.min(len, cap - o);
  if (mode === "rd") {
    buf.set(u8.subarray(o, o + first), 0);
    if (first < len)
      buf.set(u8.subarray(0, len - first), first);
  } else {
    u8.set(buf.subarray(0, first), o);
    if (first < len)
      u8.set(buf.subarray(first), 0);
  }
}

// crates/afterglow-web/web/src/workers/opfs-blob-storage.ts
var POINTER_SUFFIX = ".ptr";
var SLOT0_SUFFIX = ".0";
var SLOT1_SUFFIX = ".1";
var ENVELOPE_MAGIC = 1112557377;
var ENVELOPE_BYTES = 16;
var TRANSACTION_CAPACITY = 8;
function validComponent(value, maximum) {
  return value.length > 0 && value.length <= maximum && /^[A-Za-z0-9._-]+$/.test(value);
}
function crcUpdate(crc, bytes, start = 0) {
  for (let index = start;index < bytes.byteLength; index++) {
    crc ^= bytes[index] ?? 0;
    for (let bit = 0;bit < 8; bit++)
      crc = crc >>> 1 ^ ((crc & 1) !== 0 ? 3988292384 : 0);
  }
  return crc >>> 0;
}
function crc32(bytes, start = 0) {
  return (crcUpdate(4294967295, bytes, start) ^ 4294967295) >>> 0;
}
async function readFile(directory, name) {
  try {
    return await (await directory.getFileHandle(name)).getFile();
  } catch (error) {
    if (error instanceof DOMException && error.name === "NotFoundError")
      return null;
    throw error;
  }
}
async function writeFile(directory, name, bytes) {
  const writable = await (await directory.getFileHandle(name, { create: true })).createWritable();
  try {
    await writable.write(bytes);
    await writable.close();
  } catch (error) {
    await writable.abort();
    throw error;
  }
}
function envelopeHeader(generation, length, checksum) {
  const output = new Uint8Array(ENVELOPE_BYTES);
  const view = new DataView(output.buffer);
  view.setUint32(0, ENVELOPE_MAGIC, true);
  view.setUint32(4, generation, true);
  view.setUint32(8, length, true);
  view.setUint32(12, checksum, true);
  return output;
}
function transactionParts(handle) {
  return [handle & 65535, handle >>> 16];
}
function transactionHandle(slot, generation) {
  return (generation & 65535) << 16 | slot;
}

class OpfsBlobStorageService {
  storage;
  transactions;
  root = null;
  constructor(storage) {
    this.storage = storage;
    this.transactions = new Array(TRANSACTION_CAPACITY);
    for (let index = 0;index < TRANSACTION_CAPACITY; index++)
      this.transactions[index] = { generation: 0, transaction: null };
  }
  static fromNavigator() {
    const storage = globalThis.navigator?.storage;
    if (!storage || typeof storage.getDirectory !== "function")
      throw new Error("OPFS is unavailable");
    return new OpfsBlobStorageService(storage);
  }
  async directory(namespace) {
    if (!validComponent(namespace, 64))
      throw new RangeError("invalid storage namespace");
    this.root ??= await this.storage.getDirectory();
    return this.root.getDirectoryHandle(namespace, { create: true });
  }
  async readGeneration(directory, key, slot, maxValueBytes) {
    const file = await readFile(directory, key + (slot === 0 ? SLOT0_SUFFIX : SLOT1_SUFFIX));
    if (!file || file.size < ENVELOPE_BYTES || file.size > ENVELOPE_BYTES + maxValueBytes)
      return null;
    const encoded = new Uint8Array(await file.arrayBuffer());
    const view = new DataView(encoded.buffer, encoded.byteOffset, encoded.byteLength);
    const length = view.getUint32(8, true);
    if (view.getUint32(0, true) !== ENVELOPE_MAGIC || length > maxValueBytes || encoded.byteLength !== ENVELOPE_BYTES + length || view.getUint32(12, true) !== crc32(encoded, ENVELOPE_BYTES))
      return null;
    const bytes = new Uint8Array(length);
    bytes.set(encoded.subarray(ENVELOPE_BYTES));
    return { slot, generation: view.getUint32(4, true), bytes };
  }
  async selectedGeneration(directory, key, maxValueBytes) {
    let preferred = null;
    const pointer = await readFile(directory, key + POINTER_SUFFIX);
    if (pointer?.size === 1) {
      const value = new Uint8Array(await pointer.arrayBuffer())[0];
      if (value === 0 || value === 1)
        preferred = value;
    }
    if (preferred !== null) {
      const selected = await this.readGeneration(directory, key, preferred, maxValueBytes);
      if (selected)
        return selected;
    }
    const zero = await this.readGeneration(directory, key, 0, maxValueBytes);
    const one = await this.readGeneration(directory, key, 1, maxValueBytes);
    if (!zero)
      return one;
    if (!one)
      return zero;
    return one.generation > zero.generation ? one : zero;
  }
  async list(namespace, maxEntries, maxValueBytes) {
    if (!Number.isInteger(maxEntries) || maxEntries < 0 || maxEntries > 4096)
      throw new RangeError("invalid storage list capacity");
    const directory = await this.directory(namespace);
    const entries = [];
    for await (const [name] of directory.entries()) {
      if (!name.endsWith(POINTER_SUFFIX))
        continue;
      const key = name.slice(0, -POINTER_SUFFIX.length);
      if (!validComponent(key, 128))
        throw new Error("invalid stored key");
      const selected = await this.selectedGeneration(directory, key, maxValueBytes);
      if (!selected)
        throw new Error("stored blob has no valid generation");
      if (entries.length === maxEntries)
        throw new Error("stored item capacity exceeded");
      entries.push({ key, size: selected.bytes.byteLength });
    }
    entries.sort((left, right) => left.key.localeCompare(right.key));
    return entries;
  }
  async size(namespace, key, maxValueBytes) {
    if (!validComponent(key, 128))
      throw new RangeError("invalid storage key");
    const selected = await this.selectedGeneration(await this.directory(namespace), key, maxValueBytes);
    if (!selected)
      throw new Error("blob not found");
    return selected.bytes.byteLength;
  }
  async read(namespace, key, offset, length, maxValueBytes) {
    if (!validComponent(key, 128))
      throw new RangeError("invalid storage key");
    const selected = await this.selectedGeneration(await this.directory(namespace), key, maxValueBytes);
    if (!selected)
      throw new Error("blob not found");
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > selected.bytes.byteLength || !Number.isInteger(length) || length < 0)
      throw new RangeError("invalid blob read range");
    return selected.bytes.slice(offset, Math.min(selected.bytes.byteLength, offset + length));
  }
  async beginPut(namespace, key, totalLength, checksum, maxValueBytes) {
    if (!validComponent(key, 128) || !Number.isSafeInteger(totalLength) || totalLength < 0 || totalLength > maxValueBytes || totalLength > 4294967295)
      throw new RangeError("invalid storage transaction");
    const directory = await this.directory(namespace);
    const active = await this.selectedGeneration(directory, key, maxValueBytes);
    const targetSlot = active?.slot === 0 ? 1 : 0;
    const persistedGeneration = active ? active.generation + 1 >>> 0 || 1 : 1;
    let slotIndex = -1;
    for (let index = 0;index < this.transactions.length; index++) {
      const candidate = this.transactions[index];
      if (candidate?.transaction?.namespace === namespace && candidate.transaction.key === key)
        throw new Error("blob key already has a transaction");
      if (slotIndex < 0 && candidate?.transaction === null)
        slotIndex = index;
    }
    if (slotIndex < 0)
      throw new Error("storage transaction capacity exceeded");
    const slot = this.transactions[slotIndex];
    slot.generation = slot.generation + 1 & 65535 || 1;
    const writable = await (await directory.getFileHandle(key + (targetSlot === 0 ? SLOT0_SUFFIX : SLOT1_SUFFIX), { create: true })).createWritable();
    try {
      await writable.write(envelopeHeader(persistedGeneration, totalLength, checksum));
    } catch (error) {
      await writable.abort();
      throw error;
    }
    slot.transaction = {
      generation: slot.generation,
      namespace,
      key,
      slot: targetSlot,
      totalLength,
      checksum: checksum >>> 0,
      writable,
      written: 0,
      crc: 4294967295
    };
    return transactionHandle(slotIndex, slot.generation);
  }
  async writeChunk(transaction, offset, bytes) {
    const [slotIndex, generation] = transactionParts(transaction);
    const slot = this.transactions[slotIndex];
    const tx = slot?.generation === generation ? slot.transaction : null;
    if (!tx)
      throw new Error("stale or closed storage transaction");
    if (offset !== tx.written || tx.written + bytes.byteLength > tx.totalLength)
      throw new RangeError("storage chunks must be sequential and in bounds");
    await tx.writable.write(bytes);
    tx.crc = crcUpdate(tx.crc, bytes);
    tx.written += bytes.byteLength;
    return bytes.byteLength;
  }
  async commitPut(transaction) {
    const [slotIndex, generation] = transactionParts(transaction);
    const slot = this.transactions[slotIndex];
    const tx = slot?.generation === generation ? slot.transaction : null;
    if (!slot || !tx)
      throw new Error("stale or closed storage transaction");
    slot.transaction = null;
    const checksum = (tx.crc ^ 4294967295) >>> 0;
    if (tx.written !== tx.totalLength || checksum !== tx.checksum) {
      await tx.writable.abort();
      throw new Error("storage transaction length or checksum mismatch");
    }
    await tx.writable.close();
    await writeFile(await this.directory(tx.namespace), tx.key + POINTER_SUFFIX, new Uint8Array([tx.slot]));
    return true;
  }
  async abortPut(transaction) {
    const [slotIndex, generation] = transactionParts(transaction);
    const slot = this.transactions[slotIndex];
    if (!slot || slot.generation !== generation || !slot.transaction)
      return false;
    const tx = slot.transaction;
    slot.transaction = null;
    await tx.writable.abort();
    return true;
  }
  async remove(namespace, key) {
    if (!validComponent(key, 128))
      throw new RangeError("invalid storage key");
    const directory = await this.directory(namespace);
    const existed = await readFile(directory, key + POINTER_SUFFIX) !== null;
    for (const suffix of [POINTER_SUFFIX, SLOT0_SUFFIX, SLOT1_SUFFIX]) {
      try {
        await directory.removeEntry(key + suffix);
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "NotFoundError"))
          throw error;
      }
    }
    return existed;
  }
  async clear(namespace) {
    const directory = await this.directory(namespace);
    const names = [];
    for await (const [name] of directory.entries())
      names.push(name);
    for (const name of names)
      await directory.removeEntry(name, { recursive: true });
    return true;
  }
}

// crates/afterglow-web/web/src/workers/storage-worker.ts
var state = "init";
var sab = null;
var requestBase = 0;
var responseBase = 0;
var bufferSize = 0;
var wakePending = false;
var wakeResolve = null;
var service = null;
self.onmessage = async (event) => {
  const message = event.data;
  if (state === "init" && message?.type === "init") {
    try {
      sab = message.sab;
      requestBase = message.reqBase;
      responseBase = message.respBase;
      bufferSize = message.bufSize;
      service = OpfsBlobStorageService.fromNavigator();
      state = "ready";
      self.postMessage({ type: "ready" });
    } catch (error) {
      self.postMessage({ type: "error", message: error instanceof Error ? error.message : String(error) });
    }
    return;
  }
  if (state === "ready" && message?.type === "run") {
    state = "running";
    runLoop();
    return;
  }
  if (state === "running" && message === "wake") {
    if (wakeResolve) {
      const resolve = wakeResolve;
      wakeResolve = null;
      resolve();
    } else
      wakePending = true;
  }
};
function waitForWake() {
  if (wakePending) {
    wakePending = false;
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    wakeResolve = resolve;
  });
}
function success(payload) {
  return concat(encodeVarint(0), encodeBytes(payload));
}
function failure(method, error) {
  const message = error instanceof Error ? error.message : String(error);
  return concat(encodeVarint(1), encodeVarint(method), encodeString(message));
}
function encodeList(entries) {
  let length = 4;
  const keys = new Array(entries.length);
  const encoder = new TextEncoder;
  for (let index = 0;index < entries.length; index++) {
    const key = encoder.encode(entries[index].key);
    if (key.byteLength > 255)
      throw new Error("storage key exceeds encoded index limit");
    keys[index] = key;
    length += 1 + key.byteLength + 8;
  }
  const output = new Uint8Array(length);
  const view = new DataView(output.buffer);
  view.setUint32(0, entries.length, true);
  let cursor = 4;
  for (let index = 0;index < entries.length; index++) {
    const key = keys[index];
    output[cursor++] = key.byteLength;
    output.set(key, cursor);
    cursor += key.byteLength;
    view.setBigUint64(cursor, BigInt(entries[index].size), true);
    cursor += 8;
  }
  return output;
}
async function serve(method, args) {
  if (!service)
    throw new Error("storage service is not initialized");
  let offset = 0;
  if (method === 0) {
    let namespace, maxEntries, maxValueBytes;
    [namespace, offset] = decodeString(args, offset);
    [maxEntries, offset] = decodeU32(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeBytes(encodeList(await service.list(namespace, maxEntries, maxValueBytes)));
  }
  if (method === 1) {
    let namespace, key, maxValueBytes;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeU64(await service.size(namespace, key, maxValueBytes));
  }
  if (method === 2) {
    let namespace, key, readOffset, length, maxValueBytes;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    [readOffset, offset] = decodeU64(args, offset);
    [length, offset] = decodeU32(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeBytes(await service.read(namespace, key, readOffset, length, maxValueBytes));
  }
  if (method === 3) {
    let namespace, key, totalLength, checksum, maxValueBytes;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    [totalLength, offset] = decodeU64(args, offset);
    [checksum, offset] = decodeU32(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeU32(await service.beginPut(namespace, key, totalLength, checksum, maxValueBytes));
  }
  if (method === 4) {
    let transaction, writeOffset, bytes;
    [transaction, offset] = decodeU32(args, offset);
    [writeOffset, offset] = decodeU64(args, offset);
    [bytes, offset] = decodeBytes(args, offset);
    return encodeU32(await service.writeChunk(transaction, writeOffset, bytes));
  }
  if (method === 5) {
    const [transaction] = decodeU32(args, offset);
    return encodeBool(await service.commitPut(transaction));
  }
  if (method === 6) {
    const [transaction] = decodeU32(args, offset);
    return encodeBool(await service.abortPut(transaction));
  }
  if (method === 7) {
    let namespace, key;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    return encodeBool(await service.remove(namespace, key));
  }
  if (method === 8) {
    const [namespace] = decodeString(args, offset);
    return encodeBool(await service.clear(namespace));
  }
  throw new Error(`unknown storage method ${method}`);
}
async function runLoop() {
  const memory = sab;
  if (!memory)
    return;
  const requestCapacity = Atomics.load(new Uint32Array(memory, requestBase, 1), 0) >>> 0;
  const responseCapacity = Atomics.load(new Uint32Array(memory, responseBase, 1), 0) >>> 0;
  if (requestCapacity === 0 || requestCapacity !== bufferSize - HEADER || responseCapacity === 0 || responseCapacity !== bufferSize - HEADER) {
    self.postMessage({ type: "error", message: "bad storage ring capacity" });
    return;
  }
  const requestWrite = new Int32Array(memory, requestBase + U32, 1);
  const requestRead = new Int32Array(memory, requestBase + 2 * U32, 1);
  const requestData = new Uint8Array(memory, requestBase + HEADER, requestCapacity);
  const responseWrite = new Int32Array(memory, responseBase + U32, 1);
  const responseRead = new Int32Array(memory, responseBase + 2 * U32, 1);
  const responseData = new Uint8Array(memory, responseBase + HEADER, responseCapacity);
  for (;; ) {
    const write = Atomics.load(requestWrite, 0) >>> 0;
    const read = Atomics.load(requestRead, 0) >>> 0;
    const used = write - read >>> 0;
    if (used === 0) {
      await waitForWake();
      continue;
    }
    if (used > requestCapacity || used < U32) {
      Atomics.store(requestRead, 0, write);
      self.postMessage({ type: "error", message: "corrupt storage request ring" });
      continue;
    }
    const ringOffset = read % requestCapacity;
    const payloadLength = rdU32(requestData, ringOffset, requestCapacity);
    const frameLength = U32 + payloadLength;
    if (payloadLength < U32 || frameLength > used || frameLength > requestCapacity) {
      Atomics.store(requestRead, 0, write);
      self.postMessage({ type: "error", message: "corrupt storage request frame" });
      continue;
    }
    const method = rdU32(requestData, ringOffset + U32, requestCapacity) >>> 0;
    const args = new Uint8Array(payloadLength - U32);
    xfer(requestData, ringOffset + 2 * U32, requestCapacity, args, args.byteLength, "rd");
    Atomics.store(requestRead, 0, read + frameLength >>> 0);
    let response;
    try {
      response = success(await serve(method, args));
    } catch (error) {
      response = failure(method, error);
    }
    const responseFrameLength = U32 + response.byteLength;
    const responseWriteIndex = Atomics.load(responseWrite, 0) >>> 0;
    const responseReadIndex = Atomics.load(responseRead, 0) >>> 0;
    const responseUsed = responseWriteIndex - responseReadIndex >>> 0;
    if (responseFrameLength > responseCapacity || responseFrameLength > responseCapacity - responseUsed) {
      self.postMessage({ type: "error", message: "storage response ring full" });
      continue;
    }
    const responseOffset = responseWriteIndex % responseCapacity;
    wrU32(responseData, responseOffset, responseCapacity, response.byteLength);
    xfer(responseData, responseOffset + U32, responseCapacity, response, response.byteLength, "wr");
    Atomics.store(responseWrite, 0, responseWriteIndex + responseFrameLength >>> 0);
    self.postMessage("wake");
  }
}
