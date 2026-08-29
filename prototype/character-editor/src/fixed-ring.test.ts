import { describe, expect, test } from 'bun:test';
import { FixedRing } from './fixed-ring.ts';

describe('FixedRing', () => {
  test('keeps item order after a wrap', () => {
    const ring = new FixedRing<number>(3);
    expect(ring.push(1)).toBe(true);
    expect(ring.push(2)).toBe(true);
    expect(ring.shift()).toBe(1);
    expect(ring.push(3)).toBe(true);
    expect(ring.push(4)).toBe(true);
    expect(ring.length).toBe(3);
    expect(ring.peek()).toBe(2);
    expect([ring.shift(), ring.shift(), ring.shift()]).toEqual([2, 3, 4]);
    expect(ring.length).toBe(0);
  });

  test('rejects a new item when it is full', () => {
    const ring = new FixedRing<string>(2);
    expect(ring.push('begin')).toBe(true);
    expect(ring.push('commit')).toBe(true);
    expect(ring.push('next')).toBe(false);
    expect([ring.shift(), ring.shift()]).toEqual(['begin', 'commit']);
  });

  test('keeps all stroke boundaries in a backlog', () => {
    const ring = new FixedRing<string>(1200);
    for (let stroke = 0; stroke < 100; stroke++) {
      expect(ring.push(`begin:${stroke}`)).toBe(true);
      for (let sample = 0; sample < 10; sample++) {
        expect(ring.push(`sample:${stroke}:${sample}`)).toBe(true);
      }
      expect(ring.push(`commit:${stroke}`)).toBe(true);
    }
    for (let stroke = 0; stroke < 100; stroke++) {
      expect(ring.shift()).toBe(`begin:${stroke}`);
      for (let sample = 0; sample < 10; sample++) {
        expect(ring.shift()).toBe(`sample:${stroke}:${sample}`);
      }
      expect(ring.shift()).toBe(`commit:${stroke}`);
    }
    expect(ring.length).toBe(0);
  });

  test('clears all item references', () => {
    const ring = new FixedRing<object>(2);
    ring.push({});
    ring.push({});
    ring.clear();
    expect(ring.length).toBe(0);
    expect(ring.shift()).toBeUndefined();
    expect(ring.push({ value: 1 })).toBe(true);
  });
});
