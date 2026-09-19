# Generic diagnostics migration

## Status

Implementation started. The generic TypeScript producer now belongs to
`crates/afterglow-telemetry/web/src/telemetry.ts`.
The engine imports that implementation and adds its ECS resource registration.
The Rust crate already has no engine or shell dependency.

The shared recorder supports prefix and rolling retention with separate loss counts.
DGTB batches now contain session, producer and clock generations, and sequence ranges.
DGTL export retains these batches and their metadata. Old AGTB input is rejected.
This is not the full diagnostics runtime. DIAG-011 replaces the application listener
with an outbound WebSocket connection to the profiling CLI server.
Native and browser WebSocket paint captures passed with no lost records.
The native capture contains separate host and engine sources.
Registered field schemas and live inspection remain incomplete.
The browser adapter and CLI file-analysis commands passed their functional checks.
Six fresh native processes compared the same workload with capture on and off.
The median mean was 7.118 ms for control and 7.206 ms for capture (1.24% higher).
Capture runs also had larger frame-time peaks. One run had an unexplained one-second cadence interruption.
The overhead and sustained memory checks remain open.
See [the comparison report](../benchmarks/native-paint-shell/profiling-comparison-1788655121419/README.md).
Further host spans located long surface presentation calls.
A [descriptor-decoded syscall trace](../benchmarks/native-paint-shell/profiling-syscalls-1788709350/README.md)
identified synchronization fence polls, not RPC socket waits.
The user made the capture-pause investigation the current priority after the CLI checks passed.
Field-codec work stays pending.
A [native stack trace](../benchmarks/native-paint-shell/profiling-pause-stacks-1788730509/README.md)
places a 24.981 ms fence poll inside NVIDIA driver presentation, below `NativeSwapchain::present`.
The Vulkan trace capture (`profiling-pause-vulkan-1788730686`, 264 MB)
confirms long `vkQueuePresentKHR` calls, but its GPU timing relation is not verified.
The capture is not committed; it exceeds GitHub's 100 MB file limit.
The [visibility control](../benchmarks/native-paint-shell/profiling-pause-visibility-controls-1788731282/README.md)
reproduced a 997.555 ms interruption with no collector connected and zero transmitted batches.
The [socket stack](../benchmarks/native-paint-shell/profiling-pause-visibility-stacks-1788731483/README.md)
identifies `wl_display_dispatch_queue` inside NVIDIA presentation on the main thread.
An inactive-workspace move is a verified trigger for the one-second symptom, independent of capture.
This completes the cause-identification check, not a runtime repair or performance acceptance check.
The exact internal driver condition and the cause of every historical interruption remain unverified.
The existing DGTL format now supplies `spans`, `correlate`, and `metrics` commands.
They expose observed duration statistics, operation records, exact metric snapshots, loss counters, and incomplete intervals.
Six CLI regression tests, the telemetry test suite, strict scoped Clippy, and the release build passed.
The retained `profiling-syscalls-1788709350` capture returned 140 RPC spans and the two exact records for a selected correlation.
That capture has no metric samples. Regression tests cover exact metric values and histogram queries.
These are file-analysis checks, not new application overhead measurements.
The CLI now decodes the existing numeric argument metadata, including signed values, float bits, booleans, names, and units.
Records and correlations include typed arguments. Span output includes both boundary values.
The recorder, transport, and capture formats do not change. Bounded text and reference-field integration remains open.
The recorder-to-CLI regression check, telemetry test suite, strict scoped Clippy, release build, and book build passed.

## Accepted decisions

The user supplied the diagnostics proposal and accepted these decisions:

| ID | Decision |
|---|---|
| DIAG-001 | Change `afterglow-telemetry` into application-independent diagnostics. The engine and shell are separate reporting targets. |
| DIAG-002 | The profiling CLI is a separate process. DIAG-011 defines connection ownership. |
| DIAG-003 | The new runtime rejects old AGTB/AGTL formats. Do not add a compatibility reader or converter. Keep historical evidence files unchanged. |
| DIAG-004 | Recording stays off until a viewer connects. Sensitive fields stay excluded by default. |
| DIAG-005 | Start with shell, RPC, and input records plus export. Add debugger control and replay afterward. |
| DIAG-006 | Validate the Linux native shell and public-web paint demo. Keep the core platform-independent. |
| DIAG-007 | Superseded by DIAG-011. After target exit, save retained data and exit. |
| DIAG-008 | The viewer owns its output directory and finite byte limit. At capacity, stop capture and keep files. Do not stop the application. |
| DIAG-009 | Removed. No permission tiers, role registry, token exchange, or private process bootstrap. |
| DIAG-010 | Superseded by DIAG-011 for connection direction. No authentication or process-owner framework. |
| DIAG-011 | The profiling tool owns a WebSocket server. Native and browser applications connect to it. No application listener or browser bridge. |
| DIAG-012 | Supply a CLI for agent capture, inspection, and analysis. Do not build a graphical inspector. |

The crate name and repository location stay unchanged. Independence concerns
its dependencies, protocol, and behavior, not a new repository.

## Core and adapter boundary

The dependency direction is:

```text
Application instrumentation ─┐
Engine adapter ───────────────┼──> generic producer API and binary records
Shell adapter ────────────────┤                  │
Runtime / OS / device probes ─┘                  ▼
                                   independent collector
                                              │
                                   inspector / exports
```

The core owns record semantics, identity, raw clocks, registered schemas,
capacity checks, capability records, and health records.
It must not import ECS, EngineMemory, RingBuffer, Three.js, Blitz, wgpu, or Deno.
It must not start a window, device, worker, or transport during producer creation.
An application supplies its identity, clock, fixed storage, and transport adapter.
A system name is a namespace, not an engine type.

Afterglow adapters supply EngineMemory storage and existing RingBuffer payload
transport. Those are Afterglow rules, not requirements for another application.
The engine and shell can use one session without sharing producer ownership.
Each producer has one owner and its own sequence and clock identity.
The collector cannot inspect mutable target state directly.

The application connects to the profiling CLI WebSocket server through an
explicit adapter. It does not replace the existing Afterglow worker transport.
Public-web collection uses a worker and reports browser lifecycle limits.
A generic browser adapter can use bounded transferred buffers where SAB is
unavailable. The Afterglow web adapter must retain its RingBuffer boundary.

## Retained proposal

- [Original design](diagnostics-proposal/DESIGN.md).
- [Revised proposed TypeScript contract](../../crates/afterglow-telemetry/contracts/diagnostics.d.ts).

The imported design is source material, not a statement of implemented behavior.
Its engine-specific examples do not limit the generic system.
This plan overrides its deployment and compatibility alternatives.

Original SHA-256 values:

```text
DESIGN.md
 a66a5a8db1ae4c2d1a27b90e117dd9808cae8f6e147e5f5ca94c8249dd3ec10b
diagnostics.d.ts
 41251f056b7f94159513ecd0296d5da60bca91e647929da51a88e8287b4b4dc2
```

The revised contract rejects mixed synchronous/Promise callback return types.
It also adds explicit snapshot truncation, command target generations, generic
application capability scope, and missing bootstrap capacity fields.
These declarations are design contracts only. They do not export executable
`createDiagnostics` or inspector functions.
Type checks cannot replace runtime validation of callbacks or external input.

## Implementation sequence

### 1. Separate the existing core

- Move the TypeScript implementation into the telemetry crate.
- Keep one implementation. The engine adds only its resource registration.
- Keep allocation lint coverage after the move.
- Run standalone producer tests and engine adapter tests.
- Make sure the core compiles without engine or shell imports.

Phase 1 checks passed:

- Nine TypeScript producer, catalog, and adapter tests.
- Standalone TypeScript compilation and proposed API type checks.
- Thirty-one repository contract tests.
- Ten Rust telemetry tests, including the no-allocation test.
- Allocation lint for 64 hot regions and 65 effect-declared functions.
- Import boundaries, xtask compilation, web build and artifact drift, mdBook,
  and whitespace checks.

The mdBook build reported a preprocessor version warning but completed.
These checks do not validate the new diagnostics format or collector.

### 2. Replace the record and capture model

- Specify session/producer identity, generations, sequence ranges, and raw clocks.
- Add schema registration, bounded field encoding, and a strict decoder.
- Replace finite-prefix capture with explicit off/flight/trace/deep behavior.
- Keep metadata in bounded storage that survives data-ring wrap.
- Expose loss, missing metadata, overwritten history, and truncated intervals.
- Add slices, operations, events, metrics, resources, epochs, queues, and links.
- Supply scalar/binary hot writers as well as the object API.
- Replace Rust and TypeScript producers together with cross-language byte tests.
- Replace active AGTB/AGTL consumers. Reject old format versions explicitly.

Phase 2 is incomplete. The recorder foundation now has:

- Prefix and rolling retention in Rust and TypeScript, initially inactive.
- Constant-time fixed-storage writes and separate overwrite/drop counters.
- Static descriptor storage that survives data-buffer wrap.
- Cold, allocation-free chronological rotation at freeze.
- DGTB export for prefix and rolling snapshots, with full identity and loss metadata.
- DGTL export with original batch boundaries and sequence gaps.
- Shared Rust/TypeScript full-batch bytes and strict old-format rejection.
- Binary validation before any caller-output mutation.

The recorder foundation passed 14 Rust tests, including 100,003 rolling writes
and freeze without allocation. The TypeScript producer and adapter tests also passed,
including overlapping-output rejection and shared Rust/TypeScript record bytes.
Clippy, allocation lint, import boundaries, repository contracts, web build/drift,
and mdBook passed. A release microbenchmark exercised 983,616 replacements.
This is a recorder check, not a sustained application overhead or latency result.

The batch identity and transport-format migration is present. The VT profile and
replay parsers use the shared DGTB decoder. Their obsolete browser capture entry
point still needs migration before a live CLI check.
Registered schemas, full capture modes, semantic records, and metadata checkpoints
remain open.
A [bounded field codec prototype](../api/telemetry-fields.md) now has matching Rust and TypeScript layouts.
It passed the shared-byte, malformed-input, field-exclusion, capacity, and Rust no-allocation checks.
The compact codec passed the complete telemetry checks: 41 Rust tests and 35 TypeScript tests.
The prototype does not change active records or capture files.
The [native codec benchmark](../benchmarks/telemetry-field-codec/README.md) found that empty or excluded 1 KiB text fields used 1,029 encoded bytes.
The replacement removes padding and returns the actual encoded length, within the registered maximum.
Compact-layout regression checks and a matching benchmark passed.
Empty and redacted 1 KiB text now use five bytes and one byte, respectively.
This codec change does not change the active capture ABI or establish an application timing limit.
Recorder retention is a mechanism, not the complete `off/flight/trace/deep` API.

### 3. Add the collector process and initial adapters

The in-process collector has explicit source, descriptor, metadata, record,
batch, metric-sample, and encoded DGTL byte limits. Exact byte accounting rejects
excess work before it changes retained data.
The optional `collector` feature now supplies a WebSocket client and a CLI server.
`afterglow-collector capture` listens on localhost:8086 by default and saves bounded
DGTL captures without replacement. Protocol 3 uses one opcode and its payload in
each binary WebSocket message. No tokens or child process bootstrap are used.
WebSocket checks passed connection, reconnect, capacity, malformed data, application
exit, duration, origin rejection, and slow-server tests. The bounded DGTL reader
and CLI filtering, pagination, JSON output, and Chrome export tests also passed.
The complete telemetry suite passed 35 tests.
Dependency-wide strict lint still reports existing RPC warnings.
The native `afterglow-diagnostics-worker` adapter uses generated RPC startup and one
I/O worker. The shell connects to port 8086 by default. `AFTERGLOW_DIAGNOSTICS_PORT`
selects the server port, or `off` disables profiling. No application listener exists.
An unavailable server leaves capture inactive and causes at most one connection
attempt per second. Bounded socket I/O stays on the worker.
The shell polls through the generated native diagnostics client. A cold pump
admits at most eight sources, one registration or batch per step, and one RPC in
flight. The host records presentation, input type, and asynchronous RPC spans.
The paint adapter records stroke admission and raster publication in fixed storage.
Epoch checks reject old batches and old RPC completions after reconnection.
Empty sources skip transfer because repeated empty batches reuse sequence numbers.
The public-web adapter uses the same generated diagnostics client and a dedicated
TypeScript worker over existing RPC rings. That worker owns the browser WebSocket.
It uses a 65,536-byte socket admission limit and at most one connection attempt per second.
The paint bootstrap supplies the worker and transport asset URLs.
Browser build, TypeScript, allocation-lint, artifact-drift, and RPC checks passed.
Real native and browser WebSocket captures passed with no lost records.
The native capture retained 140 RPC pairs, 40 stroke samples, and four raster publications.
The browser capture retained 40 stroke samples and 13 worker messages.
The six-process comparison retained valid captures but showed larger frame-time peaks with capture active.
Overhead acceptance and sustained memory checks remain open.

- Add bounded ingestion, direct local connection, finite capture, and export.
- Keep the collector independent of the application event loop.
- Preserve raw clocks and uncertainty in native capture data.
- Export Perfetto tracks and flows without invented CPU or display timing.
- Add shell scheduler, native bridge, RPC, and input records.
- Prove that the engine, shell, and a non-engine application use the same core.
- Report collector failure and backpressure without blocking application work.

### 4. Add deeper diagnostics

- Add source identity, sampling, resource history, and UI cause records.
- Add bounded owner-thread inspection and explicit commands without permission tiers.
- Validate generations and revisions before enqueue and at each safe point.
- Add evidence-based explanations with explicit missing signals.
- Add debugger adapters and scoped replay only after their acceptance checks.

## Measurement and remaining decisions

Wire layout, chunk size, batching cadence, and initial capacity profiles need
measured prototypes. They are technical checks, not implicit product policy.
Each capacity must be explicit and finite. A requested history duration is not
a guarantee when the record rate exceeds storage capacity.

DIAG-011 replaces the earlier application listener with a CLI WebSocket server.
DIAG-012 selects CLI inspection and analysis, not a graphical inspector.
Remote connections stay excluded.
The user requested continued implementation through paint-demo validation,
not completion reports for isolated recorder changes.
No recorder failure may silently change application error or cancellation behavior.

Acceptance checks include ring wrap, metadata loss, producer death, late records,
clock drift, generation reuse, malformed input, privacy filtering, and backpressure.
They also include no-allocation hot paths and sustained off/flight/trace comparisons.
Short traces do not prove physical latency, presentation stability, or low observer cost.

Native paint input work remains paused. Its device and physical latency checks
are separate from this diagnostics migration.
