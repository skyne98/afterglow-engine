// Central profiling ECS resource — gathers engine-wide performance data.
//
// For now, gathers from Three.js (the only renderer in the engine):
//   - renderer.info — per-frame count stats (draw calls, triangles, textures,
//     memory sizes, compute calls).
//   - backend.trackTimestamp + renderer.resolveTimestampsAsync — per-type GPU
//     pass timings (render / compute) via Three's own timestamp-query pool.
//     Three auto-injects timestampWrites into its own passes, so this is the
//     authoritative GPU time for Three-owned passes (no engine surgery needed).
//
// The resource owns a fixed-capacity ring of frame samples (allocation-free at
// steady state) and a chrome-trace (Catapult) JSON exporter. It is the single
// place that touches Three's private timestamp backend; other subsystems (e.g.
// the VT feedback coordinator's GPU-timing path) should delegate here instead
// of re-casting the same Three internals.
//
// Future: engine-owned passes (custom compute) will feed the same ring via the
// standalone GpuProfiler (see renderer/gpu-profiler.ts), and a unified
// chrome-trace timeline will merge both sources. Layer-3 "why" (SOL / stall
// reasons) stays in vendor tools (RGP for the 680M, Nsight for the workstation)
// — see docs/research/engine-gpu-profiling.md.

import { defineResource, type Resource } from '../core/resource.ts';

/** Minimal Three renderer surface the profiler reads. */
export interface ProfilingRenderer {
  readonly info: {
    readonly render: { readonly calls: number; readonly drawCalls: number; readonly triangles: number; readonly points: number; readonly lines: number };
    readonly compute: { readonly calls: number };
    readonly memory: { readonly textures: number; readonly texturesSize: number; readonly uniformBuffers: number; readonly uniformBuffersSize: number; readonly geometries: number };
    reset(): void;
  };
}

/** A single frame's gathered profiling data. */
export interface ProfilingFrame {
  frameId: number;
  /** CPU wall delta for the frame (ms), if a deltaSource is attached. */
  deltaMs: number;
  // renderer.info counts (snapshot at frame end).
  renderCalls: number;
  drawCalls: number;
  triangles: number;
  points: number;
  lines: number;
  computeCalls: number;
  textures: number;
  texturesSize: number;
  uniformBuffers: number;
  uniformBuffersSize: number;
  geometries: number;
  // GPU pass durations (ms). 0 when timestamp tracking is disabled or the
  // device lacks `timestamp-query`. Quantized by the browser when the dev flag
  // is off (Chrome: 100µs floor; CEF 149 observed full resolution on the 680M).
  gpuRenderMs: number;
  gpuComputeMs: number;
}

export interface ProfilingOptions {
  /** Ring capacity (frames of history). Default 240 (~4s at 60fps). ≥ 1. */
  readonly capacity?: number;
}

/** Profiling host: the renderer + a frame-delta source. */
export interface ProfilingHost {
  readonly renderer: ProfilingRenderer;
  /** Optional: returns the frame delta in ms (e.g. from the runtime frame). */
  readonly deltaSource?: () => number;
}

const DEFAULT_CAPACITY = 240;

/**
 * Central profiling gatherer. One instance per world (ECS resource).
 *
 * Lifecycle per frame: `setEnabled(true)` once at bootstrap to enable GPU
 * timing, then `gather(frameId)` after the frame's render work. GPU results
 * lag by Three's internal query-pool depth; `gather` awaits the oldest
 * available, so call it once per frame (off the hot path, e.g. in the
 * diagnostics cadence).
 *
 * @alloc-effect none at steady state (fixed ring reused). `exportChromeTrace`
 * allocates (diagnostic only).
 */
export class Profiling {
  private readonly ring: ProfilingFrame[];
  private head = 0;       // next write index
  private count = 0;     // number of valid samples (≤ capacity)
  private enabled = false;
  private readonly capacity: number;
  private readonly deltaSource: () => number;

  constructor(private readonly host: ProfilingHost, options: ProfilingOptions = {}) {
    if (options.capacity !== undefined && (!Number.isInteger(options.capacity) || options.capacity < 1))
      throw new RangeError('Profiling capacity must be a positive integer');
    this.capacity = options.capacity ?? DEFAULT_CAPACITY;
    this.ring = new Array<ProfilingFrame>(this.capacity);
    for (let i = 0; i < this.capacity; i++) this.ring[i] = this.blank();
    this.deltaSource = host.deltaSource ?? (() => 0);
  }

  private blank(): ProfilingFrame {
    return {
      frameId: 0, deltaMs: 0, renderCalls: 0, drawCalls: 0, triangles: 0,
      points: 0, lines: 0, computeCalls: 0, textures: 0, texturesSize: 0,
      uniformBuffers: 0, uniformBuffersSize: 0, geometries: 0,
      gpuRenderMs: 0, gpuComputeMs: 0,
    };
  }

  /** Enable/disable Three's GPU timestamp tracking (bootstrap-only). */
  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    const backend = (this.host.renderer as unknown as {
      backend?: { trackTimestamp?: boolean; timestampQueryPool?: Record<string, { trackTimestamp?: boolean } | undefined> };
    }).backend;
    if (!backend) return;
    backend.trackTimestamp = enabled;
    for (const pool of Object.values(backend.timestampQueryPool ?? {})) if (pool) pool.trackTimestamp = enabled;
  }

  isGpuTimingEnabled(): boolean { return this.enabled; }

  /**
   * Gather one frame's data into the ring. Snapshots renderer.info counts and
   * resolves the oldest available GPU timestamps. Returns the gathered frame.
   */
  async gather(frameId: number): Promise<ProfilingFrame> {
    const info = this.host.renderer.info;
    const f = this.ring[this.head];
    if (!f) return this.blank();
    f.frameId = frameId;
    f.deltaMs = this.deltaSource();
    f.renderCalls = info.render.calls;
    f.drawCalls = info.render.drawCalls;
    f.triangles = info.render.triangles;
    f.points = info.render.points;
    f.lines = info.render.lines;
    f.computeCalls = info.compute.calls;
    f.textures = info.memory.textures;
    f.texturesSize = info.memory.texturesSize;
    f.uniformBuffers = info.memory.uniformBuffers;
    f.uniformBuffersSize = info.memory.uniformBuffersSize;
    f.geometries = info.memory.geometries;
    f.gpuRenderMs = this.enabled ? await this.resolveType('render') : 0;
    f.gpuComputeMs = this.enabled ? await this.resolveType('compute') : 0;
    this.head = (this.head + 1) % this.capacity;
    if (this.count < this.capacity) this.count++;
    return f;
  }

  /** Resolve a Three GPU timestamp type to a duration in ms (0 if unavailable). */
  private async resolveType(type: string): Promise<number> {
    const renderer = this.host.renderer as unknown as {
      resolveTimestampsAsync?(t: string): Promise<number>;
    };
    try {
      return (await renderer.resolveTimestampsAsync?.(type)) ?? 0;
    } catch {
      return 0;
    }
  }

  /** Number of valid samples currently in the ring. */
  sampleCount(): number { return this.count; }

  /**
   * Copy the latest `n` (≤ sampleCount) frames into `out` (oldest first).
   * Reuses the provided array; returns the count written.
   * @alloc-effect diagnostic (`out` defaults to a newly allocated array).
   */
  latest(n: number, out: ProfilingFrame[] = []): number {
    const take = Math.min(n, this.count);
    const start = (this.head - take + this.capacity) % this.capacity;
    for (let i = 0; i < take; i++) out[i] = this.ring[(start + i) % this.capacity] ?? this.blank();
    out.length = take;
    return take;
  }

  /**
   * Export gathered frames to Chrome-tracing (Catapult) JSON for
   * `chrome://tracing`. Each frame becomes a GPU-render + GPU-compute event.
   * @alloc-effect diagnostic (builds a string — only when exporting).
   */
  exportChromeTrace(): string {
    const frames: ProfilingFrame[] = [];
    this.latest(this.count, frames);
    let ts = 0;
    const events: object[] = [];
    for (const f of frames) {
      if (f.gpuRenderMs > 0) events.push({ name: 'gpu.render', cat: 'gpu', ph: 'X', ts, dur: f.gpuRenderMs * 1000, pid: 1, tid: 1, args: { frameId: f.frameId, drawCalls: f.drawCalls, triangles: f.triangles } });
      if (f.gpuComputeMs > 0) events.push({ name: 'gpu.compute', cat: 'gpu', ph: 'X', ts, dur: f.gpuComputeMs * 1000, pid: 1, tid: 2, args: { frameId: f.frameId } });
      ts += f.deltaMs * 1000;
    }
    return JSON.stringify({ traceEvents: events });
  }
}

/**
 * ECS resource handle for the central Profiling instance.
 * Set explicitly after RendererHost creation:
 *   ProfilingRes.set(world, new Profiling(host));
 */
export const ProfilingRes: Resource<Profiling> = defineResource<Profiling>('profiling', () => {
  throw new Error('Profiling not initialized. Call ProfilingRes.set(world, new Profiling(host)).');
});
