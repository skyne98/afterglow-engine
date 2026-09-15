import { expect, test } from 'bun:test';
import { installRingService } from './ring-service.ts';
import { HEADER, rdU32, wrU32, xfer } from './ring-buf.ts';

function setup(handler: (method: number, args: Uint8Array) => Promise<Uint8Array>) {
  const size = 1024, capacity = size - HEADER;
  const memory = new SharedArrayBuffer(size * 2);
  const words = new Int32Array(memory);
  words[0] = words[size / 4] = capacity;
  const messages: unknown[] = [];
  let output = () => {};
  const host = { onmessage: null as ((event: MessageEvent) => void) | null,
    postMessage(value: unknown) { messages.push(value); output(); } };
  const original = Object.getOwnPropertyDescriptor(globalThis, 'self');
  try {
    Object.defineProperty(globalThis, 'self', { value: host, configurable: true });
    installRingService(() => handler);
  } finally {
    if (original) Object.defineProperty(globalThis, 'self', original);
    else Reflect.deleteProperty(globalThis, 'self');
  }
  const send = (data: unknown) => host.onmessage!({ data } as MessageEvent);
  function next(): Promise<void> { return new Promise(resolve => { output = resolve; }); }
  function response(): number[] {
    const data = new Uint8Array(memory, size + HEADER, capacity);
    const index = words[size / 4 + 2]! >>> 0;
    const count = rdU32(data, index, capacity);
    const bytes = new Uint8Array(count);
    xfer(data, index + 4, capacity, bytes, count, 'rd');
    words[size / 4 + 2] = index + count + 4;
    return Array.from(bytes);
  }
  return { size, capacity, memory, words, messages, send, next, response };
}
test('shared TypeScript RPC host preserves wrap, method, arguments, response and wake order', async () => {
  const seen: number[][] = [];
  const s = setup(async (method, args) => { seen.push([method, ...args]); return args; });
  s.send({ type: 'init', sab: s.memory, reqBase: 0, respBase: s.size, bufSize: s.size });
  expect(s.messages).toEqual([{ type: 'ready' }]);
  s.words[1] = s.words[2] = s.capacity - 2;
  const data = new Uint8Array(s.memory, HEADER, s.capacity);
  wrU32(data, s.capacity - 2, s.capacity, 6);
  wrU32(data, s.capacity + 2, s.capacity, 5);
  xfer(data, s.capacity + 6, s.capacity, new Uint8Array([7,8]), 2, 'wr');
  s.words[1] = s.capacity + 8;
  const done = s.next(); s.send({ type: 'run' }); await done;
  expect(seen).toEqual([[5,7,8]]);
  expect(s.response()).toEqual([0,2,7,8]);
  expect(s.words[2]).toBe(s.words[1]);
  expect(s.messages[1]).toBe('wake');
  const invalid = s.next(); s.words[1] = s.words[2]! + 1; s.send('wake'); await invalid;
  expect(s.messages[2]).toEqual({ type: 'error', message: 'Corrupt RPC request ring' });
  s.send('wake'); expect(seen.length).toBe(1);
});
test('invalid or overlapping storage fails before request admission', () => {
  let created = 0;
  const s = setup(async () => { created++; return new Uint8Array(); });
  s.send({ type: 'init', sab: s.memory, reqBase: 0, respBase: 0, bufSize: s.size });
  expect(s.messages).toEqual([{ type: 'error', message: 'Invalid RPC ring storage' }]);
  expect(created).toBe(0);
});
test('service failures use the existing postcard response envelope', async () => {
  const s = setup(async () => { throw new Error('bad'); });
  s.send({ type: 'init', sab: s.memory, reqBase: 0, respBase: s.size, bufSize: s.size });
  const data = new Uint8Array(s.memory, HEADER, s.capacity);
  wrU32(data, 0, s.capacity, 4); wrU32(data, 4, s.capacity, 2); s.words[1] = 8;
  const done = s.next(); s.send({ type: 'run' }); await done;
  expect(s.response()).toEqual([1,2,3,98,97,100]);
});
