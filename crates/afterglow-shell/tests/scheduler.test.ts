import { expect, test } from 'bun:test';

test('scheduler exposes only an asynchronous yield continuation', async () => {
  let defers = 0;
  (globalThis as any).__afterglowSchedulerNative = {
    defer: (resolve: () => void) => {
      defers++;
      setTimeout(resolve, 0);
    },
  };
  await import('../scheduler.ts');

  expect(Object.prototype.toString.call((globalThis as any).scheduler)).toBe('[object Scheduler]');
  expect(typeof (globalThis as any).scheduler.yield).toBe('function');
  expect((globalThis as any).scheduler.yield.name).toBe('yield');
  expect((globalThis as any).scheduler.postTask).toBeUndefined();
  expect((globalThis as any).TaskController).toBeUndefined();
  expect(() => new (globalThis as any).Scheduler()).toThrow(TypeError);
  const detached = (globalThis as any).scheduler.yield;
  expect(() => detached()).toThrow(TypeError);

  let resolved = false;
  const continuation = (globalThis as any).scheduler.yield().then(() => { resolved = true; });
  await Promise.resolve();
  expect(resolved).toBe(false);
  await continuation;
  expect(resolved).toBe(true);
  expect(defers).toBe(1);
});
