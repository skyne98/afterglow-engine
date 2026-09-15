# Generic telemetry API

`afterglow-telemetry` is an application-independent core for Rust and TypeScript
producers. The engine and shell are reporting targets, not core dependencies.
The core does not need ECS, EngineMemory, a renderer, or a native host.

The current runtime combines two correlated planes:

- fixed always-on metrics: counters, gauges, maxima, and 32-bucket log2
  histograms;
- explicitly armed bounded traces: synchronous spans, asynchronous spans,
  instant events, and cross-track flows.

Producer creation does not spawn threads, open sockets, format hot-path strings,
or create a worker transport. Afterglow worker adapters must use the existing
`afterglow-rpc::RingBuffer` mechanism. Other applications supply their own
transport adapters.

## Diagnostics migration

The accepted [migration plan](../implementation/generic-diagnostics-plan.md)
extends this core into the supplied diagnostics design.
The selected deployment uses a separate native collector process.
The new runtime will reject old AGTB/AGTL formats without a compatibility reader.
Afterglow recording stays off until the application connects to the CLI WebSocket server.
No tokens or permission tiers are used. Sensitive fields stay excluded by default.

The generic producer, rolling retention, and DGTB/DGTL transport formats are implemented.
Current metrics remain active. Captures retain explicit session, producer, clock,
sequence, and loss metadata. The decoder rejects old AGTB input.
The optional CLI collector and native/browser WebSocket capture checks passed.
Six native processes compared the same paint workload with capture on and off.
The median mean was 7.118 ms for control and 7.206 ms for capture (1.24% higher).
Capture runs had larger frame-time peaks, so the overhead check remains open.
The [comparison report](../benchmarks/native-paint-shell/profiling-comparison-1788655121419/README.md) retains all results, including an unexplained one-second cadence interruption.
Registered field schemas and live inspection are not implemented. The file
`crates/afterglow-telemetry/contracts/diagnostics.d.ts` is a proposed contract,
not an executable module.

## Dynamic trace record

Every event is one little-endian 40-byte `TraceRecord`:

```rust
#[repr(C)]
pub struct TraceRecord {
    pub timestamp: u64,
    pub correlation: u64,
    pub argument0: u64,
    pub argument1: u64,
    pub descriptor: u32,
    pub phase: u8,
    pub flags: u8,
    pub reserved: u16,
}
```

Producer identity, clock domain, capture epoch, record count, and dropped count
live in the batch header. Names, categories, argument schemas, units, and
severity live in a producer-local static descriptor table. A hot call therefore
performs no string operation or descriptor hash lookup.

## Rust recording

```rust
use afterglow_telemetry::{
    CaptureConfig, DescriptorId, MonotonicClock, Recorder, TraceContext,
};

let mut recorder = Recorder::new(DESCRIPTORS, 16_384, MonotonicClock)?;
recorder.arm(CaptureConfig::all(1))?;

let context = TraceContext::from_parts(asset_id, request_id);
let span = recorder.span(DescriptorId(0), context, offset, length);
// Work; nested events may use the same recorder.
drop(span);

recorder.async_begin(DescriptorId(1), context, input_bytes, format);
recorder.async_end(DescriptorId(1), context, output_bytes, status);
recorder.stop()?;
let snapshot = recorder.snapshot()?;
```

`Recorder` is producer-local and deliberately `!Sync`. Capture control is run on
the owning thread/worker, so writes need no global mutex or atomic cursor. Its
lifecycle is:

```text
Idle → Armed → Frozen → Idle
```

`arm` clears counts and computes the enabled-descriptor bitset before capture.
`stop` freezes the buffer. `snapshot` is valid only while frozen, and `reset`
returns it to idle. Invalid transitions return `CaptureError`.

`CaptureConfig::all(epoch)` selects `CaptureRetention::Prefix`.
A full prefix buffer rejects new records, increments the saturating dropped counter,
and returns `RecordStatus::BufferFull`.

`CaptureConfig::flight(epoch)` selects `CaptureRetention::Rolling`.
A full rolling buffer replaces its oldest record and increments `overwritten_records`.
Each accepted write takes constant time and uses fixed storage.
Disabled, filtered, and invalid-descriptor calls do not replace records.
Static descriptors stay separate from the data buffer and survive every wrap.

At `stop`, a rolling recorder rotates its retained records into chronological order.
This cold operation takes O(capacity) time and allocates no temporary buffer.
Do not include it in a sealed frame-time hot path.
The snapshot gives `retention`, `overwritten_records`, and `dropped_records` separately.
It does not reconstruct a missing span start or end.
Reset clears the window and counters but retains the allocated storage.

After the caller consumes a frozen window, `resume()` continues the same capture.
It clears that window without allocation and keeps the epoch, filter, retention,
sequence, and cumulative loss counters. Consume or copy the snapshot before resume.
The snapshot gives `[first_sequence, next_sequence)` in Rust and
`[firstSequence, nextSequence)` in TypeScript. These ranges do not overlap across
continued windows. An empty window has equal sequence bounds.
Sequence exhaustion rejects new records before clock access and returns
`SequenceExhausted`. Rust stops at `u64::MAX`. TypeScript stops at
`Number.MAX_SAFE_INTEGER`. Neither counter wraps.

`CategoryMask` supports 256 source-defined categories. Filtering and capture
state are checked before reading the clock.

## Metrics

```rust
let metrics = MetricBank::new(METRIC_DESCRIPTORS);
metrics.counter_add(BYTES_READ, bytes);
metrics.gauge_set(PENDING, pending as i64);
metrics.maximum(MAX_LATENCY_NS, elapsed_ns);
metrics.histogram_log2(READ_LATENCY_NS, elapsed_ns);

let mut samples = vec![MetricSample::default(); metrics.required_sample_capacity()];
let count = metrics.snapshot_into(&mut samples)?;
```

Scalar metrics use one `AtomicU64`; a histogram uses exactly 32 cells. Updates
use relaxed atomics, allocate nothing, and have typed wrong-kind/invalid-ID
statuses. Snapshot storage belongs to the caller.

## Batches and collection

`encode_batch_into` and `decode_batch_into` read/write `DGTB` v1
batches without allocation. The fixed header is 96 bytes followed by contiguous
40-byte records. Encoding and decoding reject zero tick rates, unknown flags,
nonzero reserved fields, invalid phases, and decreasing timestamps.
Decoding also validates magic, version, lengths, record count, and output capacity.
Both operations validate the full input before changing caller-owned output.
The cold collector applies the same record checks.
The TypeScript encoder also rejects a shared buffer when output overlaps retained records.
Its tick rate must be a positive safe integer.

`BatchHeader::from_snapshot(identity, ticks_per_second, snapshot)` returns a result
for both prefix and rolling retention. `ProducerIdentity` supplies a nonzero 128-bit
session, producer ID and generation, and clock ID and generation.
Both generations must be nonzero. The collector rejects session or generation mismatches
and overlapping sequence ranges before it accepts records.

The header contains the following fields:

| Offset | Field |
|---:|---|
| 0 | DGTB magic, u16 version, u16 header length |
| 8 | u32 producer ID, capture epoch, clock ID, retention flags |
| 24 | u32 record count, reserved zero |
| 32 | u64 ticks per second |
| 40 | Four u32 session words |
| 56 | u32 producer generation, clock generation |
| 64 | u64 first and next sequence |
| 80 | u64 dropped and overwritten counts |

Flag bit 0 selects rolling retention. Other bits are invalid.
Sequence ranges are half-open. Loss counters are cumulative within a capture.
The collector retains original batch boundaries and reports sequence gaps.
The cold TypeScript decoder is `decodeTelemetryBatch(bytes, maxRecords)` in
`crates/afterglow-telemetry/web/src/batch.ts`. It preserves u64 fields as `bigint`
and checks the record capacity before allocation.

`Descriptor<'a>` and `MetricDescriptor<'a>` can borrow received metadata.
Source registration copies that metadata into bounded collector-owned storage.
The hot `Recorder` and `MetricBank` still use static bootstrap catalogs.

`Collector` is cold-path code. It registers source-local descriptor tables,
validates batches, applies explicit `ClockMapping`s, ingests metric snapshots,
and owns the diagnostic copies after producers freeze.
`Collector::new(session, epoch, limits)` needs explicit positive capacities for
sources, descriptors, UTF-8 metadata bytes, records, batches, metric samples,
and encoded DGTL bytes (`raw_bytes`, minimum 32).
A capacity error preserves previous capture data. Metric ingestion also checks
session and producer/clock generations. Metric timestamps cannot decrease.
`raw_bytes()` gives the exact encoded size without serialization or a record scan.
Each admission checks this byte limit before it changes retained data.
Other exports need their own file-byte limits.
It exports:

- lossless versioned `DGTL` data with session, producer generations, source, descriptor, clock, trace,
  metric, and drop metadata;
- streaming Catapult/Chrome Trace JSON through `std::io::Write`, directly
  readable by Perfetto.

The Chrome exporter performs a bounded-source k-way timestamp merge rather than
constructing one JSON object containing every event. Metric snapshots become
Chrome counter events.

## Profiling CLI and WebSocket connection

The profiling CLI owns a localhost WebSocket server. Applications connect to it.
There are no tokens, permission tiers, or child-process bootstrap messages.
The agent uses the CLI, not a graphical inspector. This protocol is not Tracy-compatible.
The WebSocket transport passed unit tests and real native/browser paint capture checks.

```sh
cargo build -p afterglow-telemetry --features collector
# Start the server before capture. The four capture arguments are optional.
afterglow-collector capture 127.0.0.1:8086 captures 10 67108864
```

The CLI defaults to the values above. It listens only on loopback addresses.
Port 0 selects an available server port. The initial JSON report gives its URL. It keeps one bounded capture and saves accepted data after application
exit, explicit finish, duration, or capacity. Invalid data stops the viewer but
still saves previously accepted records. It never terminates the application.
The duration is an absolute read deadline, including partial frames.
The exact byte limit includes DGTL metadata. Existing files are never replaced.
A synced `.partial` file becomes `.dgtl` through a no-replacement hard link.
The final JSON report gives the path, bytes, records, and stop reason.

With the optional `collector` feature, any Rust application can use
`publisher::Publisher::new(address, session, max_frame, write_timeout)`.
Creation does not connect or start a thread. Only loopback addresses are accepted.
Message capacity is 256–65,536 bytes. `poll()` connects to the CLI and sends its header.
Each connection gets a new epoch. An unavailable server causes at most one attempt
per second and leaves recording inactive.
`publish(opcode, bytes)` sends one bounded frame, or returns false without a viewer.
A write failure closes that viewer connection. `finish()` closes the capture.
Socket writes belong on an I/O worker, not the presentation or audio thread.
The WebSocket write buffer has a 66,560-byte limit. No application message queue grows.

Each binary WebSocket message contains one opcode and its payload, without a TCP
length prefix. Connection protocol version 3 has these messages:

| Opcode | Application-to-viewer payload |
|---|---|
| 0 | `CaptureHello` JSON: protocol, session, epoch, max_frame_bytes |
| 1 | Strict `WireSource` JSON metadata |
| 2 | One binary DGTB batch |
| 3 | Strict `WireMetrics` JSON snapshot |
| 4 | Empty finish payload |

The server sends no application requests, tokens, or acknowledgments. WebSocket
control frames are permitted. Unexpected application messages close the connection.
No arbitrary code execution is exposed. Other local processes can receive profiling
data. Remote access is excluded. Browser connections must use a localhost HTTP(S)
origin. Unknown web origins and non-root WebSocket paths are rejected.
JSON metadata must retain exact u64 values, not integers rounded by JavaScript.

### Afterglow adapter

`afterglow-diagnostics-worker` uses a generated `DiagnosticsClient`, existing native
RPC rings, and one OS worker. The core does not import this adapter or RPC.
`DiagnosticsWorker::connect(port)` prepares the client. The shell defaults to server
port 8086. `AFTERGLOW_DIAGNOSTICS_PORT` selects another server port, or `off` disables
profiling. Port 0 is invalid for the application. Startup logs the server URL.
No JSON configuration file or executable path is necessary.

- `start()` polls the endpoint and returns connected state, identity, raw native
  tick, 64 KiB frame capacity, and 1,024-record batch capacity.
- `ingest(epoch, opcode, payload)` returns 0 after socket admission or 2 without
  a viewer or with a stale epoch.
  Admission does not prove that the viewer saved the data.
- `finish()` closes the viewer connection and returns disconnected state.

The write timeout is 100 ms on the dedicated diagnostics worker. A slow viewer
cannot stop paint or the presentation thread. This is an allocation-bearing
slow path, not a sealed-worker allocation claim.
The shell installs a capture pump before application startup. The native WebSocket
paint capture passed on RTX 3090 with separate host/engine sources and no lost records.
The browser capture also passed, with 40 stroke samples, 13 worker messages, and no losses.
The CLI file commands passed malformed-input, exact-integer, filtering, and pagination checks.
Frame-time samples do not isolate native capture cost or prove zero application delay.
See `docs/benchmarks/native-paint-shell/`. No graphical inspector is planned.

### Capture file analysis

These CLI commands read DGTL files without a live application:

```sh
afterglow-collector summary capture.dgtl
afterglow-collector summary capture.dgtl 1
afterglow-collector records capture.dgtl 1 paint.stroke_sample 100 0
afterglow-collector spans capture.dgtl 0 rpc.round_trip 20 0
afterglow-collector correlate capture.dgtl 4294967297 - 100 0
afterglow-collector metrics capture.dgtl - - 100 0
afterglow-collector chrome capture.dgtl > trace.json
```

`summary` gives source identities, clocks, losses, descriptor phase counts, and metric catalogs.
`records` accepts an optional source ID, exact event name, limit, and offset.
Use `-` to omit a source or name filter. The default limit is 1,000 records, with a maximum of 10,000.
Results include `matched`, `returned`, and `next_offset`. Order is source then sequence, not cross-clock time.
All u64 record fields and loss counters use decimal strings. Argument type and unit codes come from the descriptor catalog.
`records` and `correlate` also return an `arguments` array with each declared argument's slot, name, type, unit, raw bits, and decoded value.
`spans` returns the same data in `begin_arguments` and `end_arguments`.
Duplicate names stay separate by slot. Arguments with type `None` are absent from these arrays.
Signed integers use two's-complement interpretation and decimal strings. Unsigned values, identifiers, byte counts, and durations also use decimal strings.
Finite `FloatBits` values become JSON numbers. Boolean values must be zero or one and become JSON booleans.
A non-finite float or invalid boolean has `value: null` and an explicit `error`, while `raw` keeps the exact bits.
Valid arguments have `error: null`. Unit names do not change the value or its scale.
These decoded views use the existing two numeric arguments, not the separate bounded field codec.
The recorder and capture formats do not change.
`chrome` writes the existing Chrome Trace export to stdout. No GUI is necessary.
Each command accepts a final file-byte limit, with a default of 64 MiB.
Unknown source IDs, malformed files, and excess capacity produce a JSON error on stderr and exit code 1.

`Collector::read_raw(bytes, limits)` validates the complete file before analysis.
It uses the existing registration, batch, clock, and metric checks.
It rejects invalid enums, padding, UTF-8, counts, versions, identities, and trailing bytes.
`write_summary_json`, `write_records_json`, `write_spans_json`, `write_correlation_json`, and `write_metrics_json` supply the CLI output.
These are cold operations with allocation. Live target inspection remains open.

`spans FILE [source|-] [name|-] [limit] [offset] [max-bytes]` returns the longest observed spans first.
Equal durations use source ID and begin sequence as tie breakers.
The query pairs records by source, descriptor ID, and correlation ID.
Synchronous spans use nested LIFO matching, including context zero.
Overlapping async spans with the same ID are ambiguous and do not supply durations.
No span joins different sources. A sequence gap closes pending starts without an inferred end.
A batch with an increased drop count does not supply span timings.
Each descriptor reports missing starts, missing ends, ambiguous pairs, and excluded records.

Span statistics include all matching pairs, not only the returned page.
They contain count, total, integer mean, minimum, maximum, and nearest-rank p50/p95/p99 durations.
Empty statistics use null for mean, minimum, maximum, and percentiles.
Durations use mapped wall time, not CPU time or physical display timing.
Nested and concurrent spans overlap. Their sum is not application elapsed time.
The command retains at most one pair per two input records and sorts the pairs in this cold analysis.

`correlate FILE ID [source|-] [limit] [offset] [max-bytes]` returns records with the exact nonzero `u64` operation ID.
It includes instant events, span boundaries, and flow records without an inferred causal order.
An application must use distinct operation IDs if unrelated work must remain separate.
Source and sequence order remain stable. Cross-source timestamps retain clock uncertainty.

`metrics FILE [source|-] [name|-] [limit] [offset] [max-bytes]` returns exact metric snapshots in source/sample order.
The output includes metric kind, unit, histogram bucket, timestamp, and decimal-string value.
It does not infer counter deltas, histogram percentiles, or samples between snapshots.
All three commands use the same pagination and byte limits as `records`.
Their output includes session identity, producer/clock generations, clock uncertainty, and source loss counters.
`records` now includes this context too. No capture format, dependency, or application permission changed.

### Application capture pump

`web/src/capture.ts` has no engine dependency. `CapturePump` accepts the structural
`CaptureClient` interface and at most eight `CaptureSource` objects. Each `step()`
performs one connection check and at most one registration or batch transfer.
Concurrent calls do not create concurrent transfers. Empty sources skip transfer
but retain their turn in the source cycle. This prevents duplicate sequence batches.
No record queue grows behind
a slow viewer. A disconnect resets the recorders. A new connection repeats source
registration with a new epoch. Native ingestion rejects previous-epoch batches.

`RecorderSource` accepts a caller-owned 1 GHz `TelemetryRecorder`, at most 1,024
records, 256 descriptors, and a 128-character source name. It owns one fixed batch
buffer. Each drain retains the sequence and cumulative loss counters. Clock metadata
includes the native reference tick and the full RPC round-trip uncertainty.

The shell calls the pump from one cold timer, 50 ms after each completed step.
Source 0 is `afterglow-shell`: presentation, input event type, and asynchronous RPC
begin/end records. The diagnostics worker does not record its own RPC calls.
Host RPC completions carry an epoch ticket, so reconnection cannot admit old spans.
Host argument labels are `value0` and `value1`, not duplicate JSON keys.
Presentation uses `(presented, 0)`. Input uses `(event_type, 0)`.
RPC begin uses `(worker_id, method_id)`. RPC end uses `(response_bytes, failed)`.
Host pause measurements also include `host.runtime_turn`, `host.frame`, and
`host.present_work` span pairs. Their duration is wall time, not CPU or GPU time.
`host.redraw_request` marks scheduler admission. Focus, occlusion, and suspension
records contain `(state, 0)`: 0 is false, 1 is true, and 2 is unknown.
Each new capture includes the last known window states. These are host events,
not proof of browser visibility or compositor activity. Epoch checks reject old
span completions. Correlation exhaustion rejects new starts without ID reuse.
`scripts/analyze-native-paint-capture.ts FILE` uses the public CLI to compare
presentation intervals with these spans and window events. It reports the eight
longest intervals, total interval count, and missing span boundaries. Its input
limit is 100,000 host records. Nested span totals overlap and must not be added.
Six fresh processes passed the host-measurement checks. Their longest captured
intervals mainly contain presentation work. One-second interruptions also occurred
without a collector, so capture is not necessary for that symptom.
See `docs/benchmarks/native-paint-shell/profiling-pauses-1788707539/README.md`.
Additional `host.hud_scene`, `host.hud_composite`, and `host.surface_present`
spans separate scene preparation, GPU composition/submission, and surface presentation.
Three fresh captures passed the additional stage checks without record loss.
Their longest 15.070/16.984/18.358 ms presentation intervals contained
14.896/16.865/18.236 ms inside `GPUCanvasContext::present()` and no application RPC.
See `docs/benchmarks/native-paint-shell/profiling-present-stages-1788708051/README.md`.
The call includes wgpu surface presentation and texture-handle release. These wall-time
measurements do not distinguish driver waits, GPU work, compositor waits, or host preemption.
Driver-level causes and the earlier one-second interruptions remain open.
The optional native probe `--strace` completed after tracer cleanup corrections.
A further trace identifies long polls on Linux `anon_inode:sync_file` fences, not RPC sockets.
The longest two polls took 27.729 and 25.251 ms. The controlling GPU or driver operation remains unknown.
No explicit mapping connects the trace wall clock to the capture clock.
See `docs/benchmarks/native-paint-shell/profiling-syscalls-1788709350/README.md`.

The later visibility experiment identifies a separate cause of the one-second symptom.
Moving only the diagnostic window to an inactive workspace reproduced a 997.555 ms interruption without a collector.
That control had no recording and zero transmitted batches.
A matching trace placed 993.773–999.398 ms of four long intervals in surface presentation, with no overlapping application RPC.
A stack check identifies `wl_display_dispatch_queue` inside NVIDIA presentation below `NativeSwapchain::present`.
The main thread waits for Wayland events, which also delays JavaScript, input dispatch, and RPC response processing.
This is a verified visibility-related trigger, not proof of the cause of every historical pause.
The exact internal driver condition remains unknown. No runtime presentation policy changed.
See [the control report](../benchmarks/native-paint-shell/profiling-pause-visibility-controls-1788731282/README.md)
and [the socket stack](../benchmarks/native-paint-shell/profiling-pause-visibility-stacks-1788731483/README.md).
These checks do not establish foreground frame budgets, capture overhead, or sustained performance.
These trace timings are not performance acceptance evidence.
`afterglowProfiling.add(name, recorder)` accepts at most seven further sources.
`afterglowProfiling.stats()` returns connection, failure, batch, and byte counts.

The engine helper `attachProfiling(name, recorder)` returns false without a profiling
owner or after capacity rejection. Engine callers retain storage ownership through
`EngineMemory`. The paint prototype attaches `afterglow-engine.paint` with native
stroke admission and raster publication records, or browser stroke and worker-message records.
Its fixed 40 KiB recorder remains inside the prototype allocation boundary, not a `GameplaySealed` guarantee.

`initializeWebProfiling({workerUrl, transportUrl, serverUrl?})` supplies the browser owner.
It never starts a browser worker on the native target. The browser must be cross-origin isolated.
The default server URL is `ws://127.0.0.1:8086/`. The diagnostics worker uses the existing
RPC rings through `installRingService`, which also supplies the OPFS worker loop.
Only bootstrap data and payload-free wake-ups use `postMessage` for this service.
The worker owns `WebSocketCaptureClient`, one socket, a five-second connection deadline,
and at most one retry per second. Socket admission cannot exceed 65,536 buffered bytes.
The browser owner accepts eight caller-owned recorders and uses the same serial capture pump.
The browser pixel probe and actual WebSocket capture passed. Sustained timing and memory checks remain open.

No trace contains text input, file paths, pointer coordinates, or paint pixels.
Metadata serialization, timers, promises, and batch transfer are cold allocation
paths. The hot record writers retain fixed storage. Capture failure must not stop
the application. The shell build uses Bun to compile its TypeScript capture entry.

The previous TCP check passed 29 telemetry tests and two native adapter tests.
This includes connection, reconnection, file capacity, invalid data, application
exit, idle deadline, and slow-viewer checks. Scoped strict Clippy, the all-target
shell check, TypeScript compilation, and mdBook build also passed.
Dependency-wide strict Clippy still reports existing RPC warnings. The shell
check and mdBook build retain their existing compiler/preprocessor warnings.

## Clock domains

`MonotonicClock` gives every native producer in one process a common nanosecond
origin. Other producers register a `ClockMapping` containing source origin,
reference origin, rational tick conversion, and uncertainty. Zero denominators,
overflows, pre-origin ticks, and timestamp regressions are rejected.

Browser-worker clock synchronization and WebGPU clock calibration are adapter
responsibilities. GPU timestamps remain on a separate clock track correlated by
frame/submission ID when WebGPU cannot expose exact CPU/GPU calibration.

## Bounded field prototype

The [field codec reference](telemetry-fields.md) describes matching Rust and TypeScript layouts for bounded typed fields.
The codec is separate from the active record ABI.
Capture integration and registered schemas remain open.

## TypeScript API

`crates/afterglow-telemetry/web/src/telemetry.ts` owns the generic TypeScript
implementation. It uses the same 40-byte ABI, one caller-owned `ArrayBuffer`,
and preallocated typed arrays.

`Telemetry` combines the trace recorder and metric bank without an engine
import. A standalone application can supply buffers and a clock directly.
`engine/telemetry/telemetry.ts` re-exports that class as `EngineTelemetry` and
adds only `TelemetryRes` registration. It does not copy the recorder.
The engine allocation lint also examines the generic source.

The engine adapter uses this API:

```ts
const telemetry = new EngineTelemetry(
  traceDescriptors,
  metricDescriptors,
  memory.telemetryTrace,
  memory.telemetryMetrics,
);

telemetry.trace.arm(1);
telemetry.trace.spanBegin(Pread, requestId, offset, bytes);
telemetry.trace.spanEnd(Pread, requestId, bytes, status);
telemetry.trace.stop();

const snapshot = telemetry.trace.snapshot();
const required = telemetry.trace.encodedBatchBytes();
telemetry.trace.encodeBatchInto(callerBuffer, {
  session, sourceId, generation, clockDomain, clockGeneration,
});
```

For a rolling snapshot, use:

```ts
telemetry.trace.arm(epoch, categoryWords, TelemetryCaptureRetention.Rolling);
// Record through the same scalar methods.
telemetry.trace.stop(); // Cold O(capacity) operation.
const window = telemetry.trace.snapshot();
// window.overwritten counts replaced records. window.dropped counts rejected writes.
```

The snapshot and its buffer retain their original identity through wrap and reset.
The default retention remains `TelemetryCaptureRetention.Prefix`.
TypeScript loss counters saturate at `Number.MAX_SAFE_INTEGER`.
Rust loss counters saturate at `u64::MAX`.
These limits bound counter precision, not a promised history duration.

The TypeScript hot API uses non-negative safe-integer ticks, correlation IDs,
and arguments and splits them into low/high `u32` words. The browser clock is
monotonic nanoseconds with a declared `1_000_000_000` ticks/second. It does not
use `BigInt`, closures, promises, dynamic strings, or new typed-array views
while recording.

`EngineMemoryConfig` requires explicit `telemetryRecords` and
`telemetryMetricCells` capacities. `EngineRuntime` constructs one
`EngineTelemetry`, exposes it as `runtime.telemetry`, and publishes it through
`TelemetryRes`. It always updates frame count/delta/max metrics. When armed, it
records the complete frame, worker poll, VT update, structural command, pose,
render preparation, game update, and render-pass spans.

The default catalog also covers BIG session startup, source size/identity,
single and bulk range reads, native page-side RPC round trips, perceptual
feedback detection, scheduler wait, three-tier bulk-queue wait/dispatch, texture queue/transcode,
complete VT page loads, atlas/page-table upload publication, and whole-scene
mesh optimization. The obsolete cache descriptors are removed. Profile parsers
use the current engine catalog rather than a copied ID table.
These paths share numeric page/request correlations. Always-on
metrics include asset bytes/read latency, RPC calls/latency, VT requested/
loaded/failed pages, upload latency, and transcode latency.

The Dungeon diagnostic profile reserves 65,536 records (2.5 MiB). The
pre-removal RTX 3090 nine-pose baseline wrote 24,850 records with zero drops and
zero unmatched spans. Raw AGTB evidence and methodology are in
`docs/benchmarks/dungeon-vt-unified-telemetry-rtx3090-2026-07-25.*`.

## Current integration boundary

Implemented now:

- standalone Rust records, recorder, metrics, batch codec, collector, raw
  export, and Chrome export;
- exact TypeScript record/batch ABI, metrics, allocation contracts, and tests;
- `EngineMemory` ownership and `EngineRuntime`/`FrameBudget` page tracks;
- web/native range-loader, BIG session, bulk batching, transcode, native RPC
  round-trip, mesh optimization, VT feedback/scheduler/load, and VT publication
  tracks.

Still to be composed:

- generated RPC server-side flow endpoints and worker-local batch draining;
- native worker-internal source-backed `pread` and codec scopes;
- audio callback/simulation adapters;
- shell capture control/file output;
- `GpuProfiler` timestamp ingestion;
- removal of superseded subsystem-specific trace exporters.

Those integrations must feed this crate rather than add another telemetry
transport or event format.

## Validation

```sh
cargo test -p afterglow-telemetry
cargo test -p afterglow-telemetry --features collector
cargo clippy -p afterglow-telemetry --all-targets -- -D warnings
cargo run --release -p afterglow-telemetry --example bench_telemetry
bun test crates/afterglow-telemetry/web/src/telemetry.test.ts
bun test crates/afterglow-web/web/src/engine/telemetry/telemetry.test.ts
bun crates/afterglow-web/web/node_modules/typescript/bin/tsc \
  --project crates/afterglow-telemetry/tsconfig.json
bun scripts/lint-hot-allocations.ts
```

`xtask check` includes the standalone TypeScript and proposed contract checks.
`xtask test` includes standalone producer tests, engine adapter tests, and the
optional collector connection/process tests.

The Rust suite includes an allocation-tracking regression proving disabled and
enabled recording plus metric updates allocate nothing.
It also checks 100,003 rolling writes and the final buffer rotation without allocation.
Both languages check every wrap position across capacities 1–9 and compare record
full batch bytes with `crates/afterglow-telemetry/tests/fixtures/rolling-batch.hex`.
Decoder tests reject malformed inputs without partial output changes.
The microbenchmark now reports rolling writes and cold freeze time separately.
Its prefix and rolling buffers have different capacities, so the result is not
an equal-memory comparison or a sustained application overhead test. On fox-laptop on
2026-07-25, the simple release microbenchmark reported approximately 1.10 ns per
disabled call and 20.37 ns per enabled event over one million events. This is a
local mechanism check, not a cross-machine performance guarantee.
