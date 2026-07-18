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
export {
  xfer,
  wrU32,
  rdU32,
  U32,
  HEADER
};
