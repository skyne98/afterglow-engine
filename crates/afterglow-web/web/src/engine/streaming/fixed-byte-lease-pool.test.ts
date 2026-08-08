import { describe, expect, test } from 'bun:test';
import { FixedByteLeasePool } from './fixed-byte-lease-pool.ts';

describe('FixedByteLeasePool', () => {
  test('reuses stable buffers with deterministic overflow and stale release rejection', () => {
    const pool = new FixedByteLeasePool(2, 16);
    const a = pool.tryAcquire()!, b = pool.tryAcquire()!;
    expect(pool.tryAcquire()).toBeNull();
    expect(pool.overflows).toBe(1);
    expect(pool.highWater).toBe(2);
    expect(a.release()).toBe(true);
    expect(a.release()).toBe(false);
    const c = pool.tryAcquire()!;
    expect(c.bytes).toBe(a.bytes);
    expect(pool.active).toBe(2);
    b.release();
    c.release();
    expect(pool.active).toBe(0);
  });
});
