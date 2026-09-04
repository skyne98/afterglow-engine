import { describe, expect, test } from 'bun:test';
import { StrokeStabilizer } from './paint-stabilizer.ts';

function start(mode: 'off' | 'string' | 'average' | 'exponential' | 'inertia', amount = 20) {
  const filter = new StrokeStabilizer();
  filter.configure(mode, amount, true);
  filter.begin(0, 0, 0);
  expect(filter.sample(0, 0, 0)).toBe(true);
  return filter;
}

describe('StrokeStabilizer', () => {
  test('passes positions through when it is off', () => {
    const filter = start('off');
    expect(filter.sample(12, 7, 16)).toBe(true);
    expect([filter.x, filter.y]).toEqual([12, 7]);
  });

  test('moves only after the pulled string is tight', () => {
    const filter = start('string', 10);
    expect(filter.sample(6, 0, 16)).toBe(false);
    expect(filter.x).toBe(0);
    expect(filter.sample(15, 0, 32)).toBe(true);
    expect(filter.x).toBeCloseTo(5, 5);
    expect(filter.sample(15, 10, 48)).toBe(true);
    expect(Math.hypot(15 - filter.x, 10 - filter.y)).toBeCloseTo(10, 5);
  });

  test('uses a bounded moving average', () => {
    const filter = start('average', 1);
    filter.sample(10, 0, 16);
    expect(filter.x).toBeCloseTo(5, 5);
    filter.sample(20, 0, 32);
    expect(filter.x).toBeCloseTo(15, 5);
    for (let value = 21; value <= 200; value++) filter.sample(value, 0, value * 16);
    expect(filter.x).toBeCloseTo(199.5, 5);
  });

  test('gives recent samples more weight in exponential mode', () => {
    const average = start('average', 50);
    const exponential = start('exponential', 50);
    average.sample(100, 0, 16);
    exponential.sample(100, 0, 16);
    expect(exponential.x).toBeGreaterThan(0);
    expect(exponential.x).toBeLessThan(average.x);
    const first = exponential.x;
    exponential.sample(100, 0, 32);
    expect(exponential.x).toBeGreaterThan(first);
  });

  test('uses stable inertia for a delayed flowing position', () => {
    const filter = start('inertia', 50);
    filter.sample(100, 0, 16);
    expect(filter.x).toBeGreaterThan(0);
    expect(filter.x).toBeLessThan(100);
    const first = filter.x;
    filter.sample(100, 0, 32);
    expect(filter.x).toBeGreaterThan(first);

    const fast = start('inertia', 1);
    for (let sample = 1; sample <= 200; sample++) fast.sample(100, 0, sample * 33);
    expect(Number.isFinite(fast.x)).toBe(true);
    expect(Math.abs(fast.x)).toBeLessThan(200);

    const dense = start('inertia', 1);
    const sparse = start('inertia', 1);
    for (let time = 1; time <= 96; time++) dense.sample(100, 0, time);
    for (let time = 8; time <= 96; time += 8) sparse.sample(100, 0, time);
    expect(Math.abs(dense.x - sparse.x)).toBeLessThan(2);
  });

  test('catches average modes up to the last pen position', () => {
    const filter = start('average', 80);
    filter.sample(100, 0, 16);
    let steps = 0;
    while (filter.stepCatchUp() && steps < 100) steps++;
    expect(steps).toBeLessThan(100);
    expect(filter.x).toBeCloseTo(100, 5);
    filter.sample(100, 0, 32);
    expect(filter.x).toBeCloseTo(100, 5);
    filter.end();
    expect(filter.stepCatchUp()).toBe(false);
  });

  test('finishes catch-up at the last pen position', () => {
    const filter = start('exponential', 80);
    filter.sample(100, 25, 16);
    expect(filter.finishCatchUp()).toBe(true);
    expect([filter.x, filter.y]).toEqual([100, 25]);
    expect(filter.finishCatchUp()).toBe(false);
  });

  test('limits settings and catch-up modes', () => {
    const filter = new StrokeStabilizer();
    filter.configure('string', 1000, true);
    expect(filter.amount).toBe(100);
    filter.begin(0, 0, 0);
    expect(filter.canCatchUp()).toBe(false);
    filter.configure('average', -5, false);
    expect(filter.amount).toBe(1);
    expect(filter.canCatchUp()).toBe(false);
  });
});
