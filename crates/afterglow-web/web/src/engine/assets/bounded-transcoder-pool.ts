import type { TextureTranscoder } from './service-types.ts';
import { EngineMetric, EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

interface TranscodeJob {
  data: Uint8Array | null;
  offset: number;
  length: number;
  format: number;
  correlation: number;
  signal: AbortSignal | undefined;
  queuedAt: number;
  resolve(value: Uint8Array): void;
  reject(error: unknown): void;
}

/** Fixed-capacity dispatcher over independent one-in-flight SPSC workers. */
export class BoundedTranscoderPool {
  private readonly jobs: (TranscodeJob | null)[];
  private readonly workerBusy: Uint8Array;
  private head = 0;
  private tail = 0;
  private count = 0;
  private active = 0;
  private completed = 0;
  private totalQueueMs = 0;
  private maxQueueMs = 0;
  private totalTranscodeMs = 0;
  private maxTranscodeMs = 0;
  private readonly stats = {
    workerCount: 0, active: 0, queued: 0, completed: 0,
    averageQueueMs: 0, maxQueueMs: 0,
    averageTranscodeMs: 0, maxTranscodeMs: 0,
  };

  constructor(
    private readonly workers: readonly TextureTranscoder[],
    capacity: number,
    private readonly telemetry?: EngineTelemetry,
  ) {
    if (workers.length === 0 || !Number.isInteger(capacity) || capacity < 1)
      throw new RangeError('VT transcoder pool requires workers and positive capacity');
    this.jobs = new Array(capacity).fill(null);
    this.workerBusy = new Uint8Array(workers.length);
  }

  submit(
    data: Uint8Array,
    format: number,
    signal?: AbortSignal,
    correlation = 0,
  ): Promise<Uint8Array> {
    return this.enqueue(data, 0, data.byteLength, format, signal, correlation);
  }

  submitSourceRange(
    offset: number,
    length: number,
    format: number,
    signal?: AbortSignal,
    correlation = 0,
  ): Promise<Uint8Array> {
    return this.enqueue(null, offset, length, format, signal, correlation);
  }

  private enqueue(
    data: Uint8Array | null,
    offset: number,
    length: number,
    format: number,
    signal: AbortSignal | undefined,
    correlation: number,
  ): Promise<Uint8Array> {
    if (this.count === this.jobs.length) return Promise.reject(new Error('VT transcode queue capacity exceeded'));
    const traceCorrelation = correlation || this.telemetry?.nextCorrelation(EngineTelemetryCategory.Texture) || 0;
    this.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.TextureTranscodeQueue, traceCorrelation, length, format,
    );
    return new Promise((resolve, reject) => {
      this.jobs[this.tail] = {
        data, offset, length, format, correlation: traceCorrelation,
        signal, queuedAt: performance.now(), resolve, reject,
      };
      this.tail = (this.tail + 1) % this.jobs.length;
      this.count++;
      this.pump();
    });
  }

  private pump(): void {
    for (let workerIndex = 0; workerIndex < this.workers.length && this.count !== 0; workerIndex++) {
      if (this.workerBusy[workerIndex] !== 0) continue;
      const job = this.jobs[this.head]!;
      this.jobs[this.head] = null;
      this.head = (this.head + 1) % this.jobs.length;
      this.count--;
      if (job.signal?.aborted) {
        this.telemetry?.trace.asyncEnd(
          EngineTraceDescriptor.TextureTranscodeQueue, job.correlation, 0, job.format,
        );
        job.reject(new Error('VT transcode canceled before dispatch'));
        workerIndex--;
        continue;
      }
      const queueMs = performance.now() - job.queuedAt;
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.TextureTranscodeQueue, job.correlation,
        job.length, job.format,
      );
      this.totalQueueMs += queueMs;
      this.maxQueueMs = Math.max(this.maxQueueMs, queueMs);
      this.workerBusy[workerIndex] = 1;
      this.active++;
      void this.run(workerIndex, job);
    }
  }

  private async run(workerIndex: number, job: TranscodeJob): Promise<void> {
    const startedAt = performance.now();
    let status = 1;
    let outputBytes = 0;
    this.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.TextureTranscode, job.correlation, job.length, job.format,
    );
    try {
      const worker = this.workers[workerIndex]!;
      const result = job.data === null
        ? await worker.transcodeSourceRange?.(job.offset, job.length, job.format)
        : await worker.transcode(job.data, job.format);
      if (result === undefined)
        throw new Error('texture worker does not support source-backed transcoding');
      outputBytes = result.byteLength;
      status = 0;
      if (job.signal?.aborted) job.reject(new Error('VT transcode canceled after dispatch'));
      // Public-web transports expose reusable wasm scratch and must copy before
      // the next call. Native op responses transfer independent Vec ownership
      // into V8 and can move directly to the upload queue.
      else job.resolve(worker.responseIsOwned ? result : result.slice());
    } catch (error) {
      job.reject(error);
    } finally {
      const elapsed = performance.now() - startedAt;
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.TextureTranscode, job.correlation, outputBytes, status,
      );
      this.telemetry?.metrics.histogramLog2(
        EngineMetric.TextureTranscodeNs, Math.max(1, Math.floor(elapsed * 1_000_000)),
      );
      this.completed++;
      this.totalTranscodeMs += elapsed;
      this.maxTranscodeMs = Math.max(this.maxTranscodeMs, elapsed);
      this.workerBusy[workerIndex] = 0;
      this.active--;
      this.pump();
    }
  }

  getStats() {
    const stats = this.stats;
    stats.workerCount = this.workers.length;
    stats.active = this.active;
    stats.queued = this.count;
    stats.completed = this.completed;
    stats.averageQueueMs = this.completed === 0 ? 0 : this.totalQueueMs / this.completed;
    stats.maxQueueMs = this.maxQueueMs;
    stats.averageTranscodeMs = this.completed === 0 ? 0 : this.totalTranscodeMs / this.completed;
    stats.maxTranscodeMs = this.maxTranscodeMs;
    return stats;
  }
}
