# Renderer Sealing

Warm every scene/camera and render-target variant before gameplay with
`warmRendererVariants`, then seal a `RendererSeal` monitor. Development builds
can detect any render or compute pipeline created afterward. The VT dungeon
warms both the main color context and RG32Uint feedback context and records GPU
timestamps outside the frame hot path.

`RendererHost` is the canonical renderer owner used as an `EngineRuntime`
render pass. It creates a fail-closed WebGPU renderer from the typed module
graph, owns canvas/resize/GPU-error listener lifetime, exposes the validated GPU
device, immutable adapter identity, and confines VT native-texture attachment
behind a host method. It
applies the configured pixel-ratio ceiling, compiles during warm-up, resets
Three's external-loop counters and assigns the engine frame ID before rendering,
seals with the runtime, and rolls back partial initialization. Later render
passes share that frame identity. Disposal is idempotent.

Generated-WGSL checks run through the bootstrap-only
`RendererHost.inspectShaderModulesDuring()` boundary. The host restores the
WebGPU method after success or failure and rejects nested or post-seal capture;
application/demo code never patches `GPUDevice.createShaderModule` directly.

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
