import {
  concat,
  decodeBytes,
  decodeString,
  decodeU32,
  decodeU64,
  encodeBool,
  encodeBytes,
  encodeString,
  encodeU32,
  encodeU64,
  encodeVarint,
} from './codec.ts';
import { HEADER, U32, rdU32, wrU32, xfer } from './ring-buf.ts';
import { OpfsBlobStorageService } from './opfs-blob-storage.ts';

let state: 'init' | 'ready' | 'running' = 'init';
let sab: SharedArrayBuffer | null = null;
let requestBase = 0;
let responseBase = 0;
let bufferSize = 0;
let wakePending = false;
let wakeResolve: (() => void) | null = null;
let service: OpfsBlobStorageService | null = null;

self.onmessage = async (event: MessageEvent): Promise<void> => {
  const message = event.data;
  if (state === 'init' && message?.type === 'init') {
    try {
      sab = message.sab as SharedArrayBuffer;
      requestBase = message.reqBase;
      responseBase = message.respBase;
      bufferSize = message.bufSize;
      service = OpfsBlobStorageService.fromNavigator();
      state = 'ready';
      self.postMessage({ type: 'ready' });
    } catch (error) {
      self.postMessage({ type: 'error', message: error instanceof Error ? error.message : String(error) });
    }
    return;
  }
  if (state === 'ready' && message?.type === 'run') {
    state = 'running';
    void runLoop();
    return;
  }
  if (state === 'running' && message === 'wake') {
    if (wakeResolve) {
      const resolve = wakeResolve;
      wakeResolve = null;
      resolve();
    } else wakePending = true;
  }
};

function waitForWake(): Promise<void> {
  if (wakePending) {
    wakePending = false;
    return Promise.resolve();
  }
  return new Promise(resolve => { wakeResolve = resolve; });
}

function success(payload: Uint8Array): Uint8Array {
  return concat(encodeVarint(0), encodeBytes(payload));
}
function failure(method: number, error: unknown): Uint8Array {
  const message = error instanceof Error ? error.message : String(error);
  return concat(encodeVarint(1), encodeVarint(method), encodeString(message));
}

function encodeList(entries: readonly { readonly key: string; readonly size: number }[]): Uint8Array {
  let length = 4;
  const keys = new Array<Uint8Array>(entries.length);
  const encoder = new TextEncoder();
  for (let index = 0; index < entries.length; index++) {
    const key = encoder.encode(entries[index]!.key);
    if (key.byteLength > 255) throw new Error('storage key exceeds encoded index limit');
    keys[index] = key;
    length += 1 + key.byteLength + 8;
  }
  const output = new Uint8Array(length);
  const view = new DataView(output.buffer);
  view.setUint32(0, entries.length, true);
  let cursor = 4;
  for (let index = 0; index < entries.length; index++) {
    const key = keys[index]!;
    output[cursor++] = key.byteLength;
    output.set(key, cursor);
    cursor += key.byteLength;
    view.setBigUint64(cursor, BigInt(entries[index]!.size), true);
    cursor += 8;
  }
  return output;
}

async function serve(method: number, args: Uint8Array): Promise<Uint8Array> {
  if (!service) throw new Error('storage service is not initialized');
  let offset = 0;
  if (method === 0) {
    let namespace: string, maxEntries: number, maxValueBytes: number;
    [namespace, offset] = decodeString(args, offset);
    [maxEntries, offset] = decodeU32(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeBytes(encodeList(await service.list(namespace, maxEntries, maxValueBytes)));
  }
  if (method === 1) {
    let namespace: string, key: string, maxValueBytes: number;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeU64(await service.size(namespace, key, maxValueBytes));
  }
  if (method === 2) {
    let namespace: string, key: string, readOffset: number, length: number, maxValueBytes: number;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    [readOffset, offset] = decodeU64(args, offset);
    [length, offset] = decodeU32(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeBytes(await service.read(namespace, key, readOffset, length, maxValueBytes));
  }
  if (method === 3) {
    let namespace: string, key: string, totalLength: number, checksum: number, maxValueBytes: number;
    [namespace, offset] = decodeString(args, offset);
    [key, offset] = decodeString(args, offset);
    [totalLength, offset] = decodeU64(args, offset);
    [checksum, offset] = decodeU32(args, offset);
    [maxValueBytes, offset] = decodeU64(args, offset);
    return encodeU32(await service.beginPut(namespace, key, totalLength, checksum, maxValueBytes));
  }
  if (method === 4) {
    let transaction: number, writeOffset: number, bytes: Uint8Array;
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
    let namespace: string, key: string;
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

async function runLoop(): Promise<void> {
  const memory = sab;
  if (!memory) return;
  const requestCapacity = Atomics.load(new Uint32Array(memory, requestBase, 1), 0) >>> 0;
  const responseCapacity = Atomics.load(new Uint32Array(memory, responseBase, 1), 0) >>> 0;
  if (requestCapacity === 0 || requestCapacity !== bufferSize - HEADER ||
      responseCapacity === 0 || responseCapacity !== bufferSize - HEADER) {
    self.postMessage({ type: 'error', message: 'bad storage ring capacity' });
    return;
  }
  const requestWrite = new Int32Array(memory, requestBase + U32, 1);
  const requestRead = new Int32Array(memory, requestBase + 2 * U32, 1);
  const requestData = new Uint8Array(memory, requestBase + HEADER, requestCapacity);
  const responseWrite = new Int32Array(memory, responseBase + U32, 1);
  const responseRead = new Int32Array(memory, responseBase + 2 * U32, 1);
  const responseData = new Uint8Array(memory, responseBase + HEADER, responseCapacity);

  for (;;) {
    const write = Atomics.load(requestWrite, 0) >>> 0;
    const read = Atomics.load(requestRead, 0) >>> 0;
    const used = (write - read) >>> 0;
    if (used === 0) { await waitForWake(); continue; }
    if (used > requestCapacity || used < U32) {
      Atomics.store(requestRead, 0, write);
      self.postMessage({ type: 'error', message: 'corrupt storage request ring' });
      continue;
    }
    const ringOffset = read % requestCapacity;
    const payloadLength = rdU32(requestData, ringOffset, requestCapacity);
    const frameLength = U32 + payloadLength;
    if (payloadLength < U32 || frameLength > used || frameLength > requestCapacity) {
      Atomics.store(requestRead, 0, write);
      self.postMessage({ type: 'error', message: 'corrupt storage request frame' });
      continue;
    }
    const method = rdU32(requestData, ringOffset + U32, requestCapacity) >>> 0;
    const args = new Uint8Array(payloadLength - U32);
    xfer(requestData, ringOffset + 2 * U32, requestCapacity, args, args.byteLength, 'rd');
    Atomics.store(requestRead, 0, (read + frameLength) >>> 0);

    let response: Uint8Array;
    try { response = success(await serve(method, args)); }
    catch (error) { response = failure(method, error); }
    const responseFrameLength = U32 + response.byteLength;
    const responseWriteIndex = Atomics.load(responseWrite, 0) >>> 0;
    const responseReadIndex = Atomics.load(responseRead, 0) >>> 0;
    const responseUsed = (responseWriteIndex - responseReadIndex) >>> 0;
    if (responseFrameLength > responseCapacity || responseFrameLength > responseCapacity - responseUsed) {
      self.postMessage({ type: 'error', message: 'storage response ring full' });
      continue;
    }
    const responseOffset = responseWriteIndex % responseCapacity;
    wrU32(responseData, responseOffset, responseCapacity, response.byteLength);
    xfer(responseData, responseOffset + U32, responseCapacity, response, response.byteLength, 'wr');
    Atomics.store(responseWrite, 0, (responseWriteIndex + responseFrameLength) >>> 0);
    self.postMessage('wake');
  }
}
