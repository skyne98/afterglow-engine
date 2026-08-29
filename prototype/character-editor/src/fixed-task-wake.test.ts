import { describe, expect, test } from 'bun:test';
import { FixedTaskWake } from './fixed-task-wake.ts';

describe('FixedTaskWake', () => {
  test('combines pending wake requests', async () => {
    let calls = 0;
    let complete!: () => void;
    const done = new Promise<void>((resolve) => { complete = resolve; });
    const wake = new FixedTaskWake(() => {
      calls++;
      complete();
    });
    for (let i = 0; i < 100; i++) wake.schedule();
    expect(calls).toBe(0);
    expect(wake.isPending).toBe(true);
    await done;
    expect(calls).toBe(1);
    expect(wake.isPending).toBe(false);
    wake.close();
  });

  test('runs a long wake chain without timer delay', async () => {
    const count = 613;
    let calls = 0;
    let complete!: () => void;
    const done = new Promise<void>((resolve) => { complete = resolve; });
    let wake!: FixedTaskWake;
    wake = new FixedTaskWake(() => {
      calls++;
      if (calls < count) wake.schedule();
      else complete();
    });
    const started = performance.now();
    wake.schedule();
    await done;
    const elapsed = performance.now() - started;
    expect(calls).toBe(count);
    expect(elapsed).toBeLessThan(100);
    wake.close();
  });
});
