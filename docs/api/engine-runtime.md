# Engine runtime

`crates/afterglow-web/web/src/engine/core/runtime.ts` is the canonical owner of sealed
browser-frame orchestration. Visual entrypoints should use it instead of owning
`requestAnimationFrame` or manually ordering workers, VT, ECS synchronization,
and render passes.

## Lifecycle

Execution state remains:

```text
Bootstrap -> Warmup -> GameplaySealed -> Running <-> Stopped -> Shutdown
```

A separate observable readiness state is stricter:

```text
Bootstrap -> Warmup -> GameplaySealed -> Starting -> GameReady
                                              \-> Suspended/Fatal -> Shutdown
```

`GameReady` requires one successful game update, one successful designated
presentation pass in the same post-seal frame, every registered worker's required
bootstrap publication to be complete, and zero recorded/dropped fatal diagnostics.
For virtual textures this includes required startup residency. First surface
presentation alone is not engine readiness.

`EngineRuntime` constructs and owns one `EngineMemory`, one `FrameBudget`, one
fixed `EngineDiagnostics`, a fixed worker-input table, and a fixed render-pass
table. `await EngineRuntime.forScene()` also constructs and owns the scene's
`RenderAdapter` and `RendererHost` at explicit capacities. Demos never construct,
attach, or register the renderer themselves. The runtime eagerly installs
memory and budget resources in the adapter world through a `ResourceManifest`.

The scene constructor accepts `scene`, `camera`, renderer viewport/options,
`maxOwnedResources`, and the normal memory/worker/pass capacities. Runtime-owned
resources are admitted with `ownDisposable()` or `ownCloseable()`; `close()`
unwinds them in reverse order before render passes and awaits asynchronous worker
shutdown. A built-in page teardown listener invokes the same idempotent path.

During bootstrap, engine subsystems register worker inputs and render passes. Registration
returns:

```ts
enum RegistrationStatus {
  Registered,
  CapacityExceeded,
  RuntimeSealed,
  PresentationAlreadyRegistered,
}
```

No registration storage grows. Late registration is rejected. Exactly one pass
must set `presentation = true`; duplicate ownership is rejected and sealing
without a presentation pass fails before mutating sealed state.

`createVirtualTextureFeedback(system, capacities)` atomically creates the one
feedback coordinator, registers its worker/pass records, performs the first real
atlas-initializing render during warm-up, attaches all pools, and owns physical
feedback resize. Games only register their material bindings with the returned
coordinator.

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

After the first complete frame, `readReadinessInto(out)` copies readiness stage,
first update/presentation frame IDs, and fatal count into caller-owned output.
On the native target the transition emits `op_afterglow_game_ready`; the shell's
first-present readiness is reserved for explicit `--compat-three` runs. A frame exception, global page error/rejection, uncaptured GPU error, or device
loss immediately invalidates readiness, enters `Fatal`, stops the loop, and
installs one bounded fatal panel. Stopping during update enters `Suspended` and
does not schedule another frame. `dispose()` is the synchronous bootstrap/test
path. `await close()` is the normal path: it awaits closeable owners in reverse
order, disposes synchronous owners and render passes, disposes the adapter, and
moves memory/runtime to shutdown.

## Interfaces

```ts
interface EngineFrameClient {
  /** @alloc-effect none */
  update(frame: Readonly<RenderFrame>): void;
}

interface EngineRenderPass {
  readonly presentation?: boolean; // exactly one true
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
overflow, unique presentation ownership, `GameReady` frame IDs, fatal readiness
invalidation, stable frame identity, stop-during-update, exception capture,
idempotent shutdown, and reverse pass disposal.
