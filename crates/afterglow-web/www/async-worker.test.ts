import { describe, expect, test } from 'bun:test';
import { AsyncWorker } from './async-worker.ts';

describe('AsyncWorker fixed task slots', () => {
  test('admits a fixed number of calls and reports deterministic exhaustion', async () => {
    const memory = new WebAssembly.Memory({ initial: 1 });
    const worker = new AsyncWorker({
      memory,
      afterglow_wasm_input_ptr: () => 0,
      afterglow_wasm_input_size: () => 4096,
      afterglow_wasm_serve_async: () => 0,
    });
    worker._memory = memory;
    worker._schedulePump = () => {};

    const pending: Promise<Uint8Array>[] = [];
    for (let index = 0; index < 256; index++) pending.push(worker.call(1, new Uint8Array()));
    expect(worker._pendingCallCount).toBe(256);
    await expect(worker.call(1, new Uint8Array())).rejects.toThrow('fixed task capacity exhausted');

    // Keep deliberately unresolved promises reachable for the duration of the test.
    expect(pending).toHaveLength(256);
  });

  test('drains a bounded completion prefix without materializing a result array', async () => {
    const memory = new WebAssembly.Memory({ initial: 1 });
    const queue = [
      { taskId: 1, value: 41 },
      { taskId: 2, value: 42 },
    ];
    const worker = new AsyncWorker({
      memory,
      afterglow_wasm_input_ptr: () => 0,
      afterglow_wasm_input_size: () => 1024,
      afterglow_wasm_serve_async: () => 0,
      afterglow_wasm_tick: () => {},
      afterglow_wasm_output_ptr: () => 2048,
      afterglow_wasm_output_size: () => 1024,
      afterglow_wasm_drain_completion: (ptr: number) => {
        const completion = queue.shift();
        if (!completion) return -1;
        const view = new DataView(memory.buffer, ptr, 11);
        view.setBigUint64(0, BigInt(completion.taskId), true);
        view.setUint8(8, 0); // Response::Ok
        view.setUint8(9, 1); // payload length
        view.setUint8(10, completion.value);
        return 11;
      },
    });
    worker._memory = memory;
    worker._schedulePump = () => {};
    const first = worker.call(1, new Uint8Array());
    const second = worker.call(1, new Uint8Array());
    let secondResolved = false;
    void second.then(() => { secondResolved = true; });
    expect(worker.poll(1)).toBe(1);
    expect((await first)[0]).toBe(41);
    expect(secondResolved).toBe(false);
    expect(worker._lastPollCompletions).toBe(1);
    expect(worker._completionLimitHits).toBe(1);
    expect(worker.poll(1)).toBe(1);
    expect((await second)[0]).toBe(42);
    expect(worker._totalCompletions).toBe(2);
  });

  test('uses fixed fetch slots with bounded probing and reuse', () => {
    const memory = new WebAssembly.Memory({ initial: 1 });
    const worker = new AsyncWorker({ memory });
    const ids: number[] = [];
    for (let index = 0; index < 256; index++) ids.push(worker._registerFetch({ index }));
    expect(ids.every(id => id > 0)).toBe(true);
    expect(worker._registerFetch({ overflow: true })).toBe(0);
    worker._releaseFetch(ids[17]);
    const reused = worker._registerFetch({ reused: true });
    expect(reused).toBeGreaterThan(0);
    expect(worker._getFetch(reused)).toEqual({ reused: true });
    expect(worker._pendingFetchCount).toBe(256);
  });
});
