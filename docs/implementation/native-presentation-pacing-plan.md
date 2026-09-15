# Native presentation pacing repair

Status: background behavior and repair scope approved by the user.
The native implementation and regression checks are in progress.
Real-driver acceptance remains open.

## Recommendation

Use Winit's existing Wayland frame callbacks for presentation pacing.
Separate runtime progress from redraw delivery in the current main-thread host.
Keep Three.js, V8, Blitz, and the shared WebGPU device under their current ownership.
Do not add a presentation worker or change the swapchain mode for this repair.

This is the smallest source-supported repair candidate, not a guarantee that a native driver call cannot block.
A real-driver acceptance check must establish whether it removes the reproduced wait.

## Evidence and source findings

The [control experiment](../benchmarks/native-paint-shell/profiling-pause-visibility-controls-1788731282/README.md)
reproduced a 997.555 ms interruption without a collector, recording, or transmitted batches.
The [stack check](../benchmarks/native-paint-shell/profiling-pause-visibility-stacks-1788731483/README.md)
places the wait in `wl_display_dispatch_queue` inside NVIDIA presentation on the main thread.
Moving the diagnostic window to an inactive workspace triggered the interruption.
The exact internal driver condition remains unknown.

The current source has two related problems:

1. `crates/afterglow-shell/src/main.rs::present_surface_inner` calls `GPUCanvasContext::present` without `Window::pre_present_notify`.
   Winit 0.30.13 documents this notification as the Wayland frame-callback integration point.
   Its Wayland event loop withholds `RedrawRequested` while a requested frame callback remains outstanding.
   The shell currently requests redraws at a monitor-derived interval without this integration.
2. `App::about_to_wait` includes pending native workers and assets in `needs_redraw`.
   `App::render` advances the runtime, and `App::user_event` ignores runtime wakes when rAF is pending after module evaluation.
   Native response rings need explicit polling. The current-thread Tokio driver also needs runtime turns.
   Thus adding frame callbacks alone could leave timers and native responses waiting for a redraw that the compositor withholds.

Related source locations:

- `crates/afterglow-shell/src/main.rs`: `present_surface_inner`, `op_present_surface`, `op_try_present_surface`, `render`, `user_event`, `about_to_wait`, resize and lifecycle handlers.
- `crates/afterglow-shell/src/rpc_bridge.rs`: `poll_async_workers`, `poll_native_assets`, and pending counters.
- `crates/afterglow-shell/raf.ts`: fixed 1,024-entry callback queue and aggregate external-op token.
- `crates/afterglow-shell/document.ts`: document surface acquisition and presentation.
- `crates/afterglow-shell/native_game.ts`: explicit initial presentation, followed by rAF rendering.
- `crates/afterglow-shell/vendor/deno_webgpu/canvas.rs`: acquisition, configuration, presentation, and texture retirement.
- Winit 0.30.13 `src/window.rs`: `pre_present_notify` contract.
- Winit 0.30.13 `src/platform_impl/linux/wayland/window/state.rs`: `request_frame_callback` permits one outstanding callback.
- Winit 0.30.13 `src/platform_impl/linux/wayland/event_loop/mod.rs`: frame-callback admission before `RedrawRequested`.
- wgpu-core 29.0.4 `src/present.rs`: `Surface::present` holds the surface presentation mutex and device fence write lock during driver presentation.

## Decisions before implementation

### PACE-001 — Background behavior

Recommended: rAF and surface rendering follow compositor redraw delivery.
When the compositor withholds redraws, retain pending callbacks but do not simulate display frames with a timer.
Continue bounded timer, promise, native RPC, diagnostics, and operating-system event processing.
Do not stop existing native workers or change audio policy.
Game updates attached to rAF will also wait. The shell must not silently move them to another clock.

Alternative: continue offscreen game rendering or simulation at a specified background rate.
That changes game behavior and needs a separate explicit contract, not a workaround in presentation code.

Focus is not visibility. A visible window without keyboard focus must continue rendering.
An absent `Occluded` event is not proof that a window is visible.
Do not infer `document.hidden` from focus or a missing frame callback.

### PACE-002 — Scope and ownership

Recommended: retain the single JavaScript/render host and existing device, surface, and texture ownership.
Retain the current present mode and frame-latency setting.
There is no asset-format change, new service, dependency, RPC transport, or public-web behavior change.

Alternative: require progress despite an arbitrarily blocked graphics driver call.
That needs a separate graphics-isolation design and explicit approval if the pacing repair cannot meet the acceptance tests.
It is not equivalent to putting `surface_present` on a thread.

### PACE-003 — Failure and timing policy

Recommended: retain fatal device-loss behavior and bounded existing surface recovery.
Do not add a WebGL/software fallback, ignored presentation errors, infinite retries, or a timeout that cannot cancel the driver call.
Keep startup readiness dependent on actual startup and presentation, not merely a queued redraw.

Keep the existing finite startup limit for this repair.
Hidden startup is a required test: record whether it completes on restore or reaches the existing limit.
Suspending that limit while the compositor withholds frames would be another behavior decision.

The proposed responsiveness thresholds below are acceptance targets, not new runtime timeout constants.
The user approved the recommended background behavior and repair scope before implementation.

## Implementation sequence

### 1. Separate runtime turns from frames

Change the existing `App` scheduling paths. Do not create a general scheduler service.

- Keep one coalesced runtime-wake flag and one runtime deadline.
- Advance native response polling, Tokio, Deno, and module evaluation independently of `RedrawRequested`.
- Use at most one maintenance turn per existing host interval while work remains.
- Preserve a bounded polling opportunity for native rings. A worker `unpark` is not a Winit event-loop wake.
- Preserve a polling opportunity for timers in the current-thread Tokio runtime, including when no native RPC is pending.
- Retain real pending runtime wakes when rAF is pending. Do not retain the current unconditional early return.
- Do not let the aggregate rAF external-op token produce an immediate, self-sustaining wake loop.
- After each turn, coalesce further work into the next admission opportunity. Do not replay missed intervals.
- If there is no pending work or deadline, use `ControlFlow::Wait`.
- A runtime-only turn must not call the document presenter, acquire a surface texture, or drain the rAF queue.

The implementation must distinguish runtime progress from presentation demand without scanning unbounded queues.
Reuse pending counters, `RuntimeWake`, and the fixed rAF queue.
Do not remove the external-op token without proving that top-level rAF awaits still keep module evaluation valid.

Arbitrary application JavaScript can still occupy the main thread.
A bounded host turn is not a preemption guarantee for user code.

### 2. Connect the presentation notification

Place `window.pre_present_notify()` in the common native presentation path.
Call it after frame/HUD command submission and immediately before the actual surface presentation.
Use the existing retained native window rather than another independent ownership layer.

- Both presentation ops must reach this point.
- Include document mode, Three.js compatibility mode, and the initial native game presentation.
- Do not request a frame callback for a missing context or an absent acquired texture.
- Keep the current synchronous texture lifetime and error propagation.
- Do not add a second raw Wayland frame-callback implementation.

One outstanding shell redraw request is sufficient until Winit delivers it.
Runtime deadlines must not repeatedly force surface work while that request waits.
On Wayland, Winit's callback controls redraw delivery.
On platforms where `pre_present_notify` is a no-op, retain the existing monitor-interval admission limit.
Do not replace presentation backpressure with unconditional `ControlFlow::Poll`.

### 3. Preserve lifecycle and input progress

- Process resize notifications independently of redraw delivery, retaining only the latest pending dimensions.
- Apply the final nonzero surface size at the next safe render boundary before acquisition.
- Do not acquire a surface for a zero-size or genuinely suspended window.
- Make restore/resume request a redraw without requiring a previous frame to perform the recovery.
- Do not treat inactive-workspace pacing as operating-system suspension.
- Keep close, focus, keyboard, clipboard, and IME handling independent of rAF.
- Flush retained pointer samples during bounded runtime maintenance when necessary, without drawing or changing their order/timestamps.
- Retain the existing pointer sample capacity and overflow behavior.
- Keep device-loss detection and shutdown reachable when no redraw arrives.

Check all three startup paths against the lifecycle state and readiness flag.
The prior document trace showed `CanvasReady` with `pending_resize=true` after paint checks passed.
Do not assume that a `Running`-only guard correctly covers document mode.
If readiness bookkeeping prevents this repair, correct that inconsistency with a focused regression test rather than ignoring transition failures.

On restore, use the current monotonic timestamp for the next rAF batch.
Do not synthesize callbacks for the hidden period or reset game clocks in the host.

### 4. Measure the candidate before adding more architecture

Repeat the existing visibility experiment with capture both connected and absent.
Measure runtime progress separately from frame intervals.
A long rAF interval while redraws are withheld is expected under PACE-001 and is not itself a failure.
A long synchronous presentation wait or delayed ready RPC completion is a failure.

Include the hide/restore transition itself, not only the stable hidden period.
Frame callbacks cannot prove that the first driver call after a visibility change will not block.
If that call still produces the reproduced stall, this candidate has not passed.

Stop and retain the failing evidence if the driver still blocks the main thread.
Do not add a sleep, increase a timeout, switch to X11, or change present mode to conceal the result.
Reopen PACE-002 before a graphics-ownership change.

## Why not a presentation thread first?

The current wgpu-core presentation path holds a device fence write lock during the driver call.
Queue submission and device maintenance also use this lock.
A thread that merely calls `surface_present` can move the same wait into the next main-thread GPU operation.
Surface resize, discard, and destruction also need correct coordination with an in-flight presentation.
V8 handles and `Rc<RefCell<SurfaceData>>` cannot simply cross the thread boundary.

A correct isolation design would need explicit GPU-operation ownership, bounded handoff, texture generations, and shutdown ordering.
It must account for device polling, garbage collection, resize, and all other shared-device callers.
That is not justified before testing the missing native pacing integration.

Changing FIFO to mailbox/immediate is also not a correctness proof.
It changes latency, power, and possibly tearing behavior, and support differs by platform.
Disabling rendering on focus loss is incorrect for visible unfocused windows.

## Regression and acceptance tests

### Deterministic tests

Extend existing shell tests and add a small host-scheduling state test where necessary:

1. Withheld redraw plus pending rAF: timers and ready native responses progress without any surface call.
2. Repeated Deno wakes: at most one admitted maintenance turn per interval, no immediate wake loop.
3. Idle runtime: no periodic redraw or runtime busy loop.
4. Outstanding redraw: at most one queued shell request, with no lost redraw after restore.
5. rAF ordering: retain pending callbacks, cancellation, nesting, capacity, and one batch per admitted frame.
6. Top-level rAF await: startup remains pending correctly and completes after redraw delivery.
7. Presentation ordering: submit, notification, present, then existing texture retirement, once per acquired frame.
8. Missing texture/context and presentation error: no false successful frame or readiness signal.
9. Resize and zero-size restore: coalesce dimensions and recover without a redraw-dependent deadlock.
10. Hidden close/device loss: shutdown remains reachable and keeps the window alive until surface destruction.
11. Pointer events: retained samples and button transitions keep their order without mandatory rendering.

Use `crates/afterglow-shell/tests/{raf,document,scheduler}.test.ts` and the existing Rust host tests.
Do not create a new test framework or public scheduling API for these checks.

### Real platform checks

Extend the existing owned-window visibility driver and native paint probe.
Add a timer/ready-response progress measurement that does not depend on rAF or an active collector.
Use one application monotonic clock for those measurements.
Record focus, visibility actions, runtime turns, redraw delivery, and surface call duration separately.

Proposed acceptance:

- Five fresh paired launches with and without capture on NVIDIA/Wayland.
- At least 20 hide/restore cycles per mode, including a move immediately after redraw admission.
- No reproduced one-second synchronous host stall, including transition frames.
- In the light diagnostic workload, no timer lateness or already-ready response dispatch delay above 100 ms.
- Report foreground p50/p99/max against matched unchanged runs. Do not accept a material regression under the existing frame-budget gates.
- Hidden frame production may stop. Runtime maintenance remains bounded, with no growing callback, task, ring, or memory backlog.
- Paint, undo/redo, save, and restore checks pass. No lost records or GPU validation errors.
- Visible-but-unfocused, hidden startup, minimize/restore, resize while hidden, close while hidden, and device loss are covered.
- Repeat on Radeon/Wayland and native X11 before cross-platform acceptance. Mark unavailable hardware checks as open, not passed.
- Complete a 30-minute hide/restore and native-response soak. This does not replace the existing broader release soaks.

The 100 ms value is a proposed diagnostic failure threshold, not a game frame budget.
No claim of physical input-to-pixel latency follows from these measurements.

## Files and documentation

Expected runtime changes are concentrated in `crates/afterglow-shell/src/main.rs` and existing shell tests.
Change `raf.ts` or the RPC bridge only if the scheduling tests show that their existing contracts need adjustment.
Keep Deno canvas and wgpu ownership unchanged unless a specific acceptance failure makes a change necessary.
Do not add paint-specific runtime branches or repair only the diagnostic page.

With implementation, update:

- `docs/api/afterglow-shell.md` for runtime turns, compositor-paced rAF, lifecycle, and limits.
- `docs/api/telemetry.md` for the distinction between withheld redraws and blocked runtime work.
- `book/src/reference/paint-editor.md` and `book/src/reference/telemetry.md` for verified user-visible behavior and evidence.
- `docs/implementation/shell-promotion-plan.md` and this plan for acceptance status.
- `docs/benchmarks/native-paint-shell/` for untraced results and any targeted trace evidence.

Build through `nix-shell shell.nix`, with explicit `CARGO_BUILD_JOBS=$(nproc)` for native builds.
Run shell tests, relevant TypeScript checks, the native release build, real platform checks, mdBook, and `git diff --check`.
No completion claim is valid while a required acceptance test fails.
