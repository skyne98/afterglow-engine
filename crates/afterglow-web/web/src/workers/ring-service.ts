import { concat, encodeBytes, encodeString, encodeVarint } from './codec.ts';
import { HEADER, U32, rdU32, wrU32, xfer } from './ring-buf.ts';

type Handler = (method: number, args: Uint8Array) => Promise<Uint8Array>;

/** Host a TypeScript service on the same rings and response envelope as RPC workers. */
export function installRingService(create: (configuration: unknown) => Handler): void {
  const host = self as unknown as { onmessage: ((event: MessageEvent) => void) | null; postMessage(value: unknown): void };
  let state: 'init' | 'ready' | 'running' | 'failed' = 'init';
  let memory: SharedArrayBuffer;
  let requestBase = 0, responseBase = 0, capacity = 0;
  let handler: Handler;
  let wakePending = false;
  let wakeResolve: (() => void) | null = null;
  function fail(error: unknown): void {
    state = 'failed';
    host.postMessage({ type: 'error', message: (error instanceof Error ? error.message : String(error)).slice(0, 512) });
  }
  host.onmessage = event => {
    const message = event.data;
    if (state === 'init' && message?.type === 'init') {
      try {
        const { sab, reqBase, respBase, bufSize } = message;
        if (!(sab instanceof SharedArrayBuffer) || !Number.isInteger(bufSize) || bufSize <= HEADER + 8
          || !Number.isInteger(reqBase) || !Number.isInteger(respBase) || reqBase < 0 || respBase < 0
          || reqBase % 4 !== 0 || respBase % 4 !== 0 || reqBase + bufSize > sab.byteLength
          || respBase + bufSize > sab.byteLength || Math.abs(reqBase - respBase) < bufSize) {
          throw new Error('Invalid RPC ring storage');
        }
        memory = sab; requestBase = reqBase; responseBase = respBase; capacity = bufSize - HEADER;
        if (Atomics.load(new Uint32Array(memory, requestBase, 1), 0) !== capacity
          || Atomics.load(new Uint32Array(memory, responseBase, 1), 0) !== capacity) {
          throw new Error('Invalid RPC ring capacity');
        }
        handler = create(message.workerInit);
        state = 'ready';
        host.postMessage({ type: 'ready' });
      } catch (error) { fail(error); }
      return;
    }
    if (state === 'ready' && message?.type === 'run') {
      state = 'running'; void run().catch(fail); return;
    }
    if (state === 'running' && message === 'wake') {
      if (wakeResolve) { const resolve = wakeResolve; wakeResolve = null; resolve(); }
      else wakePending = true;
    }
  };
  function wait(): Promise<void> {
    if (wakePending) { wakePending = false; return Promise.resolve(); }
    return new Promise(resolve => { wakeResolve = resolve; });
  }
  async function run(): Promise<void> {
    const requestWrite = new Int32Array(memory, requestBase + U32, 1);
    const requestRead = new Int32Array(memory, requestBase + 2 * U32, 1);
    const requestData = new Uint8Array(memory, requestBase + HEADER, capacity);
    const responseWrite = new Int32Array(memory, responseBase + U32, 1);
    const responseRead = new Int32Array(memory, responseBase + 2 * U32, 1);
    const responseData = new Uint8Array(memory, responseBase + HEADER, capacity);
    while (state === 'running') {
      const write = Atomics.load(requestWrite, 0) >>> 0;
      const read = Atomics.load(requestRead, 0) >>> 0;
      const used = (write - read) >>> 0;
      if (!used) { await wait(); continue; }
      if (used > capacity || used < U32) throw new Error('Corrupt RPC request ring');
      const offset = read % capacity;
      const length = rdU32(requestData, offset, capacity);
      const frame = U32 + length;
      if (length < U32 || frame > used || frame > capacity) throw new Error('Corrupt RPC request frame');
      const method = rdU32(requestData, offset + U32, capacity) >>> 0;
      const args = new Uint8Array(length - U32);
      xfer(requestData, offset + 2 * U32, capacity, args, args.byteLength, 'rd');
      Atomics.store(requestRead, 0, (read + frame) >>> 0);
      let response: Uint8Array;
      try { response = concat(encodeVarint(0), encodeBytes(await handler(method, args))); }
      catch (error) {
        const text = (error instanceof Error ? error.message : String(error)).slice(0, 512);
        response = concat(encodeVarint(1), encodeVarint(method), encodeString(text));
      }
      const responseFrame = U32 + response.byteLength;
      const responseIndex = Atomics.load(responseWrite, 0) >>> 0;
      const responseUsed = (responseIndex - (Atomics.load(responseRead, 0) >>> 0)) >>> 0;
      if (responseUsed > capacity || responseFrame > capacity - responseUsed) throw new Error('RPC response ring full');
      const responseOffset = responseIndex % capacity;
      wrU32(responseData, responseOffset, capacity, response.byteLength);
      xfer(responseData, responseOffset + U32, capacity, response, response.byteLength, 'wr');
      Atomics.store(responseWrite, 0, (responseIndex + responseFrame) >>> 0);
      host.postMessage('wake');
    }
  }
}
