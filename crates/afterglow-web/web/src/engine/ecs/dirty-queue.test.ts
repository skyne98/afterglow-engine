import { describe, expect, test } from 'bun:test';
import { DirtyMarkStatus, EntityDirtyQueue } from './dirty-queue.ts';
import { RenderDirty } from '../core/types.ts';

describe('EntityDirtyQueue bounded ring', () => {
  test('retains a deferred suffix and wraps without reallocating', () => {
    const queue = new EntityDirtyQueue(3);
    expect(queue.mark(0, RenderDirty.Transform)).toBe(DirtyMarkStatus.Accepted);
    expect(queue.mark(1, RenderDirty.Appearance)).toBe(DirtyMarkStatus.Accepted);
    expect(queue.mark(1, RenderDirty.Transform)).toBe(DirtyMarkStatus.AlreadyQueued);
    expect(queue.mark(2, RenderDirty.WorldOnly)).toBe(DirtyMarkStatus.Accepted);
    expect(queue.mark(3, RenderDirty.Transform)).toBe(DirtyMarkStatus.CapacityExceeded);
    expect(queue.clearPrefix(2)).toBe(2);
    expect(queue.count).toBe(1);
    expect(queue.entityAt(0)).toBe(2);
    expect(queue.mark(0, RenderDirty.Appearance)).toBe(DirtyMarkStatus.Accepted);
    expect(queue.entityAt(1)).toBe(0);
    expect(queue.flags[1]).toBe(RenderDirty.None);
    expect(queue.flags[2]).toBe(RenderDirty.WorldOnly);
    expect(queue.highWater).toBe(3);
    expect(queue.overflows).toBe(1);
    queue.clear();
    expect(queue.count).toBe(0);
  });
});
