import { describe, expect, test } from 'bun:test';
import { RING_BYTES, initializeRing, readFrame, writeFrame } from './shared-ring.ts';

describe('Steam Audio prototype shared rings', () => {
  test('round trips independent request and response frames', () => {
    const memory = new SharedArrayBuffer(RING_BYTES * 2);
    initializeRing(memory, 0);
    initializeRing(memory, RING_BYTES);
    const request = new Uint8Array([1, 2, 3, 4]);
    const response = new Uint8Array([9, 8, 7]);
    writeFrame(memory, 0, request);
    writeFrame(memory, RING_BYTES, response);
    const requestOut = new Uint8Array(4);
    const responseOut = new Uint8Array(3);
    expect(readFrame(memory, 0, requestOut)).toBe(4);
    expect(readFrame(memory, RING_BYTES, responseOut)).toBe(3);
    expect([...requestOut]).toEqual([...request]);
    expect([...responseOut]).toEqual([...response]);
    expect(readFrame(memory, 0, requestOut)).toBe(0);
  });

  test('fails deterministically instead of overwriting a full ring', () => {
    const memory = new SharedArrayBuffer(RING_BYTES);
    initializeRing(memory, 0);
    expect(() => writeFrame(memory, 0, new Uint8Array(4093))).toThrow('prototype ring full');
  });
});
