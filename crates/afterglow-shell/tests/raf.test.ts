import { expect, test } from 'bun:test';

test('native rAF queue is bounded, ordered, cancelable, and frame-delayed', async () => {
  const reports: unknown[] = [];
  let requested = 0;
  let emptied = 0;
  let overflows = 0;
  let drainedCallbacks = 0;
  (globalThis as any).__afterglowAnimationFrameNative = {
    requested: () => { requested++; },
    empty: () => { emptied++; },
    overflow: () => { overflows++; },
    drained: (callbacks: number) => { drainedCallbacks += callbacks; },
    report: (error: unknown) => { reports.push(error); },
  };
  await import('../raf.ts');

  const calls: string[] = [];
  requestAnimationFrame((timestamp) => {
    calls.push(`a:${timestamp}`);
    requestAnimationFrame((nextTimestamp) => calls.push(`c:${nextTimestamp}`));
  });
  requestAnimationFrame((timestamp) => calls.push(`b:${timestamp}`));
  (globalThis as any).__runNativeAnimationFrames(10);
  expect(calls).toEqual(['a:10', 'b:10']);
  (globalThis as any).__runNativeAnimationFrames(20);
  expect(calls).toEqual(['a:10', 'b:10', 'c:20']);

  const canceled = requestAnimationFrame(() => calls.push('canceled'));
  cancelAnimationFrame(canceled);
  (globalThis as any).__runNativeAnimationFrames(30);
  expect(calls).not.toContain('canceled');

  requestAnimationFrame(() => { throw new Error('first callback failed'); });
  requestAnimationFrame(() => calls.push('after-error'));
  (globalThis as any).__runNativeAnimationFrames(40);
  expect(calls.at(-1)).toBe('after-error');
  expect((reports[0] as Error).message).toBe('first callback failed');

  const noop = () => {};
  for (let index = 0; index < 1024; index++) requestAnimationFrame(noop);
  expect(() => requestAnimationFrame(noop)).toThrow(RangeError);
  expect(overflows).toBe(1);
  expect((globalThis as any).__nativeAnimationFrameStats()).toEqual({
    capacity: 1024,
    pending: 1024,
    queued: 1024,
  });
  (globalThis as any).__runNativeAnimationFrames(50);
  expect((globalThis as any).__nativeAnimationFrameStats().pending).toBe(0);
  expect(requested).toBeGreaterThan(0);
  expect(emptied).toBeGreaterThan(0);
  expect(drainedCallbacks).toBeGreaterThanOrEqual(1029);
});
