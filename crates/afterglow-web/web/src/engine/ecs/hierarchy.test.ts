import { describe, expect, test } from 'bun:test';
import { HierarchyRebuildStatus, HierarchyState } from './hierarchy.ts';
import { NULL_ENTITY } from '../core/types.ts';

function finish(hierarchy: HierarchyState, operations = 2): number {
  let calls = 0;
  while (hierarchy.topologyDirty) {
    hierarchy.stepRebuild(operations);
    if (++calls > 100) throw new Error('hierarchy rebuild did not converge');
  }
  return calls;
}

describe('HierarchyState incremental rebuild', () => {
  test('builds a unique parent-before-child order in bounded slices', () => {
    const hierarchy = new HierarchyState(8);
    hierarchy.updateCachedParent(1, NULL_ENTITY, 4);
    hierarchy.updateCachedParent(2, NULL_ENTITY, 1);
    hierarchy.updateCachedParent(3, NULL_ENTITY, 4);
    expect(() => hierarchy.stepRebuild(0)).toThrow('operation limit');
    expect(hierarchy.stepRebuild(2)).toBe(HierarchyRebuildStatus.InProgress);
    expect(finish(hierarchy, 2)).toBeGreaterThan(1);
    expect(hierarchy.hierarchyCount).toBe(3);
    const order = [...hierarchy.hierarchyOrder.slice(0, hierarchy.hierarchyCount)];
    expect(new Set(order).size).toBe(3);
    expect(order.indexOf(1)).toBeLessThan(order.indexOf(2));
    expect(hierarchy.rebuildCommits).toBe(1);
    expect(hierarchy.rebuildBudgetExhaustions).toBeGreaterThan(0);
    expect(hierarchy.justCommitted).toBe(true);
    hierarchy.finishFrame();
    expect(hierarchy.justCommitted).toBe(false);
  });

  test('work scales with active children rather than configured entity capacity', () => {
    const hierarchy = new HierarchyState(1_000_000);
    hierarchy.updateCachedParent(1, NULL_ENTITY, 2);
    expect(hierarchy.stepRebuild(8)).toBe(HierarchyRebuildStatus.Committed);
    expect(hierarchy.hierarchyCount).toBe(1);
    expect(hierarchy.lastRebuildOperations).toBeLessThanOrEqual(2);
  });

  test('restarts fixed scratch when topology changes during a rebuild', () => {
    const hierarchy = new HierarchyState(16);
    hierarchy.updateCachedParent(1, NULL_ENTITY, 4);
    hierarchy.updateCachedParent(2, NULL_ENTITY, 1);
    hierarchy.stepRebuild(1);
    expect(hierarchy.rebuilding).toBe(true);
    hierarchy.updateCachedParent(3, NULL_ENTITY, 2);
    expect(hierarchy.rebuildRestarts).toBe(1);
    finish(hierarchy, 3);
    const order = [...hierarchy.hierarchyOrder.slice(0, hierarchy.hierarchyCount)];
    expect(order.indexOf(1)).toBeLessThan(order.indexOf(2));
    expect(order.indexOf(2)).toBeLessThan(order.indexOf(3));
  });

  test('removes roots from child order while retaining their descendants', () => {
    const hierarchy = new HierarchyState(8);
    hierarchy.updateCachedParent(1, NULL_ENTITY, 4);
    hierarchy.updateCachedParent(2, NULL_ENTITY, 1);
    finish(hierarchy, 32);
    hierarchy.finishFrame();
    hierarchy.updateCachedParent(1, 4, NULL_ENTITY);
    finish(hierarchy, 32);
    const order = [...hierarchy.hierarchyOrder.slice(0, hierarchy.hierarchyCount)];
    expect(order).toEqual([2]);
    expect(hierarchy.parentByEntity[1]).toBe(NULL_ENTITY);
    expect(hierarchy.parentByEntity[2]).toBe(1);
  });

  test('rejects cycles without mutating the existing topology', () => {
    const hierarchy = new HierarchyState(8);
    hierarchy.updateCachedParent(1, NULL_ENTITY, 4);
    hierarchy.updateCachedParent(2, NULL_ENTITY, 1);
    expect(() => hierarchy.updateCachedParent(4, NULL_ENTITY, 2)).toThrow('cycle');
    expect(hierarchy.parentByEntity[4]).toBe(NULL_ENTITY);
    expect(hierarchy.parentByEntity[1]).toBe(4);
    expect(hierarchy.parentByEntity[2]).toBe(1);
  });
});
