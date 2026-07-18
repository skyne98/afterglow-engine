// Fixed dirty ring — marks entities without ECS structural churn and supports
// bounded prefix processing while preserving deferred work.

import type { EntityId } from '../core/types.ts';
import { RenderDirty } from '../core/types.ts';

export enum DirtyMarkStatus {
  Accepted = 0,
  AlreadyQueued = 1,
  CapacityExceeded = 2,
}

export class EntityDirtyQueue {
  private readonly entities: Uint32Array;
  readonly queued: Uint8Array;
  readonly flags: Uint8Array;
  private head = 0;
  private tail = 0;
  count = 0;
  highWater = 0;
  overflows = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0) throw new RangeError('dirty queue capacity must be positive');
    this.entities = new Uint32Array(capacity);
    this.queued = new Uint8Array(capacity);
    this.flags = new Uint8Array(capacity);
  }

  // @hot-no-alloc-begin EntityDirtyQueue.mark
  mark(entity: EntityId, dirty: RenderDirty): DirtyMarkStatus {
    if (entity >= this.capacity) {
      this.overflows++;
      return DirtyMarkStatus.CapacityExceeded;
    }
    this.flags[entity] |= dirty;
    if (this.queued[entity] !== 0) return DirtyMarkStatus.AlreadyQueued;
    if (this.count >= this.capacity) {
      this.overflows++;
      return DirtyMarkStatus.CapacityExceeded;
    }
    this.queued[entity] = 1;
    this.entities[this.tail] = entity;
    this.tail = (this.tail + 1) % this.capacity;
    this.count++;
    if (this.count > this.highWater) this.highWater = this.count;
    return DirtyMarkStatus.Accepted;
  }
  // @hot-no-alloc-end EntityDirtyQueue.mark

  // @hot-no-alloc-begin EntityDirtyQueue.entityAt
  entityAt(index: number): EntityId {
    return this.entities[(this.head + index) % this.capacity];
  }
  // @hot-no-alloc-end EntityDirtyQueue.entityAt

  // @hot-no-alloc-begin EntityDirtyQueue.clearPrefix
  clearPrefix(maxEntities: number): number {
    const removed = Math.min(maxEntities, this.count);
    for (let index = 0; index < removed; index++) {
      const entity = this.entities[this.head];
      this.head = (this.head + 1) % this.capacity;
      this.queued[entity] = 0;
      this.flags[entity] = RenderDirty.None;
    }
    this.count -= removed;
    if (this.count === 0) this.head = this.tail = 0;
    return removed;
  }
  // @hot-no-alloc-end EntityDirtyQueue.clearPrefix

  // @hot-no-alloc-begin EntityDirtyQueue.clear
  clear(): void { this.clearPrefix(this.count); }
  // @hot-no-alloc-end EntityDirtyQueue.clear
}
