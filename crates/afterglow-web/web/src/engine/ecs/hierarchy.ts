// Hierarchy — fixed-capacity parent/child topology for the render adapter.
//
// Topology changes update O(1) linked sibling lists. A second hierarchy-order
// buffer is rebuilt incrementally; rendering keeps the previous child matrices
// until the new parent-before-child order commits atomically.

import { createRelation, makeExclusive } from 'bitecs';
import type { EntityId } from '../core/types.ts';
import { NONE_U32, NULL_ENTITY } from '../core/types.ts';

export const ChildOf = createRelation(makeExclusive);

export enum HierarchyRebuildStatus {
  Idle = 0,
  InProgress = 1,
  Committed = 2,
}

export class HierarchyState {
  readonly parentByEntity: Uint32Array;
  readonly childCountByEntity: Uint32Array;
  private readonly firstChildByEntity: Uint32Array;
  private readonly nextSiblingByEntity: Uint32Array;
  private readonly previousSiblingByEntity: Uint32Array;
  private activeOrder: Uint32Array;
  private buildOrder: Uint32Array;
  private readonly traversalStack: Uint32Array;
  private readonly activeChildren: Uint32Array;
  private readonly activeIndexByEntity: Uint32Array;
  private activeChildCount = 0;
  private traversalStackCount = 0;
  private scanCursor = 0;
  private walkNode = NULL_ENTITY;
  private buildCount = 0;
  hierarchyCount = 0;
  topologyDirty = false;
  rebuilding = false;
  justCommitted = false;
  rebuildRestarts = 0;
  rebuildBudgetExhaustions = 0;
  rebuildCommits = 0;
  lastRebuildOperations = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError('hierarchy capacity must be positive');
    this.parentByEntity = new Uint32Array(capacity).fill(NULL_ENTITY);
    this.childCountByEntity = new Uint32Array(capacity);
    this.firstChildByEntity = new Uint32Array(capacity).fill(NULL_ENTITY);
    this.nextSiblingByEntity = new Uint32Array(capacity).fill(NULL_ENTITY);
    this.previousSiblingByEntity = new Uint32Array(capacity).fill(NULL_ENTITY);
    this.activeOrder = new Uint32Array(capacity);
    this.buildOrder = new Uint32Array(capacity);
    this.traversalStack = new Uint32Array(capacity);
    this.activeChildren = new Uint32Array(capacity);
    this.activeIndexByEntity = new Uint32Array(capacity).fill(NONE_U32);
  }

  get hierarchyOrder(): Uint32Array { return this.activeOrder; }

  /** Validate before the caller mutates its ECS relation storage. */
  validateParentChange(child: EntityId, oldParent: EntityId, newParent: EntityId): void {
    if (child >= this.capacity || (newParent !== NULL_ENTITY && newParent >= this.capacity))
      throw new RangeError('hierarchy entity exceeds configured capacity');
    if (child === newParent) throw new Error('an entity cannot parent itself');
    let ancestor = newParent;
    for (let depth = 0; ancestor !== NULL_ENTITY && depth < this.capacity; depth++) {
      if (ancestor === child) throw new Error('hierarchy parenting would create a cycle');
      ancestor = this.parentByEntity[ancestor];
    }
    if (ancestor !== NULL_ENTITY) throw new Error('hierarchy parent chain exceeds capacity');
    if (this.parentByEntity[child] !== oldParent)
      throw new Error('cached hierarchy parent does not match requested old parent');
  }

  /** O(1) linked-list topology update. Rebuild work is deferred to stepRebuild. */
  updateCachedParent(child: EntityId, oldParent: EntityId, newParent: EntityId): void {
    this.validateParentChange(child, oldParent, newParent);
    if (oldParent !== NULL_ENTITY) {
      const previous = this.previousSiblingByEntity[child];
      const next = this.nextSiblingByEntity[child];
      if (previous === NULL_ENTITY) this.firstChildByEntity[oldParent] = next;
      else this.nextSiblingByEntity[previous] = next;
      if (next !== NULL_ENTITY) this.previousSiblingByEntity[next] = previous;
      this.childCountByEntity[oldParent]--;
    }

    this.previousSiblingByEntity[child] = NULL_ENTITY;
    this.nextSiblingByEntity[child] = NULL_ENTITY;
    if (newParent !== NULL_ENTITY) {
      const first = this.firstChildByEntity[newParent];
      this.nextSiblingByEntity[child] = first;
      if (first !== NULL_ENTITY) this.previousSiblingByEntity[first] = child;
      this.firstChildByEntity[newParent] = child;
      this.childCountByEntity[newParent]++;
    }
    if (oldParent === NULL_ENTITY && newParent !== NULL_ENTITY) {
      this.activeIndexByEntity[child] = this.activeChildCount;
      this.activeChildren[this.activeChildCount++] = child;
    } else if (oldParent !== NULL_ENTITY && newParent === NULL_ENTITY) {
      const index = this.activeIndexByEntity[child];
      const last = this.activeChildren[--this.activeChildCount];
      this.activeChildren[index] = last;
      this.activeIndexByEntity[last] = index;
      this.activeIndexByEntity[child] = NONE_U32;
    }
    this.parentByEntity[child] = newParent;
    this.topologyDirty = true;
    if (this.rebuilding) {
      this.rebuildRestarts++;
      this.resetBuild();
    }
  }

  private resetBuild(): void {
    this.rebuilding = true;
    this.scanCursor = 0;
    this.walkNode = NULL_ENTITY;
    this.traversalStackCount = 0;
    this.buildCount = 0;
    this.lastRebuildOperations = 0;
  }

  /**
   * Incrementally build parent-before-child order. Each scanned or emitted
   * entity consumes one operation. Deadline checks occur every eight ops.
   */
  // @hot-no-alloc-begin HierarchyState.stepRebuild
  stepRebuild(maxOperations: number, deadlineMs = Number.POSITIVE_INFINITY): HierarchyRebuildStatus {
    if (!Number.isInteger(maxOperations) || maxOperations <= 0)
      throw new RangeError('hierarchy rebuild operation limit must be positive');
    if (!this.topologyDirty) return HierarchyRebuildStatus.Idle;
    if (!this.rebuilding) this.resetBuild();
    let operations = 0;
    while (operations < maxOperations) {
      if ((operations & 7) === 0 && performance.now() >= deadlineMs) break;
      if (this.walkNode !== NULL_ENTITY) {
        const node = this.walkNode;
        if (this.buildCount >= this.capacity) {
          this.rebuilding = false;
          throw new Error('hierarchy traversal exceeded capacity (cycle or duplicate child)');
        }
        this.buildOrder[this.buildCount++] = node;
        const sibling = this.nextSiblingByEntity[node];
        if (sibling !== NULL_ENTITY) this.traversalStack[this.traversalStackCount++] = sibling;
        const child = this.firstChildByEntity[node];
        this.walkNode = child !== NULL_ENTITY
          ? child
          : (this.traversalStackCount === 0 ? NULL_ENTITY : this.traversalStack[--this.traversalStackCount]);
        operations++;
        continue;
      }

      if (this.scanCursor < this.activeChildCount) {
        const entity = this.activeChildren[this.scanCursor++];
        const parent = this.parentByEntity[entity];
        // Start once per root's child list; DFS follows every sibling/descendant.
        if (parent !== NULL_ENTITY && this.parentByEntity[parent] === NULL_ENTITY &&
            this.firstChildByEntity[parent] === entity) this.walkNode = entity;
        operations++;
        continue;
      }

      const previous = this.activeOrder;
      this.activeOrder = this.buildOrder;
      this.buildOrder = previous;
      this.hierarchyCount = this.buildCount;
      this.rebuilding = false;
      this.topologyDirty = false;
      this.justCommitted = true;
      this.rebuildCommits++;
      this.lastRebuildOperations += operations;
      return HierarchyRebuildStatus.Committed;
    }
    this.lastRebuildOperations += operations;
    this.rebuildBudgetExhaustions++;
    return HierarchyRebuildStatus.InProgress;
  }
  // @hot-no-alloc-end HierarchyState.stepRebuild

  finishFrame(): void { this.justCommitted = false; }
}
