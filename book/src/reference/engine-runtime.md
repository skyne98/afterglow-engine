# Engine Runtime

`EngineRuntime` is the normal owner of a browser game's sealed frame loop. A
visual entrypoint should not create its own animation loop or manually order
worker polling, virtual-texture work, ECS synchronization, and rendering.

The runtime has explicit phases:

```text
Bootstrap -> Warmup -> GameplaySealed -> Running <-> Stopped -> Shutdown
```

During bootstrap, provide mandatory capacities and register bounded worker
inputs and render passes. Registration returns `Registered`,
`CapacityExceeded`, or `RuntimeSealed`; storage never grows silently.

```ts
const runtime = EngineRuntime.forScene({
  scene,
  entityCapacity: 5_000,
  memory: memoryConfig,
  diagnosticCapacity: 64,
  maxWorkerInputs: 4,
  maxRenderPasses: 4,
});

runtime.registerWorker(workerInput);
runtime.registerRenderPass(mainPass);
runtime.enterWarmup();
await runtime.warm();
runtime.sealGameplay();
runtime.start(gameFrameClient);
```

The runtime owns the scene's `RenderAdapter`, one persistent frame record, and
the only animation callback. It prepares workers, VT, structural changes,
transforms, and GPU uploads in a fixed order before invoking the game update and
registered render passes. Registered passes warm and seal with the runtime.
`RendererHost` submits through synchronous `render()` after initialization, so
normal rendering does not create one promise per frame. Stopping during an
update does not schedule another frame.

Frame exceptions stop the runtime and enter a fixed-capacity diagnostic ring.
The ring drops newest records visibly when full and exposes stable dropped and
high-water counters. Shutdown is idempotent and disposes render passes in
reverse registration order.

The current migration is not complete until every visual demo uses this path;
`engine-conformance.json` tracks that status and the architecture lint prevents
new demo-owned frame infrastructure.
