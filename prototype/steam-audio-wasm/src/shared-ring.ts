import { HEADER, U32, rdU32, wrU32, xfer } from '../../../crates/afterglow-web/web/src/workers/ring-buf.ts';

export const RING_CAPACITY = 4096;
export const RING_BYTES = HEADER + RING_CAPACITY;

export function initializeRing(memory: SharedArrayBuffer, base: number): void {
  const header = new Uint32Array(memory, base, 3);
  header[0] = RING_CAPACITY;
  header[1] = 0;
  header[2] = 0;
}

export function writeFrame(memory: SharedArrayBuffer, base: number, payload: Uint8Array): void {
  const writeIndex = new Int32Array(memory, base + U32, 1);
  const readIndex = new Int32Array(memory, base + 2 * U32, 1);
  const data = new Uint8Array(memory, base + HEADER, RING_CAPACITY);
  const write = Atomics.load(writeIndex, 0) >>> 0;
  const read = Atomics.load(readIndex, 0) >>> 0;
  const used = (write - read) >>> 0;
  const frameBytes = U32 + payload.byteLength;
  if (frameBytes > RING_CAPACITY - used) throw new Error('prototype ring full');
  const offset = write % RING_CAPACITY;
  wrU32(data, offset, RING_CAPACITY, payload.byteLength);
  xfer(data, offset + U32, RING_CAPACITY, payload, payload.byteLength, 'wr');
  Atomics.store(writeIndex, 0, (write + frameBytes) >>> 0);
}

export function readFrame(memory: SharedArrayBuffer, base: number, output: Uint8Array): number {
  const writeIndex = new Int32Array(memory, base + U32, 1);
  const readIndex = new Int32Array(memory, base + 2 * U32, 1);
  const data = new Uint8Array(memory, base + HEADER, RING_CAPACITY);
  const write = Atomics.load(writeIndex, 0) >>> 0;
  const read = Atomics.load(readIndex, 0) >>> 0;
  const used = (write - read) >>> 0;
  if (used === 0) return 0;
  if (used < U32 || used > RING_CAPACITY) throw new Error('prototype ring corrupt');
  const offset = read % RING_CAPACITY;
  const payloadBytes = rdU32(data, offset, RING_CAPACITY);
  const frameBytes = U32 + payloadBytes;
  if (frameBytes > used || payloadBytes > output.byteLength)
    throw new Error('prototype ring frame invalid');
  xfer(data, offset + U32, RING_CAPACITY, output, payloadBytes, 'rd');
  Atomics.store(readIndex, 0, (read + frameBytes) >>> 0);
  return payloadBytes;
}
