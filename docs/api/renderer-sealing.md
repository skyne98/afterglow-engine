# Renderer sealing

## Variant and pipeline warm-up

`renderer-seal.ts` exports `warmRendererVariants(renderer, variants)` and
`RendererSeal`. Install the monitor immediately after WebGPU initialization,
compile every declared scene/camera pair, perform one real render for every
render-target format, then call `seal()`.

The monitor wraps Three.js WebGPU backend render/compute pipeline creation at
bootstrap. It counts total pipelines and post-seal violations without polling
or snapshot allocation. `assertNoViolations()` is a deterministic development
gate. The VT dungeon warms its main and RG32Uint feedback contexts; its measured
steady-state count is six render pipelines with zero post-seal creation.

Three.js is initialized with `trackTimestamp: true` where profiling is enabled.
`resolveTimestampsAsync('render')` runs once per diagnostic second, not in the
frame hot path. Context IDs distinguish main and feedback GPU durations.

## Proxy pools

`RenderAdapter.warmDescriptor(id)` allocates descriptor-owned capacity before
gameplay:

- `InstancedRenderDescriptor.maxShards` creates every `InstanceShard`;
- `UniqueRenderDescriptor.poolCapacity` creates detached `Object3D` proxies.

`warmAllDescriptors()` covers the current registry. Gameplay attachment never
constructs a shard or unique object. It returns internal `RenderAttachStatus`
values and increments stable counters for `DescriptorNotWarmed` or
`CapacityExceeded`. Detached unique proxies return to their descriptor pool.
A fixed dense entity table drives at most 512 continuous unique syncs per frame.

## Bounded renderer work

Per-frame limits are:

| Work | Limit |
|---|---:|
| Structural reconciliations | 256 |
| Dirty root entities | 4,096 |
| Hierarchy child syncs | 4,096 |
| Continuous unique proxy syncs | 512 |

`EntityDirtyQueue` is a fixed ring. Processing clears only a prefix and leaves
flags on the deferred suffix. Overflow returns `DirtyMarkStatus.CapacityExceeded`
and increments telemetry. Hierarchy sync uses a rotating slice and recomputes
visited child matrices unconditionally, ensuring parent changes eventually
propagate across a multi-frame hierarchy.

`DirtySlotRanges` maintains fixed bitsets and sixteen pooled ranges per instance
attribute. Flushes coalesce adjacent dirty slots and fall back to one bounding
range under fragmentation; no per-slot GPU upload is emitted.
