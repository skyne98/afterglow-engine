// Fixed dirty queue — marks entities dirty without adding/removing ECS tag
// components (which would cause structural churn).
//
// Systems call `dirty.mark(entity, RenderDirty.Transform)` after writing to
// component arrays. The render adapter drains the queue each frame, then
// calls `clear()`.

import type { EntityId } from './types.js';
import { RenderDirty } from './types.js';

export class EntityDirtyQueue {
  readonly entities: Uint32Array;
  readonly queued: Uint8Array;
  readonly flags: Uint8Array;
  count = 0;

  constructor(capacity: number) {
    this.entities = new Uint32Array(capacity);
    this.queued = new Uint8Array(capacity);
    this.flags = new Uint8Array(capacity);
  }

  mark(entity: EntityId, dirty: RenderDirty): void {
    this.flags[entity] |= dirty;
    if (this.queued[entity] !== 0) return;
    if (this.count >= this.entities.length) {
      throw new Error('Render dirty queue capacity exceeded');
    }
    this.queued[entity] = 1;
    this.entities[this.count++] = entity;
  }

  clear(): void {
    for (let i = 0; i < this.count; i++) {
      const entity = this.entities[i];
      this.queued[entity] = 0;
      this.flags[entity] = RenderDirty.None;
    }
    this.count = 0;
  }
}
