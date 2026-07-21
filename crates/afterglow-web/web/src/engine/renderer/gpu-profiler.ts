// First-class GPU timestamp-query profiler (Layer-1).
//
// Bounded, sealed-runtime-safe, allocation-free at steady state. Replaces the
// fragile Three-internal `backend.timestampQueryPool` path for passes the
// engine controls directly. See docs/research/engine-gpu-profiling.md.
//
// Design:
// - K-rotated frame slots (default 3, matching typical GPU lag). Each slot has
//   its own GPUQuerySet + resolve buffer + mappable result buffer. Results
//   arrive `framesInFlight` frames late; never stalls the pipeline.
// - Per-pass timestamp writes via `descriptor.timestampWrites` — zero extra
//   draw work; the GPU stamps on pass begin/end.
// - `timestamp_period` applied at readback. Fail-open: if the device lacks
//   `'timestamp-query'`, the profiler is a no-op (scopes record nothing).

/** One measured scope (begin/end pair) from a completed frame. */
export interface GpuScopeTiming {
  readonly name: string;
  readonly startNs: bigint;
  readonly endNs: bigint;
  readonly durationMs: number;
}

export interface GpuProfilerOptions {
  /** In-flight frames (round-robin slots). Default 3. Must be ≥ 1. */
  readonly framesInFlight?: number;
  /** Max scopes (passes) recorded per frame. Default 16. Must be ≥ 1. */
  readonly maxScopesPerFrame?: number;
}

/** A frame's profiling context. Obtain from `GpuProfiler.beginFrame()`. */
export interface GpuFrameScope {
  /**
   * Attach begin/end timestamp writes to a pass descriptor in place and return
   * it. The pass keeps its normal usage; the GPU stamps on pass begin/end.
   * Mutates `descriptor.timestampWrites`.
   */
  withPass<T extends GPURenderPassDescriptor | GPUComputePassDescriptor>(
    name: string,
    descriptor: T,
  ): T;
  /** (Optional, requires TIMESTAMP_QUERY_INSIDE_ENCODERS.) Manual zone. */
  scope(name: string, encoder: GPUCommandEncoder): GpuZone;
}

export interface GpuZone {
  end(): void;
}

interface FrameSlot {
  readonly querySet: GPUQuerySet;
  readonly resolveBuffer: GPUBuffer;
  readonly resultBuffer: GPUBuffer;
  /** Names per [begin,end] pair, in allocation order. */
  readonly names: string[];
  /** True once the result buffer has been mapped and read. */
  mapped: boolean;
  /** True once endFrame has resolved this slot's queries. */
  resolved: boolean;
}

const DEFAULT_FRAMES_IN_FLIGHT = 3;
const DEFAULT_MAX_SCOPES = 16;

/**
 * Bounded GPU timestamp-query profiler.
 *
 * Lifecycle: `beginFrame()` → `frameScope.withPass(name, passDesc)` for each
 * profiled pass → `endFrame(encoder)` → next frame, eventually `poll()` returns
 * the oldest completed frame's scopes. Results lag by `framesInFlight`.
 *
 * @alloc-effect none at steady state (fixed-capacity result buffer reused).
 * @alloc-effect diagnostic only in `exportChromeTrace`.
 */
export class GpuProfiler {
  private readonly slots: FrameSlot[] = [];
  private readonly maxScopes: number;
  private readonly capacity: number;
  private current = 0;
  private readonly framesInFlight: number;
  private readonly supported: boolean;
  private readonly period: () => number;
  /** Fixed-capacity result buffer reused by poll(); no steady-state allocation. */
  private readonly result: GpuScopeTiming[] = [];
  /** Pending per-frame scope index allocation. */
  private frameScopeCount = 0;

  constructor(
    private readonly device: GPUDevice,
    queue: GPUQueue,
    options: GpuProfilerOptions = {},
  ) {
    if (options.framesInFlight !== undefined && (!Number.isInteger(options.framesInFlight) || options.framesInFlight < 1))
      throw new RangeError('GpuProfiler framesInFlight must be a positive integer');
    if (options.maxScopesPerFrame !== undefined && (!Number.isInteger(options.maxScopesPerFrame) || options.maxScopesPerFrame < 1))
      throw new RangeError('GpuProfiler maxScopesPerFrame must be a positive integer');
    this.framesInFlight = options.framesInFlight ?? DEFAULT_FRAMES_IN_FLIGHT;
    this.maxScopes = options.maxScopesPerFrame ?? DEFAULT_MAX_SCOPES;
    this.capacity = this.maxScopes * 2; // begin + end per scope
    this.supported = device.features.has('timestamp-query');
    // timestamp_period may change; read lazily at readback. The TS lib types
    // omit getTimestampPeriod in some versions, so access defensively.
    this.period = () => (queue as unknown as { getTimestampPeriod?: () => number }).getTimestampPeriod?.() ?? 1;
    if (this.supported) this.initSlots();
  }

  private initSlots(): void {
    for (let i = 0; i < this.framesInFlight; i++) {
      const querySet = this.device.createQuerySet({
        type: 'timestamp',
        count: this.capacity,
      });
      const resolveBuffer = this.device.createBuffer({
        size: this.capacity * 8,
        usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC,
      });
      const resultBuffer = this.device.createBuffer({
        size: this.capacity * 8,
        usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
      });
      this.slots.push({
        querySet, resolveBuffer, resultBuffer,
        names: [], mapped: true, resolved: false,
      });
    }
  }

  /** True if the device supports timestamp queries (otherwise a no-op). */
  isSupported(): boolean { return this.supported; }

  beginFrame(): GpuFrameScope {
    if (!this.supported) return noopFrameScope;
    this.frameScopeCount = 0;
    const slot = this.slots[this.current];
    if (!slot) return noopFrameScope;
    slot.names.length = 0;
    slot.resolved = false;
    slot.mapped = false;
    const profiler = this;
    const frameScope: GpuFrameScope = {
      withPass<T extends GPURenderPassDescriptor | GPUComputePassDescriptor>(
        name: string,
        descriptor: T,
      ): T {
        if (profiler.frameScopeCount >= profiler.maxScopes) return descriptor;
        const slot = profiler.slots[profiler.current];
        if (!slot) return descriptor;
        const pair = profiler.frameScopeCount * 2;
        slot.names.push(name);
        profiler.frameScopeCount++;
        descriptor.timestampWrites = {
          querySet: slot.querySet,
          beginningOfPassWriteIndex: pair,
          endOfPassWriteIndex: pair + 1,
        } as GPURenderPassTimestampWrites;
        return descriptor;
      },
      scope(name: string, encoder: GPUCommandEncoder): GpuZone {
        if (profiler.frameScopeCount >= profiler.maxScopes) return { end() {} };
        const slot = profiler.slots[profiler.current];
        if (!slot) return { end() {} };
        const pair = profiler.frameScopeCount * 2;
        slot.names.push(name);
        profiler.frameScopeCount++;
        (encoder as unknown as { writeTimestamp?: (q: GPUQuerySet, i: number) => void }).writeTimestamp?.(slot.querySet, pair);
        return {
          end() {
            (encoder as unknown as { writeTimestamp?: (q: GPUQuerySet, i: number) => void }).writeTimestamp?.(slot.querySet, pair + 1);
          },
        };
      },
    };
    return frameScope;
  }

  /** Resolve the current frame's queries into its slot; call after encoder work. */
  endFrame(encoder: GPUCommandEncoder): void {
    if (!this.supported) return;
    const slot = this.slots[this.current];
    if (!slot) return;
    if (this.frameScopeCount === 0) { this.advance(); return; }
    const used = this.frameScopeCount * 2;
    encoder.resolveQuerySet(slot.querySet, 0, used, slot.resolveBuffer, 0);
    encoder.copyBufferToBuffer(slot.resolveBuffer, 0, slot.resultBuffer, 0, used);
    slot.resolved = true;
    slot.mapped = false;
    this.advance();
  }

  private advance(): void {
    this.current = (this.current + 1) % this.framesInFlight;
  }

  /**
   * Read back the oldest completed frame's scopes (empty until the pipeline
   * drains). Reuses a fixed result buffer — no steady-state allocation.
   */
  async poll(): Promise<readonly GpuScopeTiming[]> {
    this.result.length = 0;
    if (!this.supported) return this.result;
    const slot = this.slots[this.current];
    if (!slot || !slot.resolved || slot.mapped) return this.result;
    const used = slot.names.length * 2;
    await slot.resultBuffer.mapAsync(GPUMapMode.READ);
    slot.mapped = true;
    const view = new BigUint64Array(slot.resultBuffer.getMappedRange(0, used * 8));
    const period = this.period();
    for (let i = 0; i < slot.names.length; i++) {
      const start = view[i * 2] ?? 0n;
      const end = view[i * 2 + 1] ?? 0n;
      this.result.push({
        name: slot.names[i] ?? `scope-${i}`,
        startNs: start,
        endNs: end,
        durationMs: Number(end - start) * period / 1e6,
      });
    }
    slot.resultBuffer.unmap();
    slot.resolved = false;
    return this.result;
  }

  /**
   * Export scopes to Chrome-tracing (Catapult) JSON for `chrome://tracing`.
   * @alloc-effect diagnostic (builds a string — only when exporting).
   */
  exportChromeTrace(scopes: readonly GpuScopeTiming[]): string {
    const events = scopes.map((s) => ({
      name: s.name,
      cat: 'gpu',
      ph: 'X', // complete event (begin+end)
      ts: Number(s.startNs) / 1000, // ns → µs
      dur: (s.durationMs * 1000), // ms → µs
      pid: 1,
      tid: 1,
    }));
    return JSON.stringify({ traceEvents: events });
  }

  dispose(): void {
    for (const slot of this.slots) {
      slot.querySet.destroy();
      slot.resolveBuffer.destroy();
      slot.resultBuffer.destroy();
    }
    this.slots.length = 0;
  }
}

const noopFrameScope: GpuFrameScope = {
  withPass<T extends GPURenderPassDescriptor | GPUComputePassDescriptor>(name: string, d: T): T {
    return d;
  },
  scope(_name: string, _encoder: GPUCommandEncoder): GpuZone {
    return { end() {} };
  },
};
