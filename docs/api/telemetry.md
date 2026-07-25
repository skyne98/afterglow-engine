# Unified telemetry API

`afterglow-telemetry` is the transport-neutral telemetry substrate for native
Rust, worker, TypeScript, host, and GPU adapters. It combines two correlated
planes:

- fixed always-on metrics: counters, gauges, maxima, and 32-bucket log2
  histograms;
- explicitly armed finite traces: synchronous spans, asynchronous spans,
  instant events, and cross-track flows.

The crate does not spawn threads, open sockets, format hot-path strings, or
create a worker transport. Worker batches must use the existing
`afterglow-rpc::RingBuffer` mechanism.

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

A full buffer preserves its existing prefix, drops new records, increments the
saturating dropped counter, and returns `RecordStatus::BufferFull`. It never
blocks, wraps, grows, or silently overwrites.

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

`encode_batch_into` and `decode_batch_into` read/write versioned `AGTB` v1
batches without allocation. The fixed header is 40 bytes followed by contiguous
40-byte records. Decoding validates magic, version, lengths, record count, and
output capacity.

`Collector` is cold-path code. It registers source-local descriptor tables,
validates batches, applies explicit `ClockMapping`s, ingests metric snapshots,
and owns the diagnostic copies after producers freeze. It exports:

- lossless versioned `AGTL`/`.agt` data with source, descriptor, clock, trace,
  metric, and drop metadata;
- streaming Catapult/Chrome Trace JSON through `std::io::Write`, directly
  readable by Perfetto.

The Chrome exporter performs a bounded-source k-way timestamp merge rather than
constructing one JSON object containing every event. Metric snapshots become
Chrome counter events.

## Clock domains

`MonotonicClock` gives every native producer in one process a common nanosecond
origin. Other producers register a `ClockMapping` containing source origin,
reference origin, rational tick conversion, and uncertainty. Zero denominators,
overflows, pre-origin ticks, and timestamp regressions are rejected.

Browser-worker clock synchronization and WebGPU clock calibration are adapter
responsibilities. GPU timestamps remain on a separate clock track correlated by
frame/submission ID when WebGPU cannot expose exact CPU/GPU calibration.

## TypeScript API

`engine/telemetry/telemetry.ts` mirrors the same 40-byte ABI using one
caller-owned `ArrayBuffer` and preallocated typed arrays:

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
telemetry.trace.encodeBatchInto(callerBuffer, sourceId, clockDomain);
```

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
single and bulk range reads, native page-side RPC round trips, feedback
detection, scheduler wait, bulk-queue wait/dispatch, texture queue/transcode,
complete VT page loads, atlas/page-table upload publication, and whole-scene
mesh optimization. Descriptor IDs 19–20 remain reserved solely to decode
historical pre-removal cache captures; current code emits neither. These paths share numeric page/request correlations. Always-on
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
- native worker-internal `pread`, arena lease, and codec scopes;
- audio callback/simulation adapters;
- shell capture control/file output;
- `GpuProfiler` timestamp ingestion;
- removal of superseded subsystem-specific trace exporters.

Those integrations must feed this crate rather than add another telemetry
transport or event format.

## Validation

```sh
cargo test -p afterglow-telemetry
cargo clippy -p afterglow-telemetry --all-targets -- -D warnings
cargo run --release -p afterglow-telemetry --example bench_telemetry
cd crates/afterglow-web/web
bun test src/engine/telemetry/telemetry.test.ts
```

The Rust suite includes an allocation-tracking regression proving disabled and
enabled recording plus metric updates allocate nothing. On fox-laptop on
2026-07-25, the simple release microbenchmark reported approximately 1.10 ns per
disabled call and 20.37 ns per enabled event over one million events. This is a
local mechanism check, not a cross-machine performance guarantee.
