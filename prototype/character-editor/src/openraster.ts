export type ZipEntry = {
  name: string;
  data: Uint8Array;
};

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();
const crcTable = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let value = i;
  for (let bit = 0; bit < 8; bit++) {
    value = (value & 1) !== 0 ? (value >>> 1) ^ 0xedb88320 : value >>> 1;
  }
  crcTable[i] = value >>> 0;
}

function crc32(data: Uint8Array): number {
  let value = 0xffffffff;
  for (let i = 0; i < data.length; i++) {
    value = crcTable[(value ^ data[i]) & 255] ^ (value >>> 8);
  }
  return (value ^ 0xffffffff) >>> 0;
}

function write16(view: DataView, offset: number, value: number): void {
  view.setUint16(offset, value, true);
}

function write32(view: DataView, offset: number, value: number): void {
  view.setUint32(offset, value >>> 0, true);
}

function read16(view: DataView, offset: number): number {
  return view.getUint16(offset, true);
}

function read32(view: DataView, offset: number): number {
  return view.getUint32(offset, true);
}

function concat(parts: Uint8Array[]): Uint8Array {
  let size = 0;
  for (const part of parts) size += part.length;
  const result = new Uint8Array(size);
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }
  return result;
}

export function encodeStoredZip(entries: ZipEntry[]): Uint8Array {
  const locals: Uint8Array[] = [];
  const centrals: Uint8Array[] = [];
  let offset = 0;
  for (const entry of entries) {
    const name = textEncoder.encode(entry.name);
    const data = entry.data;
    const crc = crc32(data);
    const local = new Uint8Array(30 + name.length + data.length);
    const localView = new DataView(local.buffer);
    write32(localView, 0, 0x04034b50);
    write16(localView, 4, 20);
    write16(localView, 6, 0);
    write16(localView, 8, 0);
    write16(localView, 10, 0);
    write16(localView, 12, 0);
    write32(localView, 14, crc);
    write32(localView, 18, data.length);
    write32(localView, 22, data.length);
    write16(localView, 26, name.length);
    write16(localView, 28, 0);
    local.set(name, 30);
    local.set(data, 30 + name.length);
    locals.push(local);

    const central = new Uint8Array(46 + name.length);
    const centralView = new DataView(central.buffer);
    write32(centralView, 0, 0x02014b50);
    write16(centralView, 4, 20);
    write16(centralView, 6, 20);
    write16(centralView, 8, 0);
    write16(centralView, 10, 0);
    write16(centralView, 12, 0);
    write16(centralView, 14, 0);
    write32(centralView, 16, crc);
    write32(centralView, 20, data.length);
    write32(centralView, 24, data.length);
    write16(centralView, 28, name.length);
    write16(centralView, 30, 0);
    write16(centralView, 32, 0);
    write16(centralView, 34, 0);
    write16(centralView, 36, 0);
    write32(centralView, 38, 0);
    write32(centralView, 42, offset);
    central.set(name, 46);
    centrals.push(central);
    offset += local.length;
  }
  const centralData = concat(centrals);
  const end = new Uint8Array(22);
  const endView = new DataView(end.buffer);
  write32(endView, 0, 0x06054b50);
  write16(endView, 4, 0);
  write16(endView, 6, 0);
  write16(endView, 8, entries.length);
  write16(endView, 10, entries.length);
  write32(endView, 12, centralData.length);
  write32(endView, 16, offset);
  write16(endView, 20, 0);
  return concat([...locals, centralData, end]);
}

async function inflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

export async function decodeZip(input: ArrayBuffer): Promise<Map<string, Uint8Array>> {
  const bytes = new Uint8Array(input);
  const view = new DataView(input);
  let end = -1;
  for (let offset = bytes.length - 22; offset >= 0; offset--) {
    if (read32(view, offset) === 0x06054b50) {
      end = offset;
      break;
    }
  }
  if (end < 0) throw new Error('The OpenRaster file has no ZIP directory.');
  const count = read16(view, end + 10);
  const centralOffset = read32(view, end + 16);
  const result = new Map<string, Uint8Array>();
  let cursor = centralOffset;
  for (let i = 0; i < count; i++) {
    if (read32(view, cursor) !== 0x02014b50) throw new Error('The OpenRaster ZIP directory is invalid.');
    const method = read16(view, cursor + 10);
    const compressedSize = read32(view, cursor + 20);
    const nameLength = read16(view, cursor + 28);
    const extraLength = read16(view, cursor + 30);
    const commentLength = read16(view, cursor + 32);
    const localOffset = read32(view, cursor + 42);
    const name = textDecoder.decode(bytes.subarray(cursor + 46, cursor + 46 + nameLength));
    const localNameLength = read16(view, localOffset + 26);
    const localExtraLength = read16(view, localOffset + 28);
    const dataStart = localOffset + 30 + localNameLength + localExtraLength;
    const compressed = bytes.subarray(dataStart, dataStart + compressedSize);
    const data = method === 0 ? compressed.slice() : method === 8 ? await inflateRaw(compressed) : null;
    if (!data) throw new Error(`The OpenRaster ZIP method ${method} is not supported.`);
    result.set(name, data);
    cursor += 46 + nameLength + extraLength + commentLength;
  }
  return result;
}

export function utf8(value: string): Uint8Array {
  return textEncoder.encode(value);
}

export function text(value: Uint8Array): string {
  return textDecoder.decode(value);
}
