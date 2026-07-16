# Incremental hierarchy

`engine/hierarchy.ts` owns fixed-capacity parent/child topology for
`RenderAdapter`.

## API

```ts
enum HierarchyRebuildStatus { Idle, InProgress, Committed }

class HierarchyState {
  readonly parentByEntity: Uint32Array;
  readonly childCountByEntity: Uint32Array;
  readonly hierarchyOrder: Uint32Array;
  readonly capacity: number;
  hierarchyCount: number;
  topologyDirty: boolean;
  rebuilding: boolean;
  justCommitted: boolean;

  validateParentChange(child: EntityId, oldParent: EntityId, newParent: EntityId): void;
  updateCachedParent(child: EntityId, oldParent: EntityId, newParent: EntityId): void;
  stepRebuild(maxOperations: number, deadlineMs?: number): HierarchyRebuildStatus;
  finishFrame(): void;
}
```

`RenderAdapter.setParent` calls `validateParentChange` before mutating the ECS
relation, preventing rejected cycles from leaving relation storage changed.
`updateCachedParent` then updates fixed first-child and
doubly-linked sibling arrays. It does not run an ECS query or rebuild an entity
list.

A dense fixed active-child table means rebuild cost scales with entities that
actually have parents, not configured ECS capacity. `stepRebuild` scans at most
`maxOperations` active children and checks its monotonic deadline every eight
operations. A fixed DFS stack writes parent-before-child
order into a second typed array. A topology change during rebuilding resets the
scratch traversal to the newest topology and increments `rebuildRestarts`.
The currently published order remains unchanged until the new traversal commits
by swapping the two arrays.

`RenderAdapter.prepareFrame` admits at most 512 hierarchy operations and 0.2 ms
per frame. While rebuilding, child world matrices remain at their previously
committed values. The commit frame forces matrix and appearance synchronization
for every published child, so dirty flags cleared during a multi-frame rebuild
cannot lose updates. Roots continue through the normal root pass.

Telemetry is incremental: `rebuildRestarts`, `rebuildBudgetExhaustions`,
`rebuildCommits`, and `lastRebuildOperations`. Storage is allocated only by the
constructor. Cycles and capacity violations are deterministic errors.
