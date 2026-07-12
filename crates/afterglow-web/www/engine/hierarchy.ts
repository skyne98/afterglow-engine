// Hierarchy — parent/child entity relationships for the render adapter.
//
// Uses bitECS's exclusive ChildOf relation. The adapter maintains a cached
// parent array and a hierarchy-ordered entity list (parents before children).
// The list is rebuilt only when topology changes (add/remove ChildOf), not
// every frame.

import { createRelation, makeExclusive, query } from 'bitecs';
import type { EntityId } from './types.js';
import { NULL_ENTITY } from './types.js';

export const ChildOf = createRelation(makeExclusive);

export class HierarchyState {
  readonly parentByEntity: Uint32Array;
  readonly childCountByEntity: Uint32Array;
  readonly hierarchyOrder: Uint32Array;
  hierarchyCount = 0;
  topologyDirty = false;

  constructor(capacity: number) {
    this.parentByEntity = new Uint32Array(capacity).fill(NULL_ENTITY);
    this.childCountByEntity = new Uint32Array(capacity);
    this.hierarchyOrder = new Uint32Array(capacity);
  }

  /** Cache the parent for an entity (called from the observer path). */
  updateCachedParent(child: EntityId, oldParent: EntityId, newParent: EntityId): void {
    if (oldParent !== NULL_ENTITY) {
      this.childCountByEntity[oldParent]--;
    }
    if (newParent !== NULL_ENTITY) {
      this.childCountByEntity[newParent]++;
    }
    this.parentByEntity[child] = newParent;
  }

  /** Rebuild the hierarchy-ordered entity list. Call only when topology changes. */
  rebuild(world: object, transform: object): void {
    // @ts-expect-error — bitECS query takes world + component array
    const ordered = query(world, [transform, ChildOf('*')]) as readonly EntityId[];
    if (ordered.length > this.hierarchyOrder.length) {
      throw new Error('Hierarchy order capacity exceeded');
    }
    this.hierarchyCount = ordered.length;
    for (let i = 0; i < ordered.length; i++) {
      this.hierarchyOrder[i] = ordered[i];
    }
    this.topologyDirty = false;
  }
}
