# Unified telemetry implementation plan

Status: **foundation and page-frame integration implemented; cross-domain
adapters pending**. Approved product/failure-policy defaults on 2026-07-25.

Canonical public API: [`docs/api/telemetry.md`](../api/telemetry.md).

## Fixed decisions

- Own workspace crate: `afterglow-telemetry`, initially unpublished.
- Local diagnostics only: no network, analytics, PII, or automatic upload.
- Metrics remain active in release; finite trace capture is dormant until armed.
- Full trace buffers preserve their prefix and drop new records with an exact
  counter. They never block or overwrite silently.
- No dynamic strings in sealed hot paths.
- Initial outputs are lossless `.agt` and streaming Chrome Trace JSON.
- Worker data uses `afterglow-rpc::RingBuffer`; telemetry creates no second
  transport.
- RPC flow correlation initially derives from connection/task identity rather
  than increasing every service payload.

## Architecture invariant

There is one telemetry vocabulary and binary record ABI. Different storage is
allowed only where the signal semantics require it:

- fixed atomic/local cells for always-on metrics;
- producer-local finite arrays for armed traces;
- WebGPU query buffers for GPU intervals before they are adapted into trace
  records.

Subsystem-specific counters may remain as implementation state, but their
external snapshot/trace view must feed unified descriptors. No subsystem may
add another Chrome exporter after migration.

## Phase 1 — core crate — complete

- [x] Static producer-local descriptor metadata.
- [x] Fixed 40-byte trace record.
- [x] Instant, synchronous, async, and flow phases.
- [x] Producer-local `Idle → Armed → Frozen → Idle` capture lifecycle.
- [x] 256-category precomputed filter.
- [x] Deterministic prefix-preserving overflow.
- [x] RAII Rust span guard with nested event support.
- [x] Fixed counters, gauges, maxima, and 32-bucket log2 histograms.
- [x] Allocation-free `AGTB` v1 encode/decode.
- [x] Explicit clock mapping and uncertainty.
- [x] Cold collector with descriptor/batch validation.
- [x] Metric snapshot ingestion.
- [x] Streaming Chrome Trace JSON and lossless `AGTL` raw output.
- [x] Unit, malformed-input, state-machine, and no-allocation tests.
- [x] Release microbenchmark example.

## Phase 2 — TypeScript and runtime integration — complete

- [x] Exact little-endian 40-byte TypeScript ABI without `BigInt`.
- [x] Caller-owned finite trace buffer and metric cells.
- [x] Allocation-free numeric trace/metric hot methods.
- [x] Allocation-free `AGTB` encoding into caller-owned output.
- [x] Explicit `EngineMemoryConfig.telemetryRecords` and
      `telemetryMetricCells` capacities.
- [x] `EngineTelemetry` facade and `TelemetryRes` ECS resource.
- [x] Default engine category/descriptor catalog.
- [x] Always-on frame count/delta/max metrics.
- [x] Armed complete-frame, frame-stage, game-update, and render-pass spans.
- [x] TypeScript ABI, filtering, overflow, state, metric, and runtime vertical
      tests.

## Phase 3 — RPC flow adapter

- [ ] Give every generated connection a stable telemetry connection ID.
- [ ] Emit client-send/server-receive/server-send/client-receive flow endpoints.
- [ ] Derive async flow IDs from `(connection_id, task_id)`.
- [ ] Derive sync flow IDs from `(connection_id, monotonic_call_sequence)`.
- [ ] Add reserved telemetry arm/stop/drain control framing to worker loops,
      without exposing methods on service traits.
- [ ] Drain frozen worker batches through existing event/request/response rings.
- [ ] Add native and public-web vertical tests proving the same correlation.
- [ ] Prove disabled RPC instrumentation does not regress the established 64 B
      latency distribution beyond measurement noise.

## Phase 4 — asset and texture route

- [ ] Correlate VT request slot/generation with one trace context.
- [ ] Instrument scheduler wait and bulk timer wait.
- [ ] Instrument asset request queue, `pread`, and response publication.
- [ ] Instrument arena write/read leases and deterministic release.
- [ ] Instrument texture queue, RPC flow, Basis transcode, and result publication.
- [ ] Instrument ready-upload wait, atlas write, and page-table publication.
- [ ] Record the first frame whose visible draw may sample the published page.
- [ ] Acceptance gate: follow one Dungeon tile end-to-end in one exported trace.

## Phase 5 — GPU and presentation

- [ ] Adapt `GpuProfiler` output into unified descriptors and frame/submission
      correlations.
- [ ] Emit the same names as WebGPU debug groups.
- [ ] Keep uncalibrated WebGPU timestamps on a separate GPU track with explicit
      uncertainty; never imply exact CPU overlap.
- [ ] Add shell rAF dispatch, JS turn, queue submit, composite, and presentation
      events where the host exposes valid timestamps.
- [ ] Delete `GpuProfiler.exportChromeTrace` and `Profiling.exportChromeTrace`
      after equivalent unified output is validated.

## Phase 6 — audio and remaining workers

- [ ] Map audio callback, pump, simulation, mix, underrun, and fatal counters to
      unified metric descriptors.
- [ ] Arm audio trace recording only for bounded diagnostic windows; callback
      recording must remain lock-free and allocation-free.
- [ ] Add meshopt and future physics worker adapters through the RPC integration,
      not subsystem transports.

## Phase 7 — shell capture product surface

- [ ] Add explicit local arm/stop/export commands to the native shell.
- [ ] Reserve all source capacities during native bootstrap and expose high-water
      plus dropped-source telemetry.
- [ ] Stop with a deadline and emit missing-source markers rather than hanging.
- [ ] Write `.agt` atomically, then optionally convert to Chrome JSON.
- [ ] Add a deterministic capture test and cleanup temporary outputs.

## Release gates

- Disabled trace calls check state before clock access.
- Rust and TypeScript hot calls perform no general allocation.
- No producer contends on one global mutex or MPSC queue.
- Every capacity and overflow policy is explicit and test-covered.
- Malformed batches cannot panic, overrun output, or partially enter a collector.
- Clock-domain uncertainty is retained through export.
- Long trace-disabled and armed-window soaks plateau in memory.
- Worker telemetry uses the one RingBuffer payload mechanism.
- One Dungeon capture demonstrates the complete tile route.
