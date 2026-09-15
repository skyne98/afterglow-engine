import {
  decodeBytes,
  decodeString,
  decodeU32,
  decodeU64,
  encodeBool,
  encodeBytes,
  encodeU32,
  encodeU64,
} from './codec.ts';
import { installRingService } from './ring-service.ts';
import { OpfsBlobStorageService } from './opfs-blob-storage.ts';

let service: OpfsBlobStorageService | null = null;
installRingService(() => { service = OpfsBlobStorageService.fromNavigator(); return serve; });

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
