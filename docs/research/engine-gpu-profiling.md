# Engine GPU performance profiling — research

How the most advanced and elegant performance tools in graphics engines work,
what "exact shader/system times" actually means, and how it maps to this
engine. Investigated 2026-07; sources inline.

## The core distinction

"Exact shader/system times" is **two-tier**:

1. **Engine-ownable (API timestamp queries):** per-pass and per-named-scope GPU
   execution times, measured with API timestamp queries. This is the
   *how-long* tier. Every engine can do this.
2. **Hardware-exact (vendor profilers):** per-unit speed-of-light (SOL)
   saturation, warp/wavefront stall reasons, shader-ISA-level analysis. This is
   the *why* tier. Only vendor tools with driver/hardware counters can do this.

Elegant in-engine profilers (Tracy, wgpu-profiler, Unity, Unreal) converge on
the same Layer-1 design and deliberately leave Layer 3 to vendor tools. The
deep "why" is never engine-ownable.

## Layer 1 — API timestamp queries (the foundation)

Every modern API (WebGPU, Vulkan, D3D12, Metal) exposes timestamp queries: ask
the GPU to stamp a 64-bit nanosecond counter at a point in the command stream,
read it back asynchronously. This is the only way to measure *actual GPU
execution time* — CPU timers measure submission, not execution.

### The WebGPU mechanism (this engine's stack)

Source: [webgpufundamentals — WebGPU Timing][1], [Chrome Intent to Ship:
timestamp queries][2].

1. Require the optional `'timestamp-query'` device feature.
2. Create a `GPUQuerySet` (`type: 'timestamp'`, `count ≥ 2`) — an array of
   64-bit slots.
3. In a pass, write timestamps. Two flavors:
   - **Pass-boundary writes** (cheap, preferred): pass `timestampWrites` to
     `beginRenderPass`/`beginComputePass`:
     `{ querySet, beginOfPassWriteIndex: 0, endOfPassWriteIndex: 1 }`.
   - **Arbitrary writes inside a pass/encoder** (expensive, deep-dive only):
     `pass.writeTimestamp(querySet, i)` — requires the extra features
     `TIMESTAMP_QUERY_INSIDE_ENCODERS` / `TIMESTAMP_QUERY_INSIDE_PASSES`.
4. `encoder.resolveQuerySet(querySet, 0, 2, resolveBuffer, 0)` → copies results
   into a buffer.
5. `encoder.copyBufferToBuffer(resolveBuffer → resultBuffer)` then
   `resultBuffer.mapAsync('read')` → read as `BigUint64Array`.
6. Multiply by `queue.getTimestampPeriod()` (GPU ticks → ns; some GPUs tick at
   non-nanosecond rates — don't assume).
7. `elapsed = Number(end - start)` — subtract as BigInt first to keep 53-bit
   `Number` precision.

[1]: https://webgpufundamentals.org/webgpu/lessons/webgpu-timing.html
[2]: https://groups.google.com/a/chromium.org/g/blink-dev/c/dtYJ0MQYMlU

### The two subtleties everyone gets wrong

- **GPU results lag the CPU by N frames.** A timestamp written this frame is not
  available until the GPU finishes — typically 2–4 frames later. Reading it back
  immediately stalls the pipeline. **Elegant profilers rotate K query pools**
  (double/triple buffering): during frame F they resolve+read-back the pool from
  frame F−K, never stalling. wgpu-profiler's `process_finished_frame` returns
  the *oldest available* frame, not the current one.
- **Browsers quantize timestamps to 100 µs** to mitigate timing-side-channel
  attacks. Chrome's `enable-webgpu-developer-features` flag removes
  quantization. 100 µs is enough to compare shader techniques, but it hides
  sub-100 µs work (a single cheap draw). Always document the quantization when
  reporting numbers.

### The three feature tiers

wgpu-profiler's model is the cleanest statement:

| Feature | Allows | Cost |
|---|---|---|
| `TIMESTAMP_QUERY` | writes at pass begin/end only | cheapest; always use for per-pass |
| `TIMESTAMP_QUERY_INSIDE_ENCODERS` | between passes, in the encoder | medium |
| `TIMESTAMP_QUERY_INSIDE_PASSES` | per-draw / per-dispatch inside a pass | expensive (burns slots fast) |

Source: [wgpu-profiler][3].

[3]: https://github.com/Wumpf/wgpu-profiler

### The scope abstraction (the elegant pattern)

Tracy, wgpu-profiler, Unity (`Profiling.SampleBegin`), Unreal
(`SCOPED_GPU_DRAW_STATS`) all converge on: wrap the encoder/pass in an RAII
scope guard that writes begin/end timestamps and auto-closes on drop. Nested
scopes → a flamegraph. This decouples "where to measure" from "how to read it
back." wgpu-profiler:

```rust
let mut scope = profiler.scope("name", &mut encoder);
let mut nested = scope.scope("nested!");            // arbitrary nesting
let mut pass = nested.scoped_compute_pass("compute"); // profiled pass
// auto-closed on drop
```

**WebGPU `pushDebugGroup`/`popDebugGroup`** serves the same attribution role
for external tools — RenderDoc, PIX, Nsight, and Tracy all read these group
names to label GPU ranges. Emitting debug groups alongside timestamp scopes
means a vendor-tool capture shows your engine's scope names.

## Layer 2 — Frame debuggers (correctness, not timing)

RenderDoc, Spector, WebGPU Inspector (Chrome extension). These are **frame
debuggers, not profilers**: capture one frame's full command stream, scrub
draw-by-draw, inspect bound textures/buffers, edit shaders live. They give "is
this draw correct + what does it touch," not "how long." Per-draw timing in
RenderDoc is rough (repeat-frame heuristic), not authoritative. Use them for
correctness, then Layer 1/3 for timing.

WebGPU Inspector runs inside Chrome — RenderDoc-style capture + a frame-time
plot + live shader editing — and works in CEF on the 680M with no engine
changes.

## Layer 3 — Vendor profilers (the deep "why")

This is where "exact shader/system times" actually lives, using driver/hardware
counters that engine APIs cannot expose. They measure not just *how long* but
*why* — how close each GPU unit is to its theoretical peak (SOL).

### NVIDIA Nsight Graphics — "GPU Trace" (state of the art for NVIDIA)

Source: [NVIDIA — Migrating Range Profiler to GPU Trace][4], [Nsight Shader
Profiler guide][5].

- **Time-series, not range.** The old Range Profiler gave one number per region;
  GPU Trace streams metrics over the whole frame timeline (workloads overlap, so
  summing per-range is wrong).
- **Unit throughputs (SOL):** for every GPU stage — Primitive Distributor (PD),
  Vertex Attribute Fetch (VAF), Rasterizer, ZROP/CROP (depth/color ROPs), SM
  (shader), L1TEX, L2, VRAM — % of that unit's peak. The unit at ~100% SOL is
  the bottleneck. **Profiling by saturation, not by time.**
- **Warp Stall reasons** (Advanced Mode): per-warp, *why* the SM stalled — Long
  Scoreboard (memory latency), TEX Throttle (texture unit busy), Not Selected
  (occupancy too high — could use more registers/shared mem), IMC miss (texture
  cache miss), wait (sync). The deepest "why is my shader slow" answer.
- **SM Occupancy** (warps active per stage, register/shared-memory pressure).
- **Shader Profiler / ISA:** disassembles the shader to vendor ISA and shows
  stall reasons *per instruction*.

[4]: https://developer.nvidia.com/blog/migrating-from-range-profiler-to-gpu-trace-in-nsight-graphics/
[5]: https://docs.nvidia.com/nsight-graphics/UserGuide/shader-profiler.html

### AMD Radeon GPU Profiler (RGP) — the RDNA equivalent (the 680M)

The tool for the engine's 680M target. Unreal's own performance guide says: "use
RGP in lieu of GPU Visualizer for RDNA — more accurate timings + low-level ISA
analysis." RGP shows wavefront occupancy, shader-ISA stall reasons,
texture-cache/L2/memory-bandwidth breakdown, and per-clk throughput. **This is
the tool that would diagnose the 33 ms base-render regression observed on the
680M** (texture-fetch-bound? ROP-bound at 2880×1800? L2-bound?) — the question
no in-engine profiler can answer.

### Microsoft PIX / Intel GPA

PIX (D3D): per-pass GPU timing, timing captures, draw-level GPU capture,
`PIX_EVENT_BEGIN/END` runtime markers that other tools also read. Intel GPA:
Platform Analyzer + Frame Analyzer, same SOL/counter model.

### The common deep-tool pattern

Capture a frame with markers (your `pushDebugGroup` names appear), then the tool
replays hardware counters and attributes them to your named scopes + to hardware
units. You get a flamegraph of your passes overlaid with SOL saturation and
stall reasons: "scope name → time → which unit is saturated → why." That
attribution is what "exact shader/system times" means in practice.

## Layer 4 — Frame/sampling profilers (CPU+GPU timeline)

Bridge CPU and GPU across frames: a timeline of named scopes from both sides,
plus memory/locks/context-switches.

### Tracy — the elegant gold standard

Source: [wolfpld/tracy][6]. Real-time, nanosecond, remote-telemetry, **hybrid**
(manual instrumentation zones + statistical sampling). GPU zones for all APIs
**including WebGPU**. One timeline showing CPU zones, GPU zones, memory
allocations, locks, and context switches correlated by time — so the CPU→GPU
handoff latency is visible. Exports to chrome-tracing JSON.

[6]: https://github.com/wolfpld/tracy

### Unreal Insights

`trace=default,gpu,memory`; `stat gpu`/`stat unit`/`stat scenerendering`
overlays; `ProfileGPU` (GPU Visualizer) for per-pass breakdown; integrates with
RGP/Nsight for deep dives. Cleanest engine-internal model: named timing scopes
feed trace channels; the GPU channel is backed by timestamp queries.

### Chrome `chrome://tracing` — the universal interchange

wgpu-profiler, Tracy, and Unreal Insights all export to it. The
`BenchmarkInstrumentation:*` events + `disabled-by-default-gpu.device` /
`gpu.service` tracing categories give Chrome-internal GPU timing. This repo's
`latency-tool` already uses a CDP trace mode for swap counting — but AGENTS.md
notes RADV undercounts swaps on the 680M, so rAF interval timing is the
reliable fallback there.

## The elegant-engine pattern (what to copy)

The best in-engine profilers all converge on:

1. **RAII scope guard** writing begin/end timestamps, auto-close on drop —
   `scope("POM march", encoder)` — nesting = flamegraph.
2. **K-buffered query-pool rotation** — never stall; read back frame F−K during
   frame F.
3. **Async resolve path** — `resolveQuerySet → copyBuffer → mapAsync` on a
   separate buffer; results arrive a few frames late, tagged with origin frame.
4. **`timestamp_period` + rolling average** — handle non-ns ticks and per-frame
   noise.
5. **`pushDebugGroup` names** shared with external tools (RenderDoc/PIX/Nsight/
   Tracy) so a capture shows your scope names.
6. **Chrome-trace JSON export** — universal viewer, no proprietary UI.
7. **SOL/stall-reason depth left to vendor tools** — the engine provides
   *time-per-scope*; the *why* comes from RGP/Nsight on the captured frame.

## This engine's current state + gap

The repo has a **thin Layer-1 integration**:
`VirtualTextureFeedbackCoordinator.setGpuTimingEnabled` toggles Three r185's
private `backend.trackTimestamp` / `timestampQueryPool`, and
`resolveGpuTimings()` calls `renderer.resolveTimestampsAsync('render')` to get
**two coarse numbers** — `gpuMainMs` and `gpuFeedbackMs` (whole main render
pass, whole feedback pass). That is pass-level timing via Three's internal
timestamp-query plumbing. It works (the dungeon `timing` object surfaces
`gpuMainMs`/`gpuFeedbackMs`/`gpuTotalMs`).

**Gap vs the elegant/advanced pattern:**

- Only 2 whole-pass numbers, not arbitrary named scopes. No RAII scope
  abstraction; no per-pass breakdown (POM pass, VT feedback, shadow pass) beyond
  main/feedback.
- Relies on Three r185 private `backend.timestampQueryPool` internals (the
  `@unsafe-cast DME-030` accesses in `virtual-texture-feedback-coordinator.ts`)
  — fragile, not first-class.
- No K-buffered rotation of its own (defers to Three's internals).
- No chrome-trace JSON export; no `pushDebugGroup` scope names for external
  tools.
- No SOL/stall-reason path (vendor-tool territory — RGP for the 680M, Nsight for
  the workstation dGPU).

## 680M RGP result (2026-07-21)

The RADV/RGP gate is now complete. Two Radeon GPU Profiler 2.7 captures compared
the same 2880×1800 Dungeon view with feedback disabled and POM on/off. One
full-coverage, two-triangle material draw dominated both captures:

- base: 4.824 ms event duration, 40 FS VGPR, 12/16 occupancy;
- POM: 5.749 ms event duration, 56 FS VGPR, 9/16 occupancy;
- POM delta: +0.924 ms / +19.2% for the dominant draw;
- both shaded about 4.56 million pixels with no scratch spills;
- RGP explicitly identifies vector-register use as the occupancy limiter.

These are trace-local attribution numbers, not production timings. SQTT tracing
was active and the safe immediate capture preceded settled fine-page residency.
A subsequent audit also invalidated the old 10.63 ms engine timestamp: Afterglow's
external rAF loop left Three's frame ID at zero, making each resolve group all
unresolved passes under one frame, while `gpuMainMs` selected the ~1.07 ms output
color-transform pass rather than the internal HDR scene pass. With frame identity
temporarily corrected, settled scene-plus-output totals were 4.19/4.28/5.84 ms
non-POM means and 6.56/5.49/8.29 ms POM means across forward/reverse/corner;
worst pose p99 was 10.49 ms. RADV's compiler dump still shows why the resolver
is structurally non-trivial (1,135 static instructions, 14 image operations, and
44 branches versus 287/2/2 for constant standard PBR), but it does not assign a
production duration to those instructions. See
[`amd-rgp-radv-capture-methodology.md`](amd-rgp-radv-capture-methodology.md) for
screenshots, exact metrics, the audit, safe RADV capture commands, NixOS viewer
setup, CEF frame-delimiter caveats, and the failed long-trace attempt.

## Recommended path

1. **In-engine (high-value, engine-ownable):** a first-class `GpuProfiler`
   (wgpu-profiler-style) — `scope()` guard over encoder/pass, K-rotated query
   pools, async resolve, `timestamp_period` scaling, chrome-trace export. See
   the proposed design below.
2. **Deep "why" for the 680M:** **AMD Radeon GPU Profiler (RGP)** — completed
   for event timing, wavefront occupancy, register pressure, and spills. Current
   RADV captures expose no API shader hash/instruction-timing data, so
   source/ISA hotspots and instruction-level stalls remain unavailable.
3. **WebGPU-browser, zero-engine-change:** WebGPU Inspector Chrome extension +
   `chrome://tracing` with `gpu` categories — works in CEF on the 680M today.

## Proposed engine `GpuProfiler` design (Layer-1 upgrade)

A bounded, sealed-runtime-safe, allocation-free-at-steady-state profiler
matching the engine's frame-budget rules. Replaces the fragile Three-internal
`backend.timestampQueryPool` path with a direct WebGPU query pool.

### Public surface

```ts
class GpuProfiler {
  constructor(device: GPUDevice, queue: GPUQueue, options?: { framesInFlight?: number; maxScopesPerFrame?: number });
  // Begin a profiling frame. Returns a handle used to open scopes.
  beginFrame(): GpuFrameScope;
  // Resolve + read back the oldest in-flight frame; call once per frame after
  // encoder.finish(). Results arrive `framesInFlight` frames late.
  endFrame(encoder: GPUCommandEncoder): void;
  // Pull the oldest completed frame's scopes (empty until the pipeline drains).
  // @alloc-effect none at steady state (reuses a fixed result buffer).
  poll(): readonly GpuScopeTiming[];
  // Chrome-tracing JSON (Catapult format) over N polled frames.
  // @alloc-effect diagnostic (only when exporting).
  exportChromeTrace(scopes: readonly GpuScopeTiming[]): string;
  dispose(): void;
}

interface GpuFrameScope {
  // Attach begin/end timestamp writes to a render/compute pass descriptor.
  // Mutates the pass descriptor's `timestampWrites` in place — no extra pass.
  withPass(name: string, descriptor: GPURenderPassDescriptor | GPUComputePassDescriptor): typeof descriptor;
  // (Optional, requires INSIDE_ENCODERS) a manual between-pass scope.
  scope(name: string, encoder: GPUCommandEncoder): GpuZone;
}

interface GpuScopeTiming { readonly name: string; readonly startNs: bigint; readonly endNs: bigint; readonly durationMs: number; }
interface GpuZone { end(): void; } // RAII manual close
```

### Internals (the design that makes it elegant)

- **K-rotated frame slots** (`framesInFlight` default 3 — matches typical GPU
  lag). Each slot holds: a `GPUQuerySet` of `2 * maxScopesPerFrame` slots (begin
  + end per scope), a resolve buffer, a mappable result buffer, and a fixed
  `GpuScopeTiming[]` filled on readback. Slots cycle round-robin.
- **Per-frame scope index** — `withPass(name)` allocates the next begin/end pair
  in the current frame's query set, records `name → [beginIdx, endIdx]`, and sets
  `descriptor.timestampWrites` to point at those slots. Zero extra draw work;
  the GPU stamps on pass begin/end.
- **endFrame** resolves the current frame's queryset into its resolve buffer and
  copies to the mappable buffer. Readback (`mapAsync`) is issued lazily when
  `poll()` reaches that slot K frames later.
- **`timestamp_period`** applied at readback: `durationMs = Number(end-start) *
  period / 1e6`.
- **No steady-state allocation:** the result array is fixed-capacity and
  overwritten each `poll()`; `exportChromeTrace` allocates (diagnostic, not a
  hot path).
- **Feature gating:** requires `'timestamp-query'`. If absent, `GpuProfiler`
  operates as a no-op (scopes record nothing, `poll()` returns `[]`). Fail-open,
  never fatal — matches the engine's "telemetry, not correctness" stance for
  profiling.

### Adoption plan (separate implementation pass)

1. Introduce `GpuProfiler` as a standalone module + unit tests (mock device).
2. Wire the **engine-owned passes** (VT feedback pass in
   `virtual-texture-feedback-coordinator.ts`) to use it directly — these are
   created by the engine, so `withPass` is trivial.
3. For the **Three-driven main render** (`renderer.render(scene, camera)`), keep
   the existing Three-internal `resolveTimestampsAsync('render')` as an adapter
   *until* the engine owns its main pass creation. The `GpuProfiler` then
   provides per-scope timing for everything the engine controls directly.
4. Emit `pushDebugGroup`/`popDebugGroup` names alongside `withPass` so RGP/Nsight
   captures show the same scope names.
5. Keep the validated 680M RGP runbook current and validate the workstation path
   separately with Nsight GPU Trace.

This keeps the engine-ownable Layer-1 work KISS and bounded, leaves the deep
"why" to RGP/Nsight, and removes the fragile `@unsafe-cast DME-030` dependency
on Three internals for the passes the engine controls.
