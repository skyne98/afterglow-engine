# Engine Diagnostics
## Causal flight recorder, profiler, and inspection protocol — design v0.1

**Status:** proposed architecture and API contracts, not an implemented library. `diagnostics.d.ts`, `example.ts`, and `contract.test.ts` pass TypeScript 5.8.3 in strict mode. This checks type consistency, not runtime behavior, completeness, overhead, or integration with your engine. External API references were checked on 5 September 2026. Pin adapter implementations to the versions you actually ship.

## 1. The contract

Build one diagnostic model for your engine and native shell. Provide a small producer API, an extensible provider interface, and a separate authenticated inspector API.

The contract is **no silent unknowns**, not “record everything without cost.” Every explanation must show its evidence, missing signals, sampling, losses, and timing uncertainty. Native access improves coverage, but it does not make hardware state, driver behavior, every race, or every earlier instruction observable. The web adapter must use the same semantics without pretending it has native visibility.

The system should answer these questions:

- What missed its deadline, and which work actually delayed it?
- Was a task executing, ready but unscheduled, waiting on another task, waiting for a resource, or in an unknown state?
- Which change caused this component, layout subtree, resource upload, or GPU pass to run?
- Which resources remain live, who owns them, and what evidence explains their lifetime?
- What state and code produced an error, and which recorded inputs can reproduce it?
- What evidence is absent, and which additional probe would most reduce the uncertainty?

Do not equate wall time with CPU time, a parent span with a dependency, a present request with display, a callback with the underlying completion instant, or missing data with zero.

## 2. Architecture

```text
Game JS/WASM     Shell Rust     Native workers     Runtime / OS / GPU probes
      \              |                |                       /
       +-------------+----------------+----------------------+
                            |
             Registered sites + producer-local binary writers
                            |
              Bounded per-thread / per-realm record buffers
                            |
          Native collector process OR browser collector worker
                            |
       Versioned trace store + resource history + dependency graph
                   /                 |                  \
          Engine inspector      Perfetto export      Offline queries / CI
                   |
          Authenticated command broker
                   |
       Owner-thread safe points / V8 inspector / native debug adapter
```

Keep event production independent of storage, analysis, exports, and the UI. A stopped or slow inspector must not block the game. Prefer a separate native collector process for crash resilience. Use a collector worker on the web, subject to browser scheduling and lifecycle limits.

A producer belongs to a process, realm/isolate, thread or executor, and tenant/origin. A track represents an execution lane, a logical operation lane, or a device queue. Do not mix concurrent operations on one nested synchronous track.

The UI and collector are also workloads. Give them their own tracks. A separate process does not stop them from competing for CPU, memory bandwidth, or a GPU.

### Public surfaces

| Surface | Purpose | Caller |
|---|---|---|
| `Diagnostics` | Systems, sites, events, metrics, resources, epochs, context, links | Engine/game code |
| `ProbePlugin` | Platform sources, clock mappings, schemas, capabilities | Trusted host adapters |
| `InspectorClient` | Captures, analysis, inspection, commands, debug/replay control | Authorized developer tools |

Do not give the inspector client or native provider privileges to ordinary page code. Registering an inspector or command exposes a service; it does not authorize callers to invoke it.

## 3. Stable semantic core

Keep the record model small. A new subsystem should extend schemas and views, not require a new tracing engine.

| Primitive | Meaning | Typical records |
|---|---|---|
| Slice/span | Synchronous elapsed interval on one execution track | Physics step, native call, style matching |
| Operation/task | Logical lifetime that may span waits and tracks | Asset load, worker job, frame construction |
| Event/log | Fact at an instant, with bounded structured fields | Error, invalidation, milestone, cache miss |
| Metric | Counter, signed counter, gauge, or histogram | Bytes copied, queue depth, task latency distribution |
| Sample | Statistical observation with sampling metadata | JS/Rust stack, allocation stack, hardware counter |
| Resource | Stable identity, revision, ownership, and lifecycle | Entity, buffer, texture, component, lock, socket |
| Link | Typed relationship between records/resources | Unblocks, requires, invalidates, owns, aliases |
| Snapshot/artifact | Bounded state or a separately stored large object | Heap snapshot, scene state, shader source, checkpoint |
| Health | Coverage and recorder integrity | Lost records, disabled source, clock error, buffer pressure |

A structured log is an event with severity, schema, source, and context. Do not maintain unrelated log and trace worlds.

### Common identity and metadata

Each retained record resolves to a session, producer, sequence number, schema version, raw timestamp and clock domain, system/site ID, source/build identity, quality information, and optional logical context. Encode common metadata through descriptors and dictionaries instead of duplicating strings in each record.

Use globally unambiguous session/producer identities and producer-local IDs. Use generations for reused handles. A raw pointer or reusable integer alone is not a resource identity. Keep destroyed-resource tombstones for the retained capture window.

Contexts contain diagnostic IDs and bounded correlations, not credentials or arbitrary object graphs. Propagate them over worker messages, native operations, IPC, I/O, and service boundaries. Use links for fan-in and multiple causes. A parent is an ownership/organization relationship; it does not prove the parent waited for the child. OpenTelemetry context and links provide useful interoperability concepts, but do not use remote-service span semantics as the entire frame profiler. [S1]

### Source identity

Record content-addressed build IDs, module IDs, code hashes, source maps, native symbol references, WASM function information, and pipeline/shader IDs. Resolve expensive symbols after capture. Retain hot-reload/JIT mapping epochs so an old sample cannot resolve against new code.

Link generated WGSL to its generator and material/node graph where your build pipeline can provide that mapping. Do not claim source-level GPU debugging when only a shader label and generated source are available.

## 4. Everyday API

Register a system and sites once. Emit compact records on the hot path.

```ts
const physics = diag.system({ id: "engine.physics", schemaVersion: 1 });

const step = physics.span("step", {
  fields: { bodies: field.u32({ privacy: "public" }) },
  detail: "flight",
});

step.run({ bodies: bodyCount }, () => world.step(dt), { context });
```

`run` measures synchronous elapsed time. It closes on normal return and exceptions. It rethrows the application error. The contract rejects Promise-returning callbacks; the type tests check this. A native implementation uses a scope guard. Generated JS instrumentation can lower this to an enabled check and `try/finally` without a callback allocation.

The ergonomic object API is not an allocation-free promise. Arguments are evaluated before a normal function call, even when recording is disabled. Provide an explicit `enabled` check for expensive fields and a build transform/macros for high-frequency sites. Compile-time disabled builds may erase instrumentation only when doing so preserves application side effects.

### Async operations

```ts
const rebuild = physics.task("rebuild", {
  fields: { seed: field.u32({ privacy: "public" }) },
});

const result = await rebuild.run({ seed }, async task => {
  const job = workers.submit({ seed }, diag.context.inject(task.context));
  const vertices = await task.wait(job, { kind: "dependency" });
  return upload.run({ bytes: vertices.byteLength }, () => uploadMesh(vertices), {
    context: task.context,
  });
}, { context });
```

`workers.submit` in this example is an engine adapter contract, not a built-in browser method. It must return an instrumented operation reference and a result Promise. The full example defines the interface.

`task.run` records logical lifetime. It does not claim continuous execution. `task.wait` records the await interval and known dependency. It does not infer when the producer became ready or when the consumer was scheduled after completion. Those require scheduler/transport hooks.

`task.waitUnknown` accepts an uninstrumented Promise and leaves the upstream cause unknown. `waitAll` records all required dependencies. `waitAny` records the winner; losing work is not automatically cancelled or added to the critical path.

Do not use a global mutable “current task” across `await`. Explicit context works on native and web. A native runtime may provide async-local propagation, but its propagation must be tested through timers, promises, worker crossings, and re-entry. In Rust, use poll-scoped instrumentation such as `Future::instrument`; do not hold an entered `tracing` span across an await. [S2]

### Level of adoption

Basic integration needs a system name, a few spans, and counters. Add task links at async boundaries. Add resources, invalidation causes, inspectors, and replay only where they provide value. A plugin without a custom UI still gets timeline, metrics, logs, and resource tables from its schemas.

## 5. Scheduling, waits, and dependencies

Instrument schedulers, channels, executors, lock wrappers, and queues once. This is more valuable than asking every game system to reconstruct scheduling history.

The useful lifecycle is:

```text
created -> enqueued -> ready -> running -> suspended/waiting -> ready -> ...
                                                        -> completed/cancelled
```

Keep actual queue policy, capacity, priority, deadline, selected consumer, work-stealing events, and cancellation outcomes. “Enqueued” and “ready” are distinct when prerequisites are unresolved. Polling is not equivalent to CPU running time. A thread can be descheduled during a poll or synchronous slice.

For a cross-worker operation, capture send, serialization, ownership transfer/copy, receive, enqueue, ready, execute/poll, result send, result receive, and consumer resume. Keep each stage only when observable. Count bytes at the site where copies actually occur, not from guessed payload sizes. Distinguish JS-to-Rust copies, message cloning, native staging copies, CPU-to-GPU uploads, and GPU-to-CPU readbacks.

Instrument contention separately from work: lock requested/acquired/released; semaphore waits; bounded queue backpressure; thread-pool starvation; synchronous bridge blocking; and I/O readiness. Preserve the lock owner or dependency when known. Do not call a heartbeat failure a proven deadlock.

The general relationship graph may contain ownership cycles and correlation links. The latency graph must contain only valid precedence/dependency edges over interval fragments. Do not run a critical-path algorithm over arbitrary resource links or span nesting. Inferred edges remain visibly different from directly instrumented edges.

### Time accounting

For a selected logical operation, expose elapsed lifetime, observed execution slices, observed ready delay, observed blocked time, and unknown intervals. Do not sum parallel work as wall time. Compute exclusive synchronous duration by subtracting the union of nested intervals on the same track, not by subtracting arbitrary async children. Keep total work and critical-path latency as different views.

Scheduler data can distinguish CPU execution from ready-but-unscheduled and blocked time. Without it, show elapsed slices, not invented CPU utilization. Perfetto scheduling sources and platform trace systems can supply this additional evidence on supported native hosts. [S3]

## 6. Time, clocks, frames, and GPU work

### Clocks

Store raw monotonic ticks and their clock identity. Do not use `Date.now()` to compute durations. Do not store epoch nanoseconds in a JavaScript `number` and assume exact integers.

Each clock descriptor states tick frequency, resolution, possible wrap, and mapping epochs. Map clocks with an anchored affine relation:

```text
targetTicks ~= targetAnchor + rate * (sourceTicks - sourceAnchor)
```

Store uncertainty and validity intervals. Recalibrate for drift and record suspend/resume, process restart, or device reset. The canonical store retains raw ticks so improved calibration can be applied later. Perfetto supports this multi-clock model. [S4]

On the web, use the High Resolution Time model (`performance.now()` and the realm's `timeOrigin`) and preserve the effective precision. Different realm origins are not identical timestamps. Validate mappings across workers instead of subtracting unrelated local values. [S5]

### Epochs, not one global frame number

Represent input, simulation, render, submission, and presentation as separate epochs. Associate them explicitly:

```text
input events -> simulation ticks -> scene revision -> render epoch
             -> GPU submissions -> present request -> presentation feedback
```

There may be multiple simulation ticks per rendered frame, multiple submissions per render, and multiple windows. Input can be coalesced or skipped. Associate the actual state revision and input sequence consumed, not just the latest frame counter.

Record scheduled time, actual start, deadline, finish, visibility, occlusion, refresh settings, target cadence, and observed presentation feedback. Treat variable refresh, deliberate frame limiting, background throttling, and pause state explicitly. A target budget is not evidence that the display actually presented at that cadence.

### GPU rules

Separate CPU encoding, queue submission, GPU intervals, completion notification, present request, and display feedback. A callback indicating queue completion is a CPU observation after GPU work; it is not a precise GPU execution timer. Timestamp-query support is optional and must be requested/negotiated when the device is created. [S6]

Use a bounded query pool and asynchronous readback ring. Resolve timestamps after submission without adding a per-frame blocking wait. When the pool is full, skip queries and report the gap. Bind delayed results to their original device generation, queue, submission, pass, and render epoch, not to the frame that received the results.

Without a valid CPU/GPU clock mapping, show GPU durations on a separate relative timeline and retain logical submission links. Do not align a GPU timestamp with a JavaScript completion callback as though they were simultaneous. Preserve timestamp quantization/resolution and invalid query states. A zero after quantization is not evidence of zero cost.

Distinguish GPU execution duration from GPU busy percentage, queueing delay, memory bandwidth, occupancy, and shader bottlenecks. Those need different native/vendor sources, when available. Resource descriptor sizes are not actual physical VRAM residency.

Never label a `requestAnimationFrame` interval or `present()` call as measured display latency. End-to-end input-to-photon latency needs presentation/display evidence beyond ordinary application timing.

## 7. Providers for your stack

The rows below describe instrumentation to implement. They are not claims that every upstream library already exports these events.

| Provider | Capture | Main diagnostic use |
|---|---|---|
| Shell and frame scheduler | Windows, tabs, isolates, tasks, timers, microtask drains, input routing, deadlines, surface acquisition, visibility and throttling | Deadline misses, event-loop starvation, input delay |
| `deno_core` bridge | Operation identity, sync/async completion, calls, actual copied/borrowed bytes, errors, cancellations, re-entry | Too many crossings, copy amplification, native blocking |
| V8/runtime | JS samples, exceptions, compilation/load phases, heap profiles, supported GC/JIT hooks, async IDs | JS CPU, allocation churn, pauses, startup cost |
| Worker/executor | Queue lifecycle, wakes/polls, readiness, message stages, priority, dependencies, result age | Queue starvation, load imbalance, backpressure |
| Blitz DOM/style/layout/text/paint | Dirty causes, touched nodes, style matching, layout, text shaping, font/cache activity, paint work | Invalidations, layout repetition, text/paint cost |
| Vue | Component/effect IDs, tracked dependency revision, trigger reason, update batch, render/patch, host mutations | Why a component updated and the downstream cost |
| Three.js | Traversal, culling, sorting, material variants, draw preparation, draw/instance counts, uploads, cache hits, pipeline preparation | CPU render cost, draw overhead, resource churn |
| wgpu/WebGPU | Resource lifetimes, descriptors, validation/error scope results, command encoding, submissions, pass queries, device loss | CPU/GPU separation, stalls, invalid use, delayed release |
| Memory | Logical ownership, JS/native allocations, sampled allocation stacks, pool statistics, committed/resident memory where available | Growth, churn, retained resources, fragmentation candidates |
| Network | Request/task links, connection stages when available, status, streaming progress, cache and retry decisions | Loading latency and hidden dependencies |
| Files/assets | Reads, decompression, decode, parse, upload, cache key/hit/eviction, source hash | Loading spikes, duplicate work, cache misses |
| Game systems | ECS queries, transforms, animation, physics broad/narrow phase, navigation, streaming dependencies | System-specific work and waits |
| Audio/media | Buffer depth, decode/mix stages, sample clock, underruns, callback deadline | Audible glitches not explained by render FPS |
| Operating system | CPU sampling/scheduling, faults, I/O, pressure and power/thermal signals when exposed | Descheduling, resource contention, external load |
| Recorder itself | Writer cost, flush time, buffer fill, loss, sampling, transport backlog, inspector work | Observer effects and false negatives |

`deno_core` supplies an operation metrics factory and a V8 inspector interface. Use them as adapter entry points, then verify coverage for your pinned version and fast/async operation paths. The inspector is not the full Chromium DOM/Network/Performance implementation; Blitz and shell domains need your own providers. [S7]

Blitz separates DOM/style/layout/event work from paint and shell integration. Instrument those ownership boundaries rather than treating all UI work as one `render` call. [S8]

Vue's `onRenderTracked` and `onRenderTriggered` hooks are development-only. Production cause tracing needs explicit engine/compiler hooks or a documented reduced mode. Never retain raw reactive targets indefinitely just to identify them. [S9]

Three's renderer exposes statistics and an inspector integration point. Adapt the shipped version and retain renderer/pass/material IDs through to wgpu. Do not double-count work because Three and wgpu both report the same upload or submission; link the layers. [S10]

Use native error scopes and device-loss/error handlers without changing their ownership or swallowing errors. Debug capture around GPU errors must be scoped through your renderer; blindly pushing asynchronous nested error scopes from unrelated tasks can associate errors with the wrong operation. [S11]

## 8. Capability model: native and web

A capability is not a Boolean. Describe its source, scope, activation state, permissions, resolution, quality, and limitations. Capability records change when devices, modes, permissions, or agents change.

Useful states are enabled, available-but-not-enabled, unsupported, denied, disabled, and failed. A measurement separately reports direct, sampled, estimated, bounded, or missing with a reason.

| Signal | Native shell | Ordinary web page |
|---|---|---|
| Your instrumented spans, tasks, links, resources | Available through your implementation | Same semantic API |
| Precise owned scheduler lifecycle | Through native scheduler integration | Your workers/tasks only; browser scheduler internals are limited |
| JavaScript CPU sampling | V8/native debugger integration | Optional self-profiling support, or a separate external DevTools capture |
| Rust/native stacks and scheduling | OS/runtime integration, subject to permissions | Not generally available to page code |
| GPU pass timestamps | Supported features/backend permitting | Optional `timestamp-query`; preserve effective resolution |
| GPU hardware counters/residency | Driver/vendor/OS capability dependent | Not a portable page capability |
| Style/layout/paint internals | Blitz hooks you implement | Browser performance entries where exposed; less detail |
| JS heap and native memory | Inspector/allocator/OS providers at their stated scope | Logical resource accounting; optional browser memory estimates |
| Actual presentation feedback | Platform-specific integration, not universal | No general exact physical-display timing contract |
| JS breakpoints/locals | V8 inspector | External browser debugger; not an unrestricted in-page API |
| Crash/hang capture | Independent native collector/watchdog | Best effort; page/worker lifecycle limits remain |

For browser performance entries, feature-detect supported entry types. Observe applicable navigation/resource/event timing, long tasks, and long animation frames. Keep your own frame deadline accounting: a browser long-task/long-animation-frame threshold is not a complete detector for game-frame misses. Event Timing and Long Animation Frames expose only their specified scope and attribution. [S12]

Do not assume JavaScript self-profiling exists in every browser. It is a conditional source with its own policy. Memory measurement is likewise an optional estimate with scope and isolation requirements; do not replace a missing measurement with `performance.memory` and call it portable total memory. [S13]

Use `SharedArrayBuffer` only under the platform's required conditions. Otherwise use bounded reusable `ArrayBuffer` batches transferred to a collector worker. Transfer changes buffer ownership; never read a detached buffer. A worker provides a place for analysis, not a guarantee of CPU availability. [S14]

Native OS integrations can use Linux perf/scheduling sources, Windows ETW, and Apple Instruments/signposts. These are separate providers with independent privilege and coverage checks. [S3, S15]

## 9. Flight recorder and overhead control

### Modes

| Mode | Policy |
|---|---|
| Off | No telemetry recording; instrumentation does not change the application's error/cancellation policy |
| Flight | Bounded rolling history: major intervals, frame/task/queue skeleton, essential context, counters, important events; sampling only where enabled |
| Trace | More subsystem detail, broader sampling, selected resource and invalidation histories |
| Deep | Short, explicitly expensive sessions: allocation stacks, dense UI causes, heap snapshots, richer GPU/OS capture |

Replay is a separate capability and policy, not simply “more trace detail.”

All sizes, durations, and frequencies in `example.ts` are illustrative policy values. A request for eight seconds of history in 32 MiB is not a guarantee when the event rate exceeds capacity. Report the actual retained window and losses.

### Writer design

Use per-producer bounded rings or reusable chunks. Use local sequence numbers, registered sites, interned strings, compact typed fields, and delta timestamps. Prefer no blocking lock or allocator in a registered native hot path. In JS, use a generated binary writer for very hot sites. Batch JS-to-native submission; do not make one native operation per profiler event.

Move symbolization, compression, SQL, file/network export, and expensive serialization off the application hot path. Use a recursion guard in allocator/runtime probes. The collector must not profile its own allocations recursively through the same allocator hook.

Bound record size, field size, resource histories, unique names, metric series, live tasks, attachment bytes, and producer rates. Unknown or excessive schemas are rejected and counted. Do not interpolate entity IDs or URLs into metric labels. Use resource references and trace exemplars instead.

### Loss and consistency

Reserve capacity for clock/schema descriptors, sequence ranges, loss counters, mode transitions, and minimal dependency metadata. All capacity remains finite. When it is exceeded, expose that fact; reserved metadata is not an unlimited reliability guarantee.

Make each exported chunk decodable: include the relevant dictionary checkpoint and active-operation/track state, or an explicit dependency on a retained metadata chunk. A circular buffer must not overwrite the only copy of a name or an open-span definition.

Track missing sequences, overwritten windows, dropped samples, query exhaustion, truncated fields, missing predecessors, and open intervals. A capture beginning during an operation contains a left-truncated interval; a capture ending before completion contains an open/right-truncated interval. Neither is a zero-duration task.

When overloaded, aggregate/drop low-priority detail first, then report any loss in the required causal skeleton. Keep preallocated health counters. Do not silently switch modes. Tail-triggered deep capture cannot recover detail that was never recorded before the trigger.

### Triggers and crash handling

Support deadline misses, queue-age thresholds, allocation-growth thresholds, errors, device loss, invariant failures, and missed heartbeats. Each trigger has pre-roll, post-roll, memory limit, cooldown, maximum captures, and an explicit mode change.

A native watchdog must not depend on the hung isolate or its event loop. An independent collector can preserve already-published data when a process fails. Crash paths should use only appropriate preallocated/async-signal-safe mechanisms. Hard kills, power loss, device faults, and OOM can still truncate a capture.

Capture completion waits only for a bounded drain interval. Late GPU/query/worker records either join a still-open capture or appear as missing/late data. Never block the game indefinitely to make a diagnostic file look complete.

### Measure the observer

Record events/s, bytes/s, ring occupancy, drops, writer latency distribution, collector CPU, profiler allocations, export time, and inspector work. Compare representative workloads with off/flight/trace/deep modes. Sampling and instrumentation can change scheduling, cache behavior, and JIT behavior. No universal overhead percentage is assumed.

## 10. Live debugging and inspection

### State inspection

A subsystem registers paginated, bounded snapshot readers. Dispatch a reader on the owner thread at a safe point. Return revision, observation time, consistency, cursor, and truncation state.

Do not read mutable native state from an arbitrary inspector thread. Do not traverse arbitrary JavaScript getters while pretending to perform a harmless snapshot. Prefer explicit plain-data projections. A multi-system snapshot can be partial; it is not automatically a globally atomic view.

Use generational handles and weak registries where possible. Observability must not keep the entire scene alive. Retain immutable snapshot data, not live object references. Distinguish instrumented ownership graphs from actual heap-retainer graphs.

Useful built-in inspectors include scene/ECS state, DOM/style/layout boxes, Vue components/dependencies, worker queues, locks, GPU resources and bindings, shader/material/pipeline state, asset caches, sockets, and profiler health.

### Commands

Register typed, validated commands with required privilege and safe point. Examples: inspect entity, reset a system, invalidate a cache, enable an overlay, step a simulation tick, or run a controlled fault plan.

Commands must carry a request ID, target generation, expected state revision where applicable, timeout, cancellation, and audit identity. Validate before enqueueing and again at the safe point. Deduplicate retries by request ID. Distinguish accepted, applied, rejected, cancelled, and outcome-unknown when a connection fails. A dry run must not apply the mutation.

Do not execute arbitrary JavaScript from an unauthenticated diagnostics socket. The normal interface has a closed command registry. Optional debugger evaluation requires a separate explicit privilege and developer mode.

### Breakpoints and pauses

Provide source breakpoints through the appropriate debugger, and semantic break conditions through event schemas: invalid resource lifecycle, NaN state, unexpected revision, repeated invalidation, or queue threshold.

A semantic break request can pause cooperative participants at safe points; it does not travel back to the event before it was observed. Return which participants paused, which did not, and whether GPU work remains active. A frame that spans a debugger pause must be marked and excluded from normal performance comparisons.

Use V8 inspector for supported JS debugging/profiling functions and a Debug Adapter Protocol bridge for the selected native debugger. Sharing a timeline and IDs does not by itself create a perfectly merged JS/Rust call stack or a universal cross-language stepping debugger. [S7, S16]

### Security and privacy

Keep the control channel local/loopback or use authenticated encrypted remote transport. Localhost alone is not authorization. Enforce origin allowlists, session tokens, leases, and least privilege. Separate read, capture, heap, source, mutate, pause, fault, replay, and evaluate permissions.

Keep tenant/origin namespaces separate. A game's diagnostics object must not expose another website's state. Filter sensitive fields before retention, not only at export. Resolve unclassified fields and metrics from the bootstrap policy. The example marks nonsensitive counts as public; unclassified inspector output defaults to sensitive and is not automatically persisted. Do not collect passwords, authorization headers, body payloads, or arbitrary input text by default. Heap snapshots, full sources, screenshots, recordings, and replay I/O need explicit policy and limits.

Validate incoming record sizes, schema IDs, clocks, handles, and ownership. Limit decompression, attachments, graph expansion, and queries. Keep offline SQL read-only with a deadline and row/memory limits; disable arbitrary file access and extensions.

## 11. Resource and memory accounting

Track create, logical release request, and observed release as separate lifecycle events. A JavaScript `dispose` request does not prove a driver allocation is already gone. GPU work can retain resources after the application releases its handle.

Represent physical/logical backing resources and aliases. Charge shared backing storage once. Distinguish requested size, allocator usable size, committed memory, resident memory, JS heap used/reserved, and GPU allocation estimates. Do not add overlapping counters into a fake total.

Record allocation site or sampled stack, owner, asset/source, generation, pool, revision, last-use evidence, pending submissions, and release dependencies when known. Label unknown owners rather than attributing them to the currently running system by guess.

Leak analysis should distinguish live growth, retained pools, legitimate caches, fragmentation candidates, and delayed GPU release. Compare at equivalent workload checkpoints after defined settling conditions. Increasing RSS alone is not proof of a leak. A “why alive?” view must say whether the path came from a heap snapshot, an explicit ownership link, or an inference.

## 12. Analysis and the inspector UI

### Questions, not just charts

Offer `explain(target, question)` for latency, total work, memory, and correctness. Start from a selected deadline, frame, task, resource, or error. Walk backward through instrumented dependencies. Rank contributions to the selected outcome, not just the longest bar anywhere in the capture.

Each finding contains:

```text
classification: observed | inferred | unknown
statement
supporting record/resource/source references
alternative explanations
missing signals and clock/loss limitations
next useful probe, expected overhead, and why it helps
```

Keep rule versions and query inputs in the result so an explanation can be reproduced. A language-model assistant may explain an evidence graph, but it should not be the only implementation of critical-path calculation, data validation, or diagnostic rules.

### Hypothetical report

The following numbers are illustrative, not measurements from your engine:

```text
Target: render submission #841
Scheduled-start to submission: 31.6 ms; deadline budget: 16.7 ms
Critical path:
  2.1 ms  main-thread work before worker dependency
 21.8 ms  await collider job
          17.9 ms ready in queue
           2.7 ms worker execution interval
           1.2 ms result delivery/resumption interval
  5.5 ms  upload and command encoding
  2.2 ms  other measured serial work

Observed: render required collider job #443 before submission.
Observed: job #443 was ready behind navigation job #442 on its worker queue.
Inference: queue head-of-line blocking is the main recorded delay to submission.
Unknown: why navigation job #442 was slow; detailed samples were not retained.
Unknown: actual display time; no presentation-feedback provider was enabled.
Next probe: worker CPU/scheduling sampling for the same workload.
```

The 21.8 ms wait is not CPU execution. Do not add its 2.7 ms child work again. This explanation is about submission latency, not a claim that GPU or display cost is zero.

### Built-in diagnostic rules

Implement evidence-based checks for native-call storms/copy amplification, ready queue starvation, backpressure, lock contention, dependency cycles, synchronous I/O on critical tracks, UI invalidation amplification, repeated layout, compilation during a critical frame, repeated upload/resource creation, allocation/GC churn, presentation throttling, and missing instrumentation.

For “why did this update?”, link state revision -> reactive trigger -> component/effect -> host mutation -> Blitz dirty node -> layout/paint -> GPU pass. Record both the cause and the scope of work. Similar chains apply to assets, ECS changes, and material variants.

Do not infer hardware causes such as bandwidth saturation, cache misses, or thermal throttling from a long function duration alone. Require the appropriate samples/counters or label the conclusion as a hypothesis.

### UI organization

Use a frame/deadline strip with p50/p95/p99, miss counts, worst cases, and input/queue latency. Under it, show synchronized timelines for JS, native threads, logical jobs, queues, I/O, GPU, audio, and presentation feedback where available.

Selecting an item should expose its critical path, source, fields, causal links, state revision, resource history, related errors, and evidence quality. Show a persistent coverage panel with lost records, unavailable probes, clock uncertainty, mode, build, and debugger state.

Add CPU flamegraphs, off-CPU/ready views where supported, memory lifetime graphs, resource diffs, queue age distributions, and “why updated/why alive/why waiting?” views. Keep related events connected through stable IDs rather than separate unlinked dashboards.

## 13. Replay and controlled experiments

Define replay scope explicitly:

| Scope | Promise |
|---|---|
| Inputs-only | Re-submit recorded application inputs; timing/state may diverge |
| Controlled subsystem | Restore versioned state and controlled nondeterministic inputs; verify output checksums |
| Visual comparison | Compare images or state with declared tolerance; not a bitwise execution guarantee |

For supported systems, record initial checkpoint, inputs, fixed-step sequence, random seeds, clock reads routed through your services, I/O results, asset/build hashes, and relevant job-result ordering. Replay in a sandbox with external side effects disabled or mocked.

A full engine checkpoint needs versioned serializers for each participating system. A JS object snapshot does not restore native pointers, sockets, GPU queues, or driver state. Arbitrary thread races and cross-backend GPU/floating-point behavior are not automatically deterministic.

Check state hashes at controlled boundaries to find the first divergence. Report unsupported sources and the first mismatch. Do not present approximate replay as exact time travel.

Support explicit fault plans for task delays, lost messages, failed allocations at an engine allocation interface, cache eviction, I/O latency/errors, and device-loss recovery tests. Keep them off by default, authorized, deterministic where possible, and marked in captures. A fault run is not a normal performance benchmark.

## 14. Format, exports, and interoperability

The native capture bundle should contain a manifest, producer streams, schema/dictionary checkpoints, clock mappings, capture health, environment/build fingerprint, source/symbol references, and optional content-addressed artifacts. Use chunk framing, checksums, bounded lengths, and indexes so a partially written file can still be recovered where possible.

The manifest records engine, shell, runtime, library, and adapter versions; build optimization/debug configuration; OS/backend/device metadata allowed by privacy policy; enabled features; display target; and capture mode. Retain exact source/build identity instead of depending on whatever source happens to be open in the editor.

Export time tracks, events, counters, and causal flow links to Perfetto. Use its trace processor for offline queries where practical. Perfetto track events already model slices, instants, counters, and flows; keep your richer resource/replay/debug metadata in the native bundle or explicitly mapped extension tables. [S17]

Offer Chrome trace export as a compatibility view, not the canonical lossless format. Export appropriate operation spans, metrics, and structured logs to OpenTelemetry for remote services and fleet analysis. Do not put every entity update into a network tracing SDK on the frame hot path. An export must list omitted signals and semantic losses. [S1]

For CDP/DAP, negotiate supported domains and methods. Do not advertise Chromium-only domains simply because the shell uses V8. [S16]

## 15. Validation and implementation sequence

### Build in dependency order

1. Define IDs, raw clocks, schema registration, bounded writers, capability/health records, and a strict decoder.
2. Add slices, operations, context propagation, epochs, queues, and dependency links. Implement useful Perfetto export immediately.
3. Instrument your shell scheduler, worker layer, native bridge, and wgpu submission path. These integrations give new game systems useful context automatically.
4. Add sampling and source resolution, Blitz/Vue/Three cause chains, and resource accounting.
5. Add authorized snapshot/command/debug services and an evidence-based explanation engine.
6. Add checkpoints, scoped replay, fault plans, regression comparisons, and richer native/vendor probes.

Do not start by writing a custom flamegraph renderer, attaching every expensive probe, or instrumenting every JavaScript function.

### Required tests

Test type contracts and runtime schema validation. Verify exception/cancellation behavior, nested synchronous scopes, interleaved async tasks, fan-out/fan-in/races, lost worker replies, context propagation across every boundary, and native re-entry.

Test ring wrap, producer death during a record, dropped metadata, late GPU results, missing start/end records, reader backpressure, schema upgrades, hot reload, and exact build/symbol matching. Verify that overflow and permission denial are visible, not silent zeros.

Test clock drift, different realm origins, suspend/resume, counter wrap, device reset, unsupported/quantized timestamps, and captures that start/end inside operations. Do not produce negative durations by forced alignment.

Inject known bottlenecks: CPU loop, deliberate sleep, ready queue delay, lock wait, synchronous native op, repeated copies, GPU-heavy pass, intentional readback wait, UI invalidation storm, shader creation, allocation churn, and resource retention. Assert that the diagnostic result identifies the observed category and exposes missing evidence.

Test the privacy/control boundary: unauthenticated clients, cross-origin sessions, oversized payloads, unsafe getters, stale generations, duplicate mutating requests, rejected revisions, cancelled commands, malformed artifacts, and resource-limited queries.

Compare release workloads across modes, with and without debugger attachment. Measure writer/collector memory and CPU cost, tail latency, and loss under sustained load. Separate cold-start, warm-cache, shader-warm, visibility, display/backend, and hardware conditions in regression tests. Report distributions and sample counts, not only average FPS.

## 16. Scope of the accompanying files

`diagnostics.d.ts` defines the proposed producer, provider, and privileged control contracts. `example.ts` shows a physics/worker integration and a capture trigger. `contract.test.ts` verifies selected type restrictions, including rejecting an async callback in a synchronous scope. `tsconfig.json` runs the compile check.

These files intentionally contain no runtime, binary encoder, collector, operating-system probe, wgpu integration, debugger server, or replay implementation. They are a concrete design starting point, not a claim that those components have been implemented or benchmarked.

## Primary references

These references support external API behavior. The architecture, names, limits, and implementation policies above are proposed design decisions.

- **S1 — OpenTelemetry tracing/context model:** https://opentelemetry.io/docs/specs/otel/trace/api/ and https://opentelemetry.io/docs/concepts/context-propagation/
- **S2 — Rust async span lifetime:** https://docs.rs/tracing/latest/tracing/span/struct.EnteredSpan.html
- **S3 — CPU scheduling and native sampling:** https://perfetto.dev/docs/data-sources/cpu-scheduling and https://man7.org/linux/man-pages/man2/perf_event_open.2.html
- **S4 — Multiple clock domains:** https://perfetto.dev/docs/concepts/clock-sync
- **S5 — Web monotonic time and time origins:** https://www.w3.org/TR/hr-time-3/
- **S6 — GPU timestamp queries and queue semantics:** https://docs.rs/wgpu/latest/wgpu/enum.QueryType.html and https://www.w3.org/TR/webgpu/
- **S7 — deno_core adapter entry points:** https://docs.rs/deno_core/latest/deno_core/struct.RuntimeOptions.html and https://docs.rs/deno_core/latest/deno_core/struct.JsRuntimeInspector.html
- **S8 — Blitz ownership boundaries:** https://docs.rs/blitz-dom/latest/blitz_dom/ and https://docs.rs/blitz-paint/latest/src/blitz_paint/lib.rs.html
- **S9 — Vue debug hooks and their development-only restriction:** https://vuejs.org/api/composition-api-lifecycle.html#onrendertracked and https://vuejs.org/api/composition-api-lifecycle.html#onrendertriggered
- **S10 — Three renderer statistics and inspector:** https://threejs.org/docs/pages/Renderer.html and https://threejs.org/docs/pages/Info.html
- **S11 — wgpu errors and device methods:** https://docs.rs/wgpu/latest/wgpu/struct.Device.html and https://docs.rs/wgpu/latest/wgpu/enum.Error.html
- **S12 — Browser timing observations:** https://www.w3.org/TR/event-timing/ and https://www.w3.org/TR/long-animation-frames/ and https://www.w3.org/TR/performance-timeline/
- **S13 — Optional browser sampling/memory proposals:** https://wicg.github.io/js-self-profiling/ and https://wicg.github.io/performance-measure-memory/
- **S14 — Shared memory platform restrictions:** https://html.spec.whatwg.org/multipage/structured-data.html
- **S15 — Native platform tracing:** https://learn.microsoft.com/en-us/windows/win32/etw/about-event-tracing and https://developer.apple.com/documentation/os/recording-performance-data
- **S16 — Debug protocols:** https://chromedevtools.github.io/devtools-protocol/ and https://microsoft.github.io/debug-adapter-protocol/overview.html
- **S17 — Perfetto tracks, flows, and processing:** https://perfetto.dev/docs/instrumentation/track-events and https://perfetto.dev/docs/getting-started/other-formats
