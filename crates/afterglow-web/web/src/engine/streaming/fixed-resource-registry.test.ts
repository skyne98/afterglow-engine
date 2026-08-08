import { describe, expect, test } from 'bun:test';
import { FixedResourceRegistry } from './fixed-resource-registry.ts';

describe('FixedResourceRegistry', () => {
  test('bounds ownership and rejects stale handles after slot reuse', () => {
    const registry = new FixedResourceRegistry<string>(2);
    const first = registry.acquire('first');
    const second = registry.acquire('second');
    expect(first).not.toBe(0);
    expect(second).not.toBe(0);
    expect(registry.acquire('overflow')).toBe(0);
    if (first === 0 || second === 0) throw new Error('unexpected capacity failure');
    expect(registry.get(first)).toBe('first');
    expect(registry.release(first)).toBe('first');
    expect(registry.get(first)).toBeNull();
    const replacement = registry.acquire('replacement');
    expect(replacement).not.toBe(first);
    if (replacement === 0) throw new Error('unexpected replacement failure');
    expect(registry.get(replacement)).toBe('replacement');
    expect(registry.size).toBe(2);
  });
});
