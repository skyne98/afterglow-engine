# deno_core + winit scheduling for browser-correct rAF

**Investigated:** 2026-07-25  
**Scope:** `afterglow-shell` module evaluation, persistent deno_core event-loop
integration, `requestAnimationFrame`, and Three.js `compileAsync()`.

## Question

How should the native shell drive a top-level module await that yields through
`requestAnimationFrame`, without hiding deno_core errors, busy-spinning, or
changing rAF into a timer?

## Finding

The current custom module-evaluation loop is a useful root-cause prototype, but
is not an acceptable production scheduler:

- it manually polls the module future with a no-op waker;
- it calls `run_event_loop()` even though the shell owns another persistent
  event loop;
- it discards every `run_event_loop()` error with `.ok()`;
- it drains rAF as quickly as startup can loop, rather than at a presentation
  opportunity;
- it assumes Three.js's current fake-async pipeline behavior;
- it has no bounded evaluation policy or focused regression tests.

The proper fix is one persistent host scheduler shared by startup and gameplay.
It must poll one deno_core event-loop tick from the winit loop, make pending rAF
work visible to deno_core through `ExternalOpsTracker`, and run rAF callbacks
only from a winit redraw/presentation turn.

## Implementation result (2026-07-25)

The recommended defaults were approved and implemented:

- winit remains the outer loop; game-module evaluation is stored in `App` and
  advances across redraws instead of blocking `resumed()`;
- startup, frame, input, and resize paths use bounded `poll_event_loop()` turns;
  the only remaining run-to-idle helper initializes the finite DOM bridge;
- a real coalesced winit waker replaces the no-op waker;
- `raf.ts` owns a fixed 1,024-slot queue with O(1) request/cancel, batch
  timestamps, next-frame registration, exception isolation, deterministic
  overflow, and diagnostics;
- the native rAF ops hold one `ExternalOpsTracker` reference while callbacks
  remain active and request actual winit redraws;
- the host no longer directly invokes the game renderer in parallel with rAF;
- startup uses a configurable 30-second active-time fatal deadline, paused
  across native suspension; all deno/module errors propagate to a nonzero fatal
  exit;
- after the pure-rAF measurement gate failed, the user explicitly admitted only
  the standards-shaped `scheduler.yield()` subset. It is backed by
  `deno_web::op_defer`; `postTask` and `TaskController` remain absent.

The first correct-rAF engine run exposed the expected granularity problem: the
5,000-entity demo had still not completed after 90 active seconds because Three
calls multiple yields per compile item. After admitting the task continuation,
10,000 direct `scheduler.yield()` calls completed in 193.3 ms and the real
engine demo reached renderer readiness in 145 active ms under the default
30-second deadline. This validates that continuations are deno/winit task turns,
not presentation frames or microtasks.

Validation on the RTX 3090 also passed the worker-composition probe
(`OP_BRIDGE_OK`), a two-redraw top-level rAF cadence probe, and a rejected-TLA
sentinel that preserved its original error and exited nonzero. The fixed queue
and Scheduler-subset Bun regressions cover API shape, asynchronous continuation,
ordering, shared timestamps, next-frame scheduling, cancellation, callback
exception isolation, capacity, and overflow.

Long-duration allocation/cadence soak and the complete release hardware matrix
remain release gates.

## Primary-source evidence

### deno_core is embedded by polling, not run-to-idle

`deno_core` 0.408's `JsRuntime::run_event_loop()` wraps
`poll_fn(|cx| poll_event_loop(cx, ...))` and runs until all referenced work is
finished. In Deno discussion #25799, maintainer Bartlomieju explicitly states:
`run_event_loop` runs until all work finishes; an embedder that owns another
event loop should call `JsRuntime::poll_event_loop()` for one tick and poll it
again later.

This shell is a persistent browser host. A referenced timer, interval, rAF loop,
or other long-lived browser task is expected to keep work alive, so
run-to-idle is the wrong primitive for per-frame execution.

### ExternalOpsTracker exists for host-owned pending work

`deno_core::ExternalOpsTracker` is documented in source as allowing an embedder
to track operations that should keep the event loop alive. Its `ref_op()` and
`unref_op()` methods feed `EventLoopPendingState::has_pending_external_ops`.
The stalled-top-level-await check explicitly does not report a deadlock while an
external operation is pending.

A native presentation request is exactly such an operation: deno_core cannot
see the callback queue, while winit owns the event that will make it progress.
Use one referenced external-operation token while the rAF queue is non-empty.

### rAF is a presentation callback, not a timer or microtask

The Window rAF contract calls one-shot callbacks before the next repaint,
generally at display refresh cadence. All callbacks in one frame receive the
same timestamp, and requests made by those callbacks target a later frame.
Replacing rAF with a microtask or interval violates those semantics.

Three.js r185's `yieldToMain()` prefers `scheduler.yield()` and otherwise awaits
rAF. Its comments state that the yield exists to allow rendering and other
tasks. `compileAsync()` yields during async node building and between compiled
objects. An immediate startup drain proves the dependency but defeats the
reason for the yield.

### Timers and scheduler.yield are not the primary fix

`deno_web` 0.286 already provides ref-counted WHATWG timers and `op_defer`; its
`defer()` helper explicitly avoids microtask starvation. A temporary interval
could keep deno_core alive, but would fire rAF independently of repaint and
would leave a persistent referenced timer. It is not correct.

A host `scheduler.yield()` backed by a deferred task is potentially useful, and
Three.js will select it automatically. However, Deno's Priority Task Scheduler
API remains open in denoland/deno issue #27976 as of 2026-06-04. Exposing only
`scheduler.yield()` is therefore a deliberate partial-browser-API policy, not a
neutral bug fix. First make the standard rAF fallback correct. Measure warm-up
latency; add a standards-shaped scheduler continuation only if that gate shows
rAF-granularity yielding is unacceptable.

### winit should remain the outer event loop

winit 0.30 offers `pump_app_events()` for integration into an external loop,
but its own documentation calls it non-portable and strongly recommends that
redraw and lifecycle handling remain synchronously inside winit callbacks.
Keep `run_app()`/`ApplicationHandler`; wake it with a coalesced
`EventLoopProxy` user event and use `RedrawRequested` for rAF.

## Recommended design

### 1. One `NativeTaskDriver`

Own in one structure:

- the `JsRuntime`;
- an optional pinned module-evaluation future;
- startup phase/deadline and failure state;
- a real waker backed by a coalesced winit `EventLoopProxy` event;
- fixed telemetry and per-turn tick limits.

Poll the evaluation future before and after one bounded
`JsRuntime::poll_event_loop()` tick using the same real `Context`. Never use a
no-op waker, never call `run_event_loop()` from frame/input/startup paths, and
never discard an event-loop error.

A JavaScript task remains run-to-completion, so the host cannot preempt an
individual long task. The driver bounds the number of deno ticks and host work
per winit turn; cooperative yields allow input/redraw turns between chunks.

### 2. Host-tracked, bounded rAF queue

Keep callback ownership in the browser shim, but replace the growing `Map` and
per-frame `Array.from()` with a fixed-capacity queue/slot table allocated at
bootstrap. Required behavior:

- O(1) request and cancellation;
- registration-order callbacks;
- one shared frame timestamp;
- requests created during a drain run on a later frame;
- cancellation before invocation suppresses the callback;
- one callback exception is reported but does not skip later callbacks;
- deterministic capacity rejection and telemetry.

On transition from empty to non-empty:

1. call a native op that references one `ExternalOpsTracker` token;
2. request a winit redraw.

When the queue is drained or its final callback is canceled, unreference that
single token. State on the Rust side must guard ref/unref balance.

If the window is suspended or occluded, leave the token referenced and resume
at the next admitted redraw; do not classify the top-level await as deadlocked.

### 3. Non-blocking startup state machine

`resumed()` creates the window/runtime, starts module evaluation, stores both in
`App`, requests the first redraw, and returns to winit. It must not synchronously
run warm-up to completion.

During `RedrawRequested`:

1. flush coalesced input/resize state;
2. drain the current rAF batch with the frame timestamp;
3. poll a bounded deno tick and the evaluation future;
4. synchronize DOM, submit/present at most once;
5. transition readiness or deterministic failure;
6. request another redraw only when rAF/game presentation requires one.

Async deno work wakes a coalesced `RuntimeWake` user event, which polls deno
without firing rAF. rAF itself requests `RedrawRequested`.

### 4. One frame owner

Remove the dual frame ownership where the host both drains rAF and directly
calls `renderEngineFrame()`. Production game modules should schedule through
the public `EngineRuntime`/rAF API. A transitional diagnostic module can use an
explicit scheduler adapter, but the browser host should have one presentation
mechanism.

Cache callable V8 functions instead of compiling string snippets through
`execute_script()` every frame.

### 5. Failure and budget policy

Propagate JS exceptions, rejected module evaluation, async-op failures, and
unhandled rejections immediately. On fatal startup/device failure, stop the
runtime and show the shell's fatal diagnostic; never silently continue or fall
back.

A host timeout is still needed because a referenced external task may legally
remain pending forever. The existing 30-second behavior is a policy choice:
make it explicit/configurable, pause active-time accounting while suspended,
and include phase, pending-rAF count, queue high-water, deno tick count, and
stalled TLA stack in the diagnostic.

## Rejected alternatives

| Alternative | Why rejected |
|---|---|
| Ignore `run_event_loop()` errors | Hides genuine runtime failures and corrupts failure semantics. |
| No-op waker + repeated future polling | Busy/manual polling; async completion cannot wake the host correctly. |
| Microtask rAF | Runs before repaint and can recursively starve host work. |
| `setInterval(__runNativeAnimationFrames, 1)` | Timer cadence is not presentation cadence; persistent timer prevents run-to-idle. |
| One timer/async op per rAF callback | Adds Promise/op overhead and loses shared-frame batching semantics. |
| `with_event_loop_future()` workaround | Avoids one idle error but still does not make rAF visible or frame-aligned. |
| Patch Three.js | Violates the shell's unmodified-module contract and leaves all other rAF/TLA users broken. |
| Make winit `pump_app_events()` subordinate to Tokio | Supported on desktop but explicitly caveated/non-portable by winit; unnecessary when EventLoopProxy can wake the normal handler. |

## Acceptance tests

1. A module with top-level `await new Promise(requestAnimationFrame)` remains
   pending before redraw and resolves after exactly one admitted redraw.
2. Three.js `compileAsync()` completes without ignored errors; input and native
   window events are serviced between yields.
3. Multiple callbacks receive one timestamp in registration order; callbacks
   requested during the batch wait for the next frame.
4. Cancellation is O(1), suppresses callbacks, and balances the external-op
   token when the queue becomes empty.
5. A throwing callback is reported while later callbacks still run.
6. Queue overflow is deterministic and increments stable telemetry.
7. A rejected TLA, unhandled rejection, and async-op error each reach the fatal
   path unchanged.
8. A genuinely unresolved TLA reaches the configured startup timeout with a
   useful diagnostic; suspension does not consume active timeout.
9. No busy polling or unbounded user events while idle; runtime wake events are
   coalesced.
10. The engine demo, native worker-composition probe, official Three examples,
    resize/suspend/device-loss paths, and a real-GPU frame/pixel gate pass.
11. Sealed frame soak shows fixed rAF storage, plateaued heap, stable queue
    depth/high-water, and no per-frame engine-authored allocation.

## Decisions required before implementation

1. **Browser-correct startup vs narrow compile workaround.** Recommended:
   browser-correct non-blocking startup and rAF. Alternative: a minimal
   `scheduler.yield()` shim plus synchronous startup; simpler but does not let
   winit input/presentation run during warm-up.
2. **rAF capacity/overflow contract.** Recommended candidate: fixed 1024
   callbacks per window, fail the excess request deterministically, expose
   high-water/overflow telemetry. Final number should be admitted by the full
   example/demo suite rather than assumed.
3. **Top-level-await timeout.** Recommended: configurable 30 seconds of active
   (not suspended) startup time, fatal with diagnostics. Alternative: permit
   indefinitely pending TLA while the window remains responsive.
4. **Scheduler API policy — decided 2026-07-25.** The correct-rAF gate exceeded
   90 seconds, so the user admitted a standards-shaped `scheduler.yield()` host
   continuation. `scheduler.postTask` and `TaskController` remain intentionally
   unsupported; their future addition requires a separate decision and gate.

## Sources

- deno_core 0.408 local source: `runtime/jsruntime.rs`, `ops.rs`,
  `ARCHITECTURE.md`.
- Deno discussion #25799, “Is it possible to embed deno_core with a
  self-managed event loop?” https://github.com/denoland/deno/discussions/25799
- Deno issue #27976, “Add (WebWorker) Priority Task Scheduler APIs”
  https://github.com/denoland/deno/issues/27976
- deno_web 0.286 local source: `02_timers.js`, `timers.rs`, `README.md`.
- Three.js r185: `src/utils.js` and WebGPU renderer `compileAsync()`.
  https://github.com/mrdoob/three.js/blob/r185/src/utils.js
- MDN, `Window.requestAnimationFrame()`:
  https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame
- MDN, `Scheduler.yield()`:
  https://developer.mozilla.org/en-US/docs/Web/API/Scheduler/yield
- WICG Prioritized Task Scheduling explainer/spec:
  https://github.com/WICG/scheduling-apis/blob/main/explainers/yield-and-continuation.md
- winit 0.30 local source: `platform/pump_events.rs`, `event_loop.rs`.
