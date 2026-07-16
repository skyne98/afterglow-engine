// Fixed-capacity diagnostic rAF timing capture. Formatting remains a diagnostic
// slow path; tick() only writes preallocated numeric storage.

export interface BenchResults {
  n: number;
  avgFps: number;
  p50Ms: number;
  p90Ms: number;
  p99Ms: number;
  p99Fps: number;
  maxMs: number;
  maxFps: number;
  belowThreshold: number;
  thresholdFps: number;
  totalMs: number;
}

export const enum BenchStartStatus {
  Started = 0,
  InvalidSampleCount = 1,
}

export interface BenchOptions {
  /** Fixed storage capacity reserved at construction (default 300). */
  capacity?: number;
  /** FPS threshold for a dropped frame (default 55). */
  thresholdFps?: number;
  /** Receives one stable result object owned by FrameBench. */
  onDone?: (results: Readonly<BenchResults>) => void;
  onProgress?: (completed: number, total: number) => void;
}

export class FrameBench {
  private readonly intervals: Float64Array;
  private readonly sorted: Float64Array;
  private readonly thresholdFps: number;
  private readonly onDone: ((results: Readonly<BenchResults>) => void) | undefined;
  private readonly onProgress: ((completed: number, total: number) => void) | undefined;
  private readonly result: BenchResults = {
    n: 0,
    avgFps: 0,
    p50Ms: 0,
    p90Ms: 0,
    p99Ms: 0,
    p99Fps: 0,
    maxMs: 0,
    maxFps: 0,
    belowThreshold: 0,
    thresholdFps: 0,
    totalMs: 0,
  };
  private sampleTarget = 0;
  private sampleCount = 0;
  private previousTime = -1;
  private startTime = -1;
  private running = false;
  private pendingResults = false;

  constructor(options: BenchOptions = {}) {
    const capacity = options.capacity ?? 300;
    if (!Number.isInteger(capacity) || capacity < 10)
      throw new RangeError('frame benchmark capacity must be an integer of at least 10');
    const threshold = options.thresholdFps ?? 55;
    if (!Number.isFinite(threshold) || threshold <= 0)
      throw new RangeError('frame benchmark threshold must be positive');
    this.intervals = new Float64Array(capacity);
    this.sorted = new Float64Array(capacity);
    this.thresholdFps = threshold;
    this.onDone = options.onDone;
    this.onProgress = options.onProgress;
    this.result.thresholdFps = threshold;
  }

  get capacity(): number { return this.intervals.length; }
  get isRunning(): boolean { return this.running; }
  get completedSamples(): number { return this.sampleCount; }
  get hasPendingResults(): boolean { return this.pendingResults; }
  get results(): Readonly<BenchResults> { return this.result; }

  start(sampleCount = this.capacity): BenchStartStatus {
    if (!Number.isInteger(sampleCount) || sampleCount < 10 || sampleCount > this.capacity)
      return BenchStartStatus.InvalidSampleCount;
    this.sampleTarget = sampleCount;
    this.sampleCount = 0;
    this.previousTime = -1;
    this.startTime = -1;
    this.running = true;
    this.pendingResults = false;
    return BenchStartStatus.Started;
  }

  // @hot-no-alloc-begin FrameBench.tick
  tick(timestamp: number): void {
    if (!this.running) return;
    if (this.startTime < 0) this.startTime = timestamp;
    if (this.previousTime >= 0) {
      this.intervals[this.sampleCount++] = timestamp - this.previousTime;
      this.onProgress?.(this.sampleCount, this.sampleTarget);
    }
    this.previousTime = timestamp;
    if (this.sampleCount >= this.sampleTarget) {
      this.running = false;
      this.pendingResults = true;
    }
  }
  // @hot-no-alloc-end FrameBench.tick

  /** Diagnostic completion work over fixed storage; invoke from a slow path. */
  finish(): Readonly<BenchResults> | null {
    if (!this.pendingResults) return null;
    this.pendingResults = false;
    this.compute();
    this.onDone?.(this.result);
    return this.result;
  }

  private compute(): void {
    let sum = 0;
    let belowThreshold = 0;
    const thresholdMs = 1000 / this.thresholdFps;
    for (let index = 0; index < this.capacity; index++) {
      const value = index < this.sampleCount ? (this.intervals[index] ?? 0) : Number.POSITIVE_INFINITY;
      this.sorted[index] = value;
      if (index < this.sampleCount) {
        sum += value;
        if (value > thresholdMs) belowThreshold++;
      }
    }
    this.sorted.sort();
    const n = this.sampleCount;
    const average = sum / n;
    const p50 = this.sorted[Math.floor(n * 0.5)] ?? 0;
    const p90 = this.sorted[Math.floor(n * 0.9)] ?? 0;
    const p99 = this.sorted[Math.floor(n * 0.99)] ?? 0;
    const max = this.sorted[n - 1] ?? 0;
    this.result.n = n;
    this.result.avgFps = 1000 / average;
    this.result.p50Ms = p50;
    this.result.p90Ms = p90;
    this.result.p99Ms = p99;
    this.result.p99Fps = 1000 / p99;
    this.result.maxMs = max;
    this.result.maxFps = 1000 / max;
    this.result.belowThreshold = belowThreshold;
    this.result.totalMs = this.previousTime - this.startTime;
  }
}

export function formatBenchResults(result: Readonly<BenchResults>): string {
  const icon = result.p99Fps >= result.thresholdFps ? '✅' : '❌';
  return (
    `${icon} p99=${result.p99Fps.toFixed(1)}fps  max=${result.maxFps.toFixed(1)}fps  ` +
    `drops=${result.belowThreshold}/${result.n}  avg=${result.avgFps.toFixed(1)}fps  ` +
    `(${result.thresholdFps}fps threshold, ${result.totalMs.toFixed(0)}ms)`
  );
}

export function benchFromUrl(options: Omit<BenchOptions, 'capacity'> = {}): FrameBench | null {
  const value = new URLSearchParams(location.search).get('bench');
  if (!value) return null;
  const frames = Number.parseInt(value, 10);
  if (!Number.isInteger(frames) || frames < 10) return null;
  const bench = new FrameBench({ ...options, capacity: frames });
  bench.start(frames);
  return bench;
}
