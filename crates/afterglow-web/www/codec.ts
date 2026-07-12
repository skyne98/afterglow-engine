// Postcard codec for afterglow-engine TS clients.
//
// Generated #[rpc] clients import from here. Postcard format:
//   - unsigned ints (u8/u16/u32/u64/usize): LEB128 varint
//   - signed ints (i8/i16/i32/i64/isize): zigzag-encoded varint
//   - f32/f64: little-endian raw bytes
//   - bool: 1 byte
//   - String: varint(len) + UTF-8 bytes
//   - Vec<u8>: varint(len) + raw bytes
//   - Vec<f32>/Vec<f64>: varint(count) + raw bytes
//
// All decode functions return [value, newOffset]. u64 precision is limited to
// Number.MAX_SAFE_INTEGER (2^53); sufficient for asset sizes/offsets.

/// A transport with a typed `call` — the low-level ring-buffer RPC.
export type RpcTransport = {
  call(method: number, args: Uint8Array): Promise<Uint8Array>;
};

// --- varint (unsigned LEB128) -------------------------------------------

export function encodeVarint(n: number): number[] {
  const b: number[] = [];
  do {
    let x = n & 0x7f;
    n = Math.floor(n / 128);
    if (n) x |= 0x80;
    b.push(x);
  } while (n);
  return b;
}

export function decodeVarint(bytes: Uint8Array, off: number): [number, number] {
  let r = 0;
  for (let shift = 0; shift < 56; shift += 7) {
    if (off >= bytes.length) throw new Error('postcard varint truncated');
    const b = bytes[off++];
    r += (b & 0x7f) * 2 ** shift;
    if (!(b & 0x80)) return [r, off];
  }
  throw new Error('postcard varint overflows');
}

// --- zigzag (signed) ----------------------------------------------------

export function encodeZigzag(n: number): number[] {
  // zigzag: (n << 1) ^ (n >> 63), then varint. Works for safe-integer range.
  const zz = (n < 0 ? -n * 2 - 1 : n * 2);
  return encodeVarint(zz);
}

export function decodeZigzag(bytes: Uint8Array, off: number): [number, number] {
  const [zz, o] = decodeVarint(bytes, off);
  return [(zz & 1 ? -(zz + 1) / 2 : zz / 2), o];
}

// --- concat -------------------------------------------------------------

export function concat(...arrs: Array<Uint8Array | number[]>): Uint8Array {
  const out = new Uint8Array(arrs.reduce((s, a) => s + a.length, 0));
  let o = 0;
  for (const a of arrs) { out.set(a, o); o += a.length; }
  return out;
}

// --- unsigned integers --------------------------------------------------

export function encodeU8(n: number): Uint8Array {
  return new Uint8Array([n & 0xff]);
}
export function decodeU8(bytes: Uint8Array, off: number): [number, number] {
  if (off >= bytes.length) throw new Error('postcard u8 truncated');
  return [bytes[off], off + 1];
}

export function encodeU16(n: number): Uint8Array {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setUint16(0, n, true);
  return b;
}
export function decodeU16(bytes: Uint8Array, off: number): [number, number] {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 2).getUint16(0, true), off + 2];
}

export function encodeU32(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n, true);
  return b;
}
export function decodeU32(bytes: Uint8Array, off: number): [number, number] {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 4).getUint32(0, true), off + 4];
}

export function encodeU64(n: number): Uint8Array {
  return new Uint8Array(encodeVarint(n));
}
export function decodeU64(bytes: Uint8Array, off: number): [number, number] {
  return decodeVarint(bytes, off);
}

// --- signed integers (zigzag varint) ------------------------------------

export function encodeI8(n: number): Uint8Array {
  return new Uint8Array([n & 0xff]);
}
export function decodeI8(bytes: Uint8Array, off: number): [number, number] {
  if (off >= bytes.length) throw new Error('postcard i8 truncated');
  let v = bytes[off];
  if (v > 127) v -= 256;
  return [v, off + 1];
}

export function encodeI16(n: number): Uint8Array {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setInt16(0, n, true);
  return b;
}
export function decodeI16(bytes: Uint8Array, off: number): [number, number] {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 2).getInt16(0, true), off + 2];
}

export function encodeI32(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setInt32(0, n, true);
  return b;
}
export function decodeI32(bytes: Uint8Array, off: number): [number, number] {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 4).getInt32(0, true), off + 4];
}

export function encodeI64(n: number): Uint8Array {
  return new Uint8Array(encodeZigzag(n));
}
export function decodeI64(bytes: Uint8Array, off: number): [number, number] {
  return decodeZigzag(bytes, off);
}

// --- float --------------------------------------------------------------

export function encodeF32(x: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return b;
}
export function decodeF32(bytes: Uint8Array, off: number): [number, number] {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 4).getFloat32(0, true), off + 4];
}

export function encodeF64(x: number): Uint8Array {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setFloat64(0, x, true);
  return b;
}
export function decodeF64(bytes: Uint8Array, off: number): [number, number] {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 8).getFloat64(0, true), off + 8];
}

// --- bool ---------------------------------------------------------------

export function encodeBool(b: boolean): Uint8Array {
  return new Uint8Array([b ? 1 : 0]);
}
export function decodeBool(bytes: Uint8Array, off: number): [boolean, number] {
  if (off >= bytes.length) throw new Error('postcard bool truncated');
  return [bytes[off] !== 0, off + 1];
}

// --- string -------------------------------------------------------------

export function encodeString(s: string): Uint8Array {
  const enc = new TextEncoder().encode(s);
  return concat(encodeVarint(enc.length), enc);
}
export function decodeString(bytes: Uint8Array, off: number): [string, number] {
  const [len, o] = decodeVarint(bytes, off);
  const end = o + len;
  if (end > bytes.length) throw new Error('postcard string truncated');
  return [new TextDecoder().decode(bytes.subarray(o, end)), end];
}

// --- typed arrays (Vec<u8>, Vec<f32>, Vec<f64>) ------------------------

export function encodeBytes(b: Uint8Array): Uint8Array {
  return concat(encodeVarint(b.length), b);
}
export function decodeBytes(bytes: Uint8Array, off: number): [Uint8Array, number] {
  const [len, o] = decodeVarint(bytes, off);
  const end = o + len;
  if (end > bytes.length) throw new Error('postcard bytes truncated');
  return [bytes.subarray(o, end), end];
}

export function encodeF32Vec(vec: Float32Array): Uint8Array {
  const v = encodeVarint(vec.length);
  const out = new Uint8Array(v.length + vec.length * 4);
  out.set(v, 0);
  const dv = new DataView(out.buffer, out.byteOffset + v.length, vec.length * 4);
  for (let i = 0; i < vec.length; i++) dv.setFloat32(i * 4, vec[i], true);
  return out;
}
export function decodeF32Vec(bytes: Uint8Array, off: number): [Float32Array, number] {
  const [n, o] = decodeVarint(bytes, off);
  const end = o + n * 4;
  if (end > bytes.length) throw new Error('postcard f32 vec truncated');
  const out = new Float32Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + o, n * 4);
  for (let i = 0; i < n; i++) out[i] = dv.getFloat32(i * 4, true);
  return [out, end];
}

export function encodeF64Vec(vec: Float64Array): Uint8Array {
  const v = encodeVarint(vec.length);
  const out = new Uint8Array(v.length + vec.length * 8);
  out.set(v, 0);
  const dv = new DataView(out.buffer, out.byteOffset + v.length, vec.length * 8);
  for (let i = 0; i < vec.length; i++) dv.setFloat64(i * 8, vec[i], true);
  return out;
}
export function decodeF64Vec(bytes: Uint8Array, off: number): [Float64Array, number] {
  const [n, o] = decodeVarint(bytes, off);
  const end = o + n * 8;
  if (end > bytes.length) throw new Error('postcard f64 vec truncated');
  const out = new Float64Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + o, n * 8);
  for (let i = 0; i < n; i++) out[i] = dv.getFloat64(i * 8, true);
  return [out, end];
}
