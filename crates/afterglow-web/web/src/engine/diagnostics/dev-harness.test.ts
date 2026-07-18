import { describe, expect, test } from 'bun:test';
import { BootstrapGuard, FrameStepHarness } from './dev-harness.ts';

describe('BootstrapGuard', () => {
  test('rolls back in reverse and releases committed callbacks', async () => {
    const events: number[] = [];
    const guard = new BootstrapGuard(2);
    guard.defer(() => { events.push(1); });
    guard.defer(async () => { events.push(2); });
    await guard.rollback();
    expect(events).toEqual([2, 1]);
    guard.defer(() => { events.push(3); });
    guard.release();
    await guard.rollback();
    expect(events).toEqual([2, 1]);
  });
});

describe('FrameStepHarness', () => {
  test('resolves bounded slots without growing or splicing', async () => {
    const harness = new FrameStepHarness(2);
    let first = false, second = false;
    const firstPromise = harness.wait(10, 2).then(() => { first = true; });
    const secondPromise = harness.wait(10, 4).then(() => { second = true; });
    expect(() => harness.wait(10, 1)).toThrow('capacity exceeded');
    harness.poll(12);
    await firstPromise;
    expect(first).toBe(true);
    expect(second).toBe(false);
    harness.poll(14);
    await secondPromise;
    expect(second).toBe(true);
  });

  test('rejects invalid capacities and counts', () => {
    expect(() => new FrameStepHarness(0)).toThrow('capacity');
    const harness = new FrameStepHarness(1);
    expect(() => harness.wait(0, 0)).toThrow('count');
  });
});
