// FrameBench — built-in rAF frame timing benchmark.
//
// Measures main-thread frame production rate via requestAnimationFrame
// timestamps (the same method Chrome DevTools' FPS counter uses). Collects
// per-frame intervals, then computes p50/p90/p99/max and counts dropped
// frames (below a configurable FPS threshold).
//
// Usage:
//   const bench = new FrameBench({ frames: 300, thresholdFps: 55 });
//   bench.start();                          // press 'B' or call manually
//   // ... rAF loop runs ...
//   // bench calls onDone(results) when finished
//
// Or auto-start via URL param: ?bench=300
//
// The benchmark is non-invasive: it hooks into the existing rAF loop by
// intercepting timestamps. It does NOT create its own rAF loop — the engine's
// render loop drives it.

export interface BenchResults {
  /** Number of frames sampled. */
  n: number;
  /** Average FPS across all sampled frames. */
  avgFps: number;
  /** p50 frame time in ms. */
  p50Ms: number;
  /** p90 frame time in ms. */
  p90Ms: number;
  /** p99 frame time in ms. */
  p99Ms: number;
  /** p99 as FPS (1000 / p99Ms). */
  p99Fps: number;
  /** Max frame time in ms. */
  maxMs: number;
  /** Max as FPS (1000 / maxMs). */
  maxFps: number;
  /** Number of frames below the threshold FPS. */
  belowThreshold: number;
  /** Configured threshold in FPS. */
  thresholdFps: number;
  /** Total elapsed time in ms. */
  totalMs: number;
}

export interface BenchOptions {
  /** Number of frames to sample (default 300 = ~5s at 60Hz). */
  frames?: number;
  /** FPS threshold for "dropped" frames (default 55). */
  thresholdFps?: number;
  /** Called when the benchmark completes with results. */
  onDone?: (results: BenchResults) => void;
  /** Called on each frame with progress (0..1). */
  onProgress?: (progress: number) => void;
}

/**
 * A frame timing benchmark that hooks into the engine's rAF loop.
 *
 * Call `tick(timestamp)` from the render loop each frame. When enough frames
 * are collected, `onDone` fires with the results. The benchmark is passive
 * until `start()` is called.
 */
export class FrameBench {
  private readonly frames: number;
  private readonly thresholdFps: number;
  private readonly onDone?: (results: BenchResults) => void;
  private readonly onProgress?: (progress: number) => void;

  private intervals: number[] = [];
  private prevTime = -1;
  private startTime = -1;
  private running = false;

  constructor(opts: BenchOptions = {}) {
    this.frames = opts.frames ?? 300;
    this.thresholdFps = opts.thresholdFps ?? 55;
    this.onDone = opts.onDone;
    this.onProgress = opts.onProgress;
  }

  /** Is the benchmark currently collecting frames? */
  get isRunning(): boolean {
    return this.running;
  }

  /** Start collecting frame timing data. Safe to call again if already running. */
  start(): void {
    this.intervals = [];
    this.prevTime = -1;
    this.startTime = -1;
    this.running = true;
  }

  /**
   * Called from the render loop each frame with the rAF timestamp.
   * Does nothing if the benchmark is not running.
   */
  tick(timestamp: number): void {
    if (!this.running) return;
    if (this.startTime < 0) this.startTime = timestamp;
    if (this.prevTime >= 0) {
      this.intervals.push(timestamp - this.prevTime);
      this.onProgress?.(this.intervals.length / this.frames);
    }
    this.prevTime = timestamp;
    if (this.intervals.length >= this.frames) {
      this.running = false;
      this.onDone?.(this.compute());
    }
  }

  /** Compute results from collected intervals. */
  private compute(): BenchResults {
    const sorted = [...this.intervals].sort((a, b) => a - b);
    const n = sorted.length;
    const sum = sorted.reduce((s, v) => s + v, 0);
    const avg = sum / n;
    const p50 = sorted[Math.floor(n * 0.5)];
    const p90 = sorted[Math.floor(n * 0.9)];
    const p99 = sorted[Math.floor(n * 0.99)];
    const max = sorted[n - 1];
    const thresholdMs = 1000 / this.thresholdFps;
    const belowThreshold = this.intervals.filter((v) => v > thresholdMs).length;
    const totalMs = this.prevTime - this.startTime;

    return {
      n,
      avgFps: 1000 / avg,
      p50Ms: p50,
      p90Ms: p90,
      p99Ms: p99,
      p99Fps: 1000 / p99,
      maxMs: max,
      maxFps: 1000 / max,
      belowThreshold,
      thresholdFps: this.thresholdFps,
      totalMs,
    };
  }
}

/**
 * Format BenchResults as a one-line HUD string.
 * Example: "p99=60.0fps max=60.0fps drops=0/300 (55fps threshold)"
 */
export function formatBenchResults(r: BenchResults): string {
  const pass = r.p99Fps >= r.thresholdFps;
  const icon = pass ? '✅' : '❌';
  return (
    `${icon} p99=${r.p99Fps.toFixed(1)}fps  max=${r.maxFps.toFixed(1)}fps  ` +
    `drops=${r.belowThreshold}/${r.n}  avg=${r.avgFps.toFixed(1)}fps  ` +
    `(${r.thresholdFps}fps threshold, ${r.totalMs.toFixed(0)}ms)`
  );
}

/**
 * Auto-start a benchmark from the URL `?bench=<frames>` parameter.
 * Returns a FrameBench if the param is set, else null.
 */
export function benchFromUrl(opts: Omit<BenchOptions, 'frames'> = {}): FrameBench | null {
  const params = new URLSearchParams(location.search);
  const benchParam = params.get('bench');
  if (!benchParam) return null;
  const frames = parseInt(benchParam, 10);
  if (!frames || frames < 10) return null;
  return new FrameBench({ ...opts, frames });
}
