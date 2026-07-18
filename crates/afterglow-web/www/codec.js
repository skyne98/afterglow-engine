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
function encodeZigzag(n) {
  const zz = n < 0 ? -n * 2 - 1 : n * 2;
  return encodeVarint(zz);
}
function decodeZigzag(bytes, off) {
  const [zz, o] = decodeVarint(bytes, off);
  return [zz & 1 ? -(zz + 1) / 2 : zz / 2, o];
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
function encodeU8(n) {
  return new Uint8Array([n & 255]);
}
function decodeU8(bytes, off) {
  if (off >= bytes.length)
    throw new Error("postcard u8 truncated");
  return [bytes[off], off + 1];
}
function encodeU16(n) {
  return new Uint8Array(encodeVarint(n));
}
function decodeU16(bytes, off) {
  return decodeVarint(bytes, off);
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
function encodeI8(n) {
  return new Uint8Array([n & 255]);
}
function decodeI8(bytes, off) {
  if (off >= bytes.length)
    throw new Error("postcard i8 truncated");
  let v = bytes[off];
  if (v > 127)
    v -= 256;
  return [v, off + 1];
}
function encodeI16(n) {
  return new Uint8Array(encodeZigzag(n));
}
function decodeI16(bytes, off) {
  return decodeZigzag(bytes, off);
}
function encodeI32(n) {
  return new Uint8Array(encodeZigzag(n));
}
function decodeI32(bytes, off) {
  return decodeZigzag(bytes, off);
}
function encodeI64(n) {
  return new Uint8Array(encodeZigzag(n));
}
function decodeI64(bytes, off) {
  return decodeZigzag(bytes, off);
}
function encodeF32(x) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return b;
}
function decodeF32(bytes, off) {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 4).getFloat32(0, true), off + 4];
}
function encodeF64(x) {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setFloat64(0, x, true);
  return b;
}
function decodeF64(bytes, off) {
  return [new DataView(bytes.buffer, bytes.byteOffset + off, 8).getFloat64(0, true), off + 8];
}
function encodeBool(b) {
  return new Uint8Array([b ? 1 : 0]);
}
function decodeBool(bytes, off) {
  if (off >= bytes.length)
    throw new Error("postcard bool truncated");
  return [bytes[off] !== 0, off + 1];
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
function encodeF32Vec(vec) {
  const v = encodeVarint(vec.length);
  const out = new Uint8Array(v.length + vec.length * 4);
  out.set(v, 0);
  const dv = new DataView(out.buffer, out.byteOffset + v.length, vec.length * 4);
  for (let i = 0;i < vec.length; i++)
    dv.setFloat32(i * 4, vec[i], true);
  return out;
}
function decodeF32Vec(bytes, off) {
  const [n, o] = decodeVarint(bytes, off);
  const end = o + n * 4;
  if (end > bytes.length)
    throw new Error("postcard f32 vec truncated");
  const out = new Float32Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + o, n * 4);
  for (let i = 0;i < n; i++)
    out[i] = dv.getFloat32(i * 4, true);
  return [out, end];
}
function encodeF64Vec(vec) {
  const v = encodeVarint(vec.length);
  const out = new Uint8Array(v.length + vec.length * 8);
  out.set(v, 0);
  const dv = new DataView(out.buffer, out.byteOffset + v.length, vec.length * 8);
  for (let i = 0;i < vec.length; i++)
    dv.setFloat64(i * 8, vec[i], true);
  return out;
}
function decodeF64Vec(bytes, off) {
  const [n, o] = decodeVarint(bytes, off);
  const end = o + n * 8;
  if (end > bytes.length)
    throw new Error("postcard f64 vec truncated");
  const out = new Float64Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + o, n * 8);
  for (let i = 0;i < n; i++)
    out[i] = dv.getFloat64(i * 8, true);
  return [out, end];
}
function encodeU32Vec(vec) {
  const parts = [encodeVarint(vec.length)];
  for (let i = 0;i < vec.length; i++)
    parts.push(encodeVarint(vec[i]));
  return concat(...parts);
}
function decodeU32Vec(bytes, off) {
  const [n, o] = decodeVarint(bytes, off);
  const out = new Uint32Array(n);
  let pos = o;
  for (let i = 0;i < n; i++) {
    const [val, next] = decodeVarint(bytes, pos);
    out[i] = val;
    pos = next;
  }
  return [out, pos];
}
function unwrapResponse(bytes) {
  const [variant, off] = decodeVarint(bytes, 0);
  if (variant === 0) {
    const [plen, poff] = decodeVarint(bytes, off);
    if (poff + plen > bytes.length)
      throw new Error("RPC response truncated");
    return bytes.subarray(poff, poff + plen);
  }
  const [method, moff] = decodeVarint(bytes, off);
  const [mlen, eoff] = decodeVarint(bytes, moff);
  if (eoff + mlen > bytes.length)
    throw new Error("RPC error truncated");
  const msg = new TextDecoder().decode(Uint8Array.from(bytes.subarray(eoff, eoff + mlen)));
  throw new Error(`RPC ${variant === 1 ? "server" : "decode"} error (method ${method}): ${msg}`);
}
export {
  unwrapResponse,
  encodeZigzag,
  encodeVarint,
  encodeU8,
  encodeU64,
  encodeU32Vec,
  encodeU32,
  encodeU16,
  encodeString,
  encodeI8,
  encodeI64,
  encodeI32,
  encodeI16,
  encodeF64Vec,
  encodeF64,
  encodeF32Vec,
  encodeF32,
  encodeBytes,
  encodeBool,
  decodeZigzag,
  decodeVarint,
  decodeU8,
  decodeU64,
  decodeU32Vec,
  decodeU32,
  decodeU16,
  decodeString,
  decodeI8,
  decodeI64,
  decodeI32,
  decodeI16,
  decodeF64Vec,
  decodeF64,
  decodeF32Vec,
  decodeF32,
  decodeBytes,
  decodeBool,
  concat
};
