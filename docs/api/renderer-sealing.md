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

`renderer-host.ts` is the canonical owner of a WebGPU-only renderer. It creates
the renderer through `createWebGPUOnlyRenderer`, validates the methods required
by the host, exposes the validated typed GPU device and immutable
`adapterInfo`, appends/removes the canvas,
applies a bounded pixel ratio, owns the resize and uncaptured-GPU-error listeners,
compiles its scene/camera during runtime warm-up, implements `EngineRenderPass`,
resets Three's counters and assigns `RenderFrame.frameId` at each external-loop
frame boundary, and seals its `RendererSeal` when `EngineRuntime` seals
registered passes.
`EngineRuntime.createVirtualTextureFeedback(system, capacities)` reserves the
worker/pass records. During `runtime.warm()`, `RendererHost` performs the one
real atlas-initializing render, attaches every internal format pool, and binds
feedback resize to the physical canvas. Demos cannot call renderer attachment.
Three's private native-texture lookup remains confined to the host, and physical
stores never cross the public boundary. Initialization failure disposes
the partially created renderer; `dispose()` is idempotent.

`inspectShaderModulesDuring(operation, inspect)` is a bootstrap-only generated-
WGSL inspection boundary. It rejects nested or post-seal use and restores
`GPUDevice.createShaderModule` in `finally`, including operation or validation
failure. This confines temporary WebGPU interception to `RendererHost`; demos
use subsystem validators rather than patching the device.

`webgpu-only.ts` imports Three's WebGPU renderer from the module graph instead
of reading `window.THREE`. A renderer factory can be injected for tests, while
the production factory remains fail-closed against WebGL fallback.

Three.js is initialized with `trackTimestamp: true` where profiling is enabled.
Timestamp resolution is a diagnostic slow path, not frame-hot work. The
clean-break timing record carries validity and resolved frame ID plus separate
HDR scene, output-transform, VT-feedback, and total durations; context IDs are
filtered to exactly that frame.

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
