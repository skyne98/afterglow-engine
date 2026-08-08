# Engine Runtime

`EngineRuntime` is the normal owner of a browser game's sealed frame loop. A
visual entrypoint should not create its own animation loop or manually order
worker polling, virtual-texture work, ECS synchronization, and rendering.

The runtime has explicit execution and readiness phases:

```text
execution: Bootstrap -> Warmup -> GameplaySealed -> Running <-> Stopped -> Shutdown
readiness: Bootstrap -> Warmup -> GameplaySealed -> Starting -> GameReady
                                                        \-> Suspended/Fatal
```

During bootstrap, provide mandatory capacities. The normal asynchronous scene
constructor owns the renderer, presentation pass, page teardown, and reverse
shutdown; demos do not create or register them.

```ts
const runtime = await EngineRuntime.forScene({
  scene,
  camera,
  entityCapacity: 5_000,
  memory: memoryConfig,
  diagnosticCapacity: 64,
  maxWorkerInputs: 4,
  maxRenderPasses: 4,
  maxOwnedResources: 8,
  renderer: { parameters: { antialias: true }, onResize: resizeCamera },
});

runtime.ownCloseable(engineAssets);
runtime.createVirtualTextureFeedback(textures, feedbackCapacities);
runtime.enterWarmup();
await runtime.warm();
runtime.sealGameplay();
runtime.start(gameFrameClient);
```

The runtime owns the scene's `RenderAdapter`, one persistent frame record, and
the only animation callback. It prepares workers, VT, structural changes,
transforms, and GPU uploads in a fixed order before invoking the game update and
registered render passes. Registered passes warm and seal with the runtime.
`RendererHost` resets Three's external-loop counters, assigns the engine frame
ID, and submits through synchronous `render()` after initialization. Later
render passes share that frame identity, and normal rendering creates no promise
per frame. `GameReady` is published only after one successful post-seal game
update and designated presentation in the same complete frame, all required
worker/bootstrap publications (including VT startup residency), and zero fatal
diagnostics. On native this is the strict signal consumed by the shell;
first-present readiness is limited to explicit `--compat-three` runs. Stopping
during an update enters suspended readiness and does not schedule another frame.

Frame exceptions, global page failures, GPU errors, and device loss stop the
runtime, enter fatal readiness, write a fixed-capacity diagnostic ring, and show
one fatal panel.
The ring drops newest records visibly when full and exposes stable dropped and
high-water counters. `await close()` is idempotent, awaits closeable owners in
reverse order, and then disposes render passes and the adapter. Every visual demo
uses this runtime-owned renderer/lifecycle path; architecture and deletion-ledger
checks prevent the old construction and cleanup graphs from returning.
