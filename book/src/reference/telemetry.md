# Unified Telemetry

Afterglow has one bounded telemetry substrate for metrics and correlated traces.
The Rust implementation lives in `afterglow-telemetry`; the TypeScript runtime
uses the same 40-byte little-endian trace record ABI.

## Metrics versus traces

Metrics are cheap and always active:

- counters;
- gauges;
- maxima;
- fixed 32-bucket logarithmic histograms.

Traces are explicitly armed for a finite diagnostic window. They record spans,
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
update, and render-pass ranges.

```ts
runtime.telemetry.trace.arm(1);
// Run a bounded scenario.
runtime.telemetry.trace.stop();

const snapshot = runtime.telemetry.trace.snapshot();
const output = new Uint8Array(runtime.telemetry.trace.encodedBatchBytes());
runtime.telemetry.trace.encodeBatchInto(output, 1, 1);
```

The output is an `AGTB` v1 batch. A full capture preserves existing records,
drops new records, and reports the exact dropped count. It never blocks or grows.

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
before reading the clock. Worker batches must be carried through the existing
RPC rings—telemetry is not another transport.

The Rust collector validates source descriptors, clocks, record ordering, and
batch bounds. It streams Chrome Trace JSON readable by Perfetto and can preserve
the lossless `.agt` representation.

## Current scope

The core crate and TypeScript frame/runtime integration are implemented. RPC
flow generation, worker draining, native asset/texture/audio scopes, shell file
capture, and GPU timestamp ingestion are subsequent integration gates. New
subsystem profilers should feed this API rather than create another event format
or Chrome exporter.

See [`docs/api/telemetry.md`](../../../docs/api/telemetry.md) for the complete API
and `docs/implementation/unified-telemetry-plan.md` for migration status.
