# Renderer Sealing

Warm every scene/camera and render-target variant before gameplay with
`warmRendererVariants`, then seal a `RendererSeal` monitor. Development builds
can detect any render or compute pipeline created afterward. The VT dungeon
warms both the main color context and RG32Uint feedback context and records GPU
timestamps outside the frame hot path.

Render descriptors declare fixed capacity:

- instanced descriptors use `maxShards`;
- unique descriptors use `poolCapacity`.

Call `RenderAdapter.warmAllDescriptors()` before sealing. Gameplay attachment
then reuses shards and detached objects, returning explicit not-warmed or
capacity-exceeded telemetry instead of creating Three.js objects.

Renderer preparation is sliced to 256 structural changes, 4,096 dirty roots,
4,096 hierarchy children, and 512 continuous unique proxies per frame. Deferred
ring entries retain their dirty flags. Instance uploads are coalesced through
fixed dirty bitsets and pooled ranges.
