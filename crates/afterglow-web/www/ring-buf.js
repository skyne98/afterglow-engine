// afterglow-web ring buffer wrap-safe read/write helpers.
//
// These are pure functions operating on a Uint8Array view of ring data of
// `cap` bytes. All offsets wrap at `cap` so that frame accesses that straddle
// the circular boundary work correctly. Extracted from worker.js so they can
// be imported and tested in Node without a browser or Worker context.
//
// Usage (browser / worker):
//   import { rdU32, wrU32, xfer, U32, HEADER } from './ring-buf.js';
//
// No dependencies. Not specific to any protocol framing.

/** Size of a u32 in bytes. */
export const U32 = 4;

/** Ring header size: capacity + write_idx + read_idx. */
export const HEADER = 12;

// Read 4 LE bytes at `off` (wrap-safe across the `cap`-byte data area).
export function rdU32(u8, off, cap) {
  return (u8[off % cap]
    | (u8[(off + 1) % cap] << 8)
    | (u8[(off + 2) % cap] << 16)
    | (u8[(off + 3) % cap] << 24)) >>> 0;
}

// Write 4 LE bytes of `val` at `off` (wrap-safe).
export function wrU32(u8, off, cap, val) {
  u8[off % cap] = val & 0xff;
  u8[(off + 1) % cap] = (val >>> 8) & 0xff;
  u8[(off + 2) % cap] = (val >>> 16) & 0xff;
  u8[(off + 3) % cap] = (val >>> 24) & 0xff;
}

// Copy `len` bytes between ring data `u8` (at `off`, wrapping at `cap`) and a
// flat `buf`. mode 'rd': ring->buf, 'wr': buf->ring.
export function xfer(u8, off, cap, buf, len, mode) {
  const o = off % cap, first = Math.min(len, cap - o);
  if (mode === 'rd') {
    buf.set(u8.subarray(o, o + first), 0);
    if (first < len) buf.set(u8.subarray(0, len - first), first);
  } else {
    u8.set(buf.subarray(0, first), o);
    if (first < len) u8.set(buf.subarray(first), 0);
  }
}
