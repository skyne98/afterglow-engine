# Engine runtime

`crates/afterglow-web/web/src/engine/core/runtime.ts` is the canonical owner of sealed
browser-frame orchestration. Visual entrypoints should use it instead of owning
`requestAnimationFrame` or manually ordering workers, VT, ECS synchronization,
and render passes.

## Lifecycle

```text
Bootstrap -> Warmup -> GameplaySealed -> Running <-> Stopped -> Shutdown
```

`EngineRuntime` constructs and owns one `EngineMemory`, one `FrameBudget`, one
fixed `EngineDiagnostics`, a fixed worker-input table, and a fixed render-pass
table. `EngineRuntime.forScene()` also constructs and owns the scene's
`RenderAdapter` at an explicit entity capacity. The runtime eagerly installs
memory and budget resources in the adapter world through a `ResourceManifest`.

During bootstrap, callers register worker inputs and render passes. Registration
returns:

```ts
enum RegistrationStatus {
  Registered,
  CapacityExceeded,
  RuntimeSealed,
}
```

No registration storage grows. Late registration is rejected.

Call `enterWarmup()`, then `await warm()`, then `sealGameplay()`. Sealing occurs
in this order:

1. render adapter;
2. engine memory;
3. ECS resource manifest;
4. every registered render pass, including `RendererHost`;
5. runtime state.

`start(client)` is legal only after sealing or from `Stopped`. The runtime owns
the only animation callback and mutates one persistent `RenderFrame` record.
Each frame executes:

1. fixed worker polling;
2. optional VT processing;
3. structural/pose drains;
4. render-adapter preparation;
5. the allocation-effect-checked game update;
6. registered render passes in registration order. `RendererHost` resets Three's
   external-loop counters and assigns the engine frame ID before using Three's
   synchronous `render()`; later passes share that identity, and no render
   promise is created per gameplay frame.

A frame exception is stored in the bounded diagnostics ring and stops the loop.
Stopping during update does not schedule another frame. `dispose()` is
idempotent, disposes render passes in reverse order, disposes the adapter, and
moves memory/runtime to shutdown.

## Interfaces

```ts
interface EngineFrameClient {
  /** @alloc-effect none */
  update(frame: Readonly<RenderFrame>): void;
}

interface EngineRenderPass {
  warm?(): Promise<void>;
  seal?(): void;
  render(frame: Readonly<RenderFrame>): void;
  dispose(): void;
}
```

The frame client and pass objects are created during bootstrap. Runtime frame
execution does not create callback closures or frame records.

## Diagnostics

`EngineDiagnostics(capacity)` is a fixed FIFO ring. `tryRecord` returns
`CapacityExceeded` and increments `dropped` when full; it never reallocates or
overwrites an unread record. `readInto` and `shiftInto` require a caller-owned
`DiagnosticRecord`. High-water and dropped counters remain stable telemetry.

## Verification

```sh
bun test crates/afterglow-web/web/src/engine/core/runtime.test.ts \
  crates/afterglow-web/web/src/engine/core/diagnostics.test.ts
cargo run -p xtask conformance
```

The tests cover lifecycle transitions, deterministic order, fixed registration
overflow, stable frame identity, stop-during-update, exception capture,
idempotent shutdown, and reverse pass disposal.
