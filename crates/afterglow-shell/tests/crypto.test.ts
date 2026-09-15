import { expect, test } from 'bun:test';
import { createCrypto } from '../crypto.ts';

test('random bytes use only the selected integer view', () => {
  const crypto = createCrypto((bytes: Uint8Array) => bytes.fill(0xab));
  const backing = new Uint8Array(8);
  const view = new Uint16Array(backing.buffer, 2, 2);
  expect(crypto.getRandomValues(view)).toBe(view);
  expect([...backing]).toEqual([0, 0, 171, 171, 171, 171, 0, 0]);
  expect(() => crypto.getRandomValues(new Float32Array(1))).toThrow();
  expect(() => crypto.getRandomValues(new DataView(backing.buffer))).toThrow();
  expect(() => crypto.getRandomValues({ [Symbol.toStringTag]: 'Uint8Array' })).toThrow();
  expect(() => crypto.getRandomValues(new Uint8Array(65_537))).toThrow();
  expect(crypto.getRandomValues(new Uint8Array(65_536)).length).toBe(65_536);
});

test('UUID has version 4 and RFC variant bits', () => {
  const crypto = createCrypto((bytes: Uint8Array) => bytes.fill(0xff));
  expect(crypto.randomUUID()).toBe('ffffffff-ffff-4fff-bfff-ffffffffffff');
  const failed = createCrypto(() => { throw new Error('OS random failure'); });
  expect(() => failed.randomUUID()).toThrow('OS random failure');
});
