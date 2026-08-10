import { describe, expect, test } from 'bun:test';
import { MotionQueue, spline4p } from './paint-input.ts';

describe('MotionQueue', () => {
  test('keeps primitive samples in order', () => {
    const queue = new MotionQueue(4);
    queue.push(10, 1, 2, 0.5, 0, 0, 1, 0, 0.5);
    queue.push(12, 3, 4, 0.6, 0.1, 0.2, 1, 0, 0.5);
    const times: number[] = [];
    queue.drain((time, x, y) => { times.push(time, x, y); });
    expect(times).toEqual([10, 1, 2, 12, 3, 4]);
    expect(queue.length).toBe(0);
  });

  test('corrects time reversal and bounds storage', () => {
    const queue = new MotionQueue(2);
    queue.push(10, 1, 0, 0, 0, 0, 1, 0, 0);
    queue.push(9, 2, 0, 0, 0, 0, 1, 0, 0);
    queue.push(8, 3, 0, 0, 0, 0, 1, 0, 0);
    const xs: number[] = [];
    const times: number[] = [];
    queue.drain((time, x) => { times.push(time); xs.push(x); });
    expect(queue.overflowCount).toBe(1);
    expect(xs).toEqual([2, 3]);
    expect(times).toEqual([10, 10]);
  });

  test('bounded drain keeps the remaining samples when the budget is spent', () => {
    const queue = new MotionQueue(8);
    for (let i = 1; i <= 6; i++) {
      queue.push(i, i, 0, 0.5, 0, 0, 1, 0, 0.5, true, true, true);
    }
    const flushed: number[] = [];
    queue.drainInterpolatedBounded((_time, x) => { flushed.push(x); }, 0);
    expect(flushed.length).toBeGreaterThan(0);
    expect(flushed.length).toBeLessThan(6);
    expect(queue.length).toBe(6 - flushed.length);
    queue.drainInterpolatedBounded((_time, x) => { flushed.push(x); }, 1000);
    expect(flushed).toHaveLength(6);
    expect(queue.length).toBe(0);
  });

  test('bounded drain with a large budget flushes everything in order', () => {
    const queue = new MotionQueue(8);
    for (let i = 1; i <= 4; i++) {
      queue.push(i, i, 0, 0.5, 0, 0, 1, 0, 0.5, true, true, true);
    }
    const xs: number[] = [];
    queue.drainInterpolatedBounded((_time, x) => { xs.push(x); }, 1000);
    expect(xs).toEqual([1, 2, 3, 4]);
    expect(queue.length).toBe(0);
  });

  test('interpolates missing pressure and tilt values', () => {
    const queue = new MotionQueue(4);
    queue.push(0, 0, 0, 0, 0, 0, 1, 0, 0.5, true, true, true);
    queue.push(10, 1, 0, 0, 0, 0, 1, 0, 0.5, false, false, true);
    queue.push(20, 2, 0, 1, 1, 1, 1, 0, 0.5, true, true, true);
    const pressure: number[] = [];
    const tilt: number[] = [];
    queue.drainInterpolated((_time, _x, _y, value, xtilt) => {
      pressure.push(value);
      tilt.push(xtilt);
    });
    expect(pressure[1]).toBeCloseTo(0.5, 5);
    expect(tilt[1]).toBeCloseTo(0.5, 5);
    expect(queue.length).toBe(0);
  });
});

describe('spline4p', () => {
  test('returns control points at the interval ends', () => {
    expect(spline4p(0, 0, 2, 4, 6)).toBe(2);
    expect(spline4p(1, 0, 2, 4, 6)).toBe(4);
  });
});
