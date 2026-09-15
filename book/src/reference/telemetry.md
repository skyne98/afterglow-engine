# Generic Telemetry

`afterglow-telemetry` contains the Rust and TypeScript core for metrics and traces.
Any application can use the core without the engine, shell, ECS, or renderer.
The engine and shell are separate reporting targets.
The current implementations use the same 40-byte little-endian record ABI.

## Generic producers and Afterglow adapters

The standalone TypeScript module is
`crates/afterglow-telemetry/web/src/telemetry.ts`.
Its `Telemetry` class accepts caller-owned trace storage, metric cells, and a clock.
The engine re-exports that class as `EngineTelemetry` and registers `TelemetryRes`.
There is only one recorder implementation per language.

EngineMemory ownership and RingBuffer transport belong to the Afterglow adapters.
Other applications supply their own storage and transport.

The accepted diagnostics migration adds a separate collector process, rolling
history, and generic inspection. The new format will reject old AGTB/AGTL files.
Recording stays off until the application connects to the profiling CLI WebSocket server.
No tokens or permission tiers are used. Sensitive fields stay excluded by default.
The shared recorder now exports prefix and rolling captures as DGTB batches.
Native and browser WebSocket capture checks passed with no lost records.
CLI file analysis passed its checks. Live inspection and sustained overhead checks remain open.
Six native processes compared the same paint workload with capture on and off.
The median mean was 7.118 ms for control and 7.206 ms for capture (1.24% higher).
Capture runs had larger frame-time peaks, so the overhead check remains open.
The [comparison report](../../../docs/benchmarks/native-paint-shell/profiling-comparison-1788655121419/README.md) includes all runs and an unexplained one-second cadence interruption.
The current runtime behavior follows.

The generic `Publisher` connects to the localhost profiling server from any application.
The CLI owns the server and capture files. No application listener or subprocess is necessary.
The separate `afterglow-diagnostics-worker` adapter puts socket operations on an
existing native RPC OS worker. The shell defaults to port 8086.
`AFTERGLOW_DIAGNOSTICS_PORT` selects another port, or `off` disables profiling.
The shell capture pump supplies host and paint records. The native WebSocket capture
passed with no lost records. The timing samples do not prove zero application delay.
Socket operations do not belong on a presentation or audio thread.

## Bounded field prototype

A separate Rust/TypeScript codec supports numeric, boolean, bounded text, enum, and generational reference fields.
The caller supplies explicit limits and reusable buffers.
The codec uses actual text lengths and one state byte for each excluded field, without padding.
Encoding returns the written byte count, which can be less than the registered maximum.
Secret fields never enter the encoded payload.
Sensitive fields stay excluded unless the bootstrap policy permits them.
This is a data policy, not a connection permission system.

The codec does not yet change active captures or CLI output.
See the [field codec reference](../../../docs/api/telemetry-fields.md) for the layout, failure behavior, and verification status.

## Metrics versus traces

Metrics are cheap and always active:

- counters;
- gauges;
- maxima;
- fixed 32-bucket logarithmic histograms.

Traces need explicit activation and use fixed-capacity storage. They record spans,
async operations, instant events, and cross-thread/worker flows. Keeping the two
storage planes avoids turning every counter increment into a timestamped event,
while shared descriptors and correlation IDs keep the result unified.

## Runtime setup

Every runtime declares telemetry memory explicitly:

```ts
const runtime = EngineRuntime.forScene({
  scene,
  entityCapacity: 1024,
  memory: {
    frameScratchBytes: 64 * 1024,
    renderScratchBytes: 64 * 1024,
    structuralCommands: 512,
    workerCompletions: 64,
    assetRequests: 64,
    vtRequests: 1024,
    telemetryRecords: 16_384,
    telemetryMetricCells: 512,
  },
  diagnosticCapacity: 64,
  maxWorkerInputs: 2,
  maxRenderPasses: 2,
});
```

`EngineRuntime` exposes `runtime.telemetry` and registers the same object as
`TelemetryRes`. It always maintains frame count, frame-delta histogram, and
maximum-frame metrics. When tracing is armed it records the complete frame,
worker polling, VT work, structural and pose drains, render preparation, game
update, and render-pass ranges. Engine-owned BIG sessions additionally feed
source reads, feedback detection, scheduler/bulk waits, native RPC round trips,
texture queue/transcode, mesh optimization, complete page loads, and VT upload/
publication into the same correlation timeline. The old cache descriptors are removed.
The analysis scripts use the current catalog.

```ts
runtime.telemetry.trace.arm(1);
// Run a bounded scenario.
runtime.telemetry.trace.stop();

const snapshot = runtime.telemetry.trace.snapshot();
const output = new Uint8Array(runtime.telemetry.trace.encodedBatchBytes());
runtime.telemetry.trace.encodeBatchInto(output, {
  session, sourceId: 1, generation: 1, clockDomain: 1, clockGeneration: 1,
});
```

The output is a `DGTB` v1 batch with a 96-byte header.
The application supplies a nonzero 128-bit session and producer/clock generations.
The header retains sequence ranges, dropped records, and overwritten records.
Old AGTB input is rejected. Browser records use monotonic nanoseconds and
declare a 1 GHz tick rate. A full capture preserves existing records, drops new
records, and reports the dropped count. Its counter saturates at the integer limit.
It never blocks or grows.

For a rolling window, select retention explicitly:

```ts
runtime.telemetry.trace.arm(epoch, undefined, TelemetryCaptureRetention.Rolling);
// Record through the same trace methods.
runtime.telemetry.trace.stop();
const window = runtime.telemetry.trace.snapshot();
```

Rolling retention replaces the oldest records and reports `window.overwritten`.
Descriptor metadata stays separate and survives buffer wrap.
`stop()` puts the retained records in chronological order without a temporary buffer.
This cold O(capacity) operation does not belong in a sealed frame-time hot path.
A retained window can lack a span start or end. It does not imply a zero-duration span.

The DGTB encoder supports rolling retention without loss of overwrite metadata.
The collector rejects incorrect identities and overlapping sequences.
After export, `resume()` discards the consumed window and continues the same capture.
It retains sequence and loss counters, so successive batches have different ranges.
Copy or consume snapshot data before resume. Use `reset()` only to end the capture.
Sequence exhaustion rejects new records instead of reusing identifiers.

Dungeon reserves 65,536 records (2.5 MiB) for diagnostic captures. The
pre-removal RTX 3090 nine-pose baseline wrote 24,850 records with no drops or
unmatched spans. It identified the former 100 ms quality batching window and
texture-worker queueing—not bulk I/O or atlas upload—as the dominant latency.

## Rust producers

```rust
let mut recorder = Recorder::new(DESCRIPTORS, 16_384, MonotonicClock)?;
recorder.arm(CaptureConfig::all(epoch))?;
let span = recorder.span(PREAD, context, offset, length);
// work
span.finish(bytes_read, status);
recorder.stop()?;
```

Descriptors are static; dynamic records contain only a timestamp, correlation
ID, two numeric arguments, descriptor ID, and phase. The disabled path returns
before reading the clock. Afterglow worker adapters send batches through the
existing RPC rings. The generic core does not select a transport.

Rust uses `CaptureConfig::flight(epoch)` for rolling retention.
Its snapshot separates `overwritten_records` from `dropped_records`.
`BatchHeader::from_snapshot(identity, tick_rate, snapshot)` accepts either retention mode.

Collector registration can borrow received descriptor strings and then owns bounded copies.
Hot producers still use static bootstrap catalogs.

The Rust collector validates source descriptors, clocks, record ordering, and
batch bounds. Its constructor needs explicit capacities for sources, descriptors,
metadata bytes, records, batches, metric samples, and encoded DGTL bytes.
`raw_bytes()` gives the exact output size without a record scan.
Capacity errors keep previous data intact. The binary codec rejects invalid flags, phases, reserved fields,
zero tick rates, and decreasing timestamps before it changes output. It streams Chrome Trace JSON readable by Perfetto and can preserve
the lossless DGTL representation with the original DGTB batch boundaries.

## Start the profiling CLI

Build `afterglow-collector` with the `collector` Cargo feature.
The CLI owns the WebSocket server and capture files. Applications connect to it.

```sh
# Defaults: localhost:8086, captures/, 10 seconds, 64 MiB.
afterglow-collector capture
# Optional listen address, directory, seconds, and maximum file bytes:
afterglow-collector capture 127.0.0.1:8086 captures 30 67108864
```

Applications connect without credentials. One connection sends several registered
producer identities. Binary WebSocket messages use protocol 3, not the Tracy wire format.
Each message contains one opcode and its payload. The CLI reports its listening URL
as JSON. Browser origins must be localhost HTTP(S). Remote connections are excluded.
At application exit, duration, explicit finish, or capacity, the viewer saves
accepted data and exits. The exact DGTL byte limit includes metadata.
Publication does not replace an existing capture. A failed export keeps partial data.
The port does not expose arbitrary code execution. Any local process can read its
profiling data. Remote connections are excluded.
The agent uses the CLI, not a graphical inspector. Browser capture and the file commands passed their checks. The shell connects automatically and registers separate sources.
If the server is absent, recording stays off. Connection attempts occur at most once
per second on the I/O worker.
It sends at most one batch per completed 50 ms timer step, with one RPC in flight.
The host records presentation, input event type, and asynchronous RPC duration.
Additional spans measure runtime turns, frame work, and presentation work.
These spans measure wall time, not CPU time or physical display timing.
Redraw requests and focus/occlusion/suspension events help locate pauses.
Window state 2 means unknown. New captures include the last known states.
Run `bun scripts/analyze-native-paint-capture.ts FILE` for the eight longest
presentation intervals, overlapping work, and missing span boundaries.
Six fresh native processes passed these measurement checks. Most measured pauses
occurred inside presentation work. One-second interruptions also occurred without capture.
The [pause report](../../../docs/benchmarks/native-paint-shell/profiling-pauses-1788707539/README.md)
retains all results and their limits.
Three further captures passed HUD scene, composition, and surface presentation checks.
Their longest intervals occurred almost entirely inside `GPUCanvasContext::present()`, without overlapping application RPC.
The [presentation report](../../../docs/benchmarks/native-paint-shell/profiling-present-stages-1788708051/README.md)
records the measurements. Driver-level causes remain unknown.
The native probe also accepts `--strace` for main-thread waits.
A [completed trace](../../../docs/benchmarks/native-paint-shell/profiling-syscalls-1788709350/README.md)
identified Linux synchronization fence waits, not RPC socket waits.
The controlling GPU operation and the trace-to-capture clock mapping remain unknown.
Tracing changes timing and is not a performance acceptance test.

A later visibility check reproduced a 997.555 ms interruption without a collector or active recording.
Moving only the diagnostic window to an inactive workspace triggered the delay.
A matching trace placed almost one second in surface presentation, without overlapping application RPC.
The stack identifies `wl_display_dispatch_queue` inside NVIDIA presentation on the main thread.
Thus the Wayland event wait also delays JavaScript, input dispatch, and RPC response processing.
See [the control report](../../../docs/benchmarks/native-paint-shell/profiling-pause-visibility-controls-1788731282/README.md)
and [the socket stack](../../../docs/benchmarks/native-paint-shell/profiling-pause-visibility-stacks-1788731483/README.md).
This identifies a repeatable trigger, not the cause of every historical pause.
No runtime presentation policy changed. Foreground performance and capture overhead checks remain open.

The host `value0` and `value1` fields contain worker/method IDs at RPC begin and
response bytes/error status at RPC end.
The paint source records stroke admission and raster publication.
No text, file paths, pointer coordinates, or paint pixels enter these records.
A slow viewer can lose capture records but must not stop painting.

Engine code can call `attachProfiling(name, recorder)` during bootstrap.
The recorder remains caller-owned through `EngineMemory`. Shell registration accepts
at most seven application sources, each with at most 1,024 records and 256 descriptors.
The paint prototype uses a fixed 40 KiB recorder. Cold transfer still allocates.
This is not evidence of sealed allocation behavior or physical input latency.
The shell build now needs Bun for the capture entry.
See the API reference for publisher operations and frame details.

For the browser, call `initializeWebProfiling({workerUrl, transportUrl, serverUrl?})`
before registration. The browser must be cross-origin isolated. A dedicated diagnostics
worker uses the existing RPC rings and owns the outbound WebSocket connection.
The default URL is `ws://127.0.0.1:8086/`. Native targets never start this browser worker.
The browser owner accepts eight caller-owned recorders. Real browser capture passed with no lost records. Sustained timing checks remain open.

## Read captures with the CLI

```sh
afterglow-collector summary capture.dgtl
afterglow-collector records capture.dgtl 1 paint.stroke_sample 100 0
# Use the returned next_offset for the next page.
afterglow-collector records capture.dgtl 1 paint.stroke_sample 100 100
# Read the longest RPC spans and their duration statistics.
afterglow-collector spans capture.dgtl 0 rpc.round_trip 20
# Follow one operation across source records.
afterglow-collector correlate capture.dgtl 4294967297
# Read metric snapshots, including histogram buckets.
afterglow-collector metrics capture.dgtl
```

`summary` gives identities, clocks, loss counters, descriptor phase counts, and metric catalogs.
`records` filters by source ID and exact event name. Use `-` to omit either filter.
The default page size is 1,000 records, with a maximum of 10,000.
The output states the matched count and next offset. It uses source/sequence order.
All u64 record fields use decimal strings to prevent JavaScript precision loss.
`records` and `correlate` include named, typed `arguments` with units and exact raw bits.
`spans` includes both `begin_arguments` and `end_arguments`.
Signed values use decimal strings, finite floats use JSON numbers, and valid booleans use `true` or `false`.
Invalid booleans and non-finite floats have a null value and an explicit error. Raw bits remain available.
This uses the existing numeric arguments. The bounded field codec remains separate.
The optional final file-byte limit defaults to 64 MiB. The reader validates the
complete DGTL file before it writes analysis output. Errors use JSON on stderr.
`afterglow-collector chrome capture.dgtl` writes the existing Chrome Trace JSON to stdout.
`spans` returns the longest observed intervals plus count, total, mean, and p50/p95/p99 durations for all matching pairs.
Its optional source, name, limit, and offset arguments match `records`.
It reports missing boundaries and excludes ambiguous async pairs and batches with new drops.
Durations are wall time, not CPU time. Nested spans overlap, so their sum is not application elapsed time.
`correlate` selects an exact nonzero operation ID, followed by optional source, limit, and offset arguments.
It retains source identities and clock uncertainty without an inferred causal order.
`metrics` accepts the same arguments as `records` and returns exact snapshots without inferred counter deltas.
All three commands include loss counters and pagination. No additional permission setup is necessary.
Live target inspection remains open.

## Current scope

The core crate and broad TypeScript frame/asset/VT integration are implemented.
Generated server-side RPC flow points, worker-local `pread`/arena/codec detail,
worker draining, audio scopes, shell file capture, and GPU timestamp ingestion
are subsequent integration gates. New
subsystem profilers should feed this API rather than create another event format
or Chrome exporter.

See [`docs/api/telemetry.md`](../../../docs/api/telemetry.md) for the complete API
and [`docs/implementation/generic-diagnostics-plan.md`](../../../docs/implementation/generic-diagnostics-plan.md)
for the current migration status.
