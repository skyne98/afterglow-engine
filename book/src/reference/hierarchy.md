# Incremental Hierarchy

Parent changes update fixed linked sibling arrays in constant time. They do not
materialize a bitECS query or rebuild every child immediately.

A fixed dense membership table keeps rebuild work proportional to children that
actually have parents, not total ECS capacity. `HierarchyState.stepRebuild()`
writes their parent-before-child traversal into a second preallocated array. `RenderAdapter` permits at most 512 operations and
0.2 ms each frame. The previous child matrices remain visible until the new
order commits atomically; the commit then forces one complete matrix and
appearance synchronization for all children.

Changes arriving during a rebuild restart its fixed scratch traversal. Counters
expose restarts, budget exhaustion, commits, and operation count. Invalid
cycles and entities beyond configured capacity are rejected rather than
producing a partial hierarchy.
