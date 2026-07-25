import { defineResource, type Resource } from '../core/resource.ts';

export const TELEMETRY_RECORD_BYTES = 40;
export const TELEMETRY_BATCH_HEADER_BYTES = 40;
export const TELEMETRY_BATCH_VERSION = 1;
const TELEMETRY_RECORD_WORDS = TELEMETRY_RECORD_BYTES / 4;
const U32_SCALE = 0x1_0000_0000;
const U32_MAX = 0xffff_ffff;
export const TELEMETRY_HISTOGRAM_BUCKETS = 32;

export const enum TelemetryDescriptorKind {
  Instant = 1,
  Span = 2,
  AsyncSpan = 3,
  Flow = 4,
}

export const enum TelemetryPhase {
  Instant = 1,
  SpanBegin = 2,
  SpanEnd = 3,
  AsyncBegin = 4,
  AsyncEnd = 5,
  FlowStart = 6,
  FlowStep = 7,
  FlowEnd = 8,
}

export const enum TelemetryCaptureState {
  Idle = 0,
  Armed = 1,
  Frozen = 2,
}

export const enum TelemetryRecordStatus {
  Recorded = 0,
  Disabled = 1,
  CategoryDisabled = 2,
  InvalidDescriptor = 3,
  WrongDescriptorKind = 4,
  CapacityExceeded = 5,
}

export interface TelemetryDescriptor {
  readonly category: number;
  readonly categoryName: string;
  readonly name: string;
  readonly kind: TelemetryDescriptorKind;
  readonly argument0?: string;
  readonly argument1?: string;
}

export interface TelemetrySnapshot {
  epoch: number;
  count: number;
  capacity: number;
  dropped: number;
  ticksPerSecond: number;
  buffer: ArrayBuffer;
}

export type TelemetryClock = () => number;

/**
 * Producer-local finite trace recorder using the Rust crate's exact 40-byte
 * little-endian record ABI. All arrays and the stable snapshot are allocated
 * during construction.
 */
export class TelemetryRecorder {
  private readonly words: Uint32Array;
  private readonly bytes: Uint8Array;
  private readonly enabledDescriptors: Uint8Array;
  private readonly stableSnapshot: TelemetrySnapshot;
  private captureState = TelemetryCaptureState.Idle;
  private captureEpoch = 0;
  private length = 0;
  private droppedRecords = 0;

  constructor(
    readonly descriptors: readonly TelemetryDescriptor[],
    readonly buffer: ArrayBuffer,
    private readonly clock: TelemetryClock = () => performance.now() * 1_000,
    readonly ticksPerSecond = 1_000_000,
  ) {
    if (buffer.byteLength === 0 || buffer.byteLength % TELEMETRY_RECORD_BYTES !== 0)
      throw new RangeError('telemetry trace buffer must contain a positive whole number of 40-byte records');
    if (!Number.isFinite(ticksPerSecond) || ticksPerSecond <= 0)
      throw new RangeError('telemetry ticksPerSecond must be positive');
    this.words = new Uint32Array(buffer);
    this.bytes = new Uint8Array(buffer);
    this.enabledDescriptors = new Uint8Array(descriptors.length);
    this.stableSnapshot = {
      epoch: 0,
      count: 0,
      capacity: buffer.byteLength / TELEMETRY_RECORD_BYTES,
      dropped: 0,
      ticksPerSecond,
      buffer,
    };
  }

  get state(): TelemetryCaptureState { return this.captureState; }
  get count(): number { return this.length; }
  get capacity(): number { return this.stableSnapshot.capacity; }
  get dropped(): number { return this.droppedRecords; }

  arm(epoch: number, categoryWords?: Uint32Array): boolean {
    if (this.captureState !== TelemetryCaptureState.Idle ||
        !Number.isInteger(epoch) || epoch < 0 || epoch > U32_MAX)
      return false;
    this.captureEpoch = epoch;
    this.length = 0;
    this.droppedRecords = 0;
    for (let index = 0; index < this.descriptors.length; index++) {
      const category = this.descriptors[index]?.category ?? -1;
      this.enabledDescriptors[index] = categoryWords === undefined
        ? 1
        : ((categoryWords[category >>> 5] ?? 0) & (1 << (category & 31))) !== 0 ? 1 : 0;
    }
    this.captureState = TelemetryCaptureState.Armed;
    return true;
  }

  stop(): boolean {
    if (this.captureState !== TelemetryCaptureState.Armed) return false;
    this.captureState = TelemetryCaptureState.Frozen;
    return true;
  }

  snapshot(): TelemetrySnapshot | null {
    if (this.captureState !== TelemetryCaptureState.Frozen) return null;
    this.stableSnapshot.epoch = this.captureEpoch;
    this.stableSnapshot.count = this.length;
    this.stableSnapshot.dropped = this.droppedRecords;
    return this.stableSnapshot;
  }

  reset(): boolean {
    if (this.captureState !== TelemetryCaptureState.Frozen) return false;
    this.length = 0;
    this.droppedRecords = 0;
    this.enabledDescriptors.fill(0);
    this.captureState = TelemetryCaptureState.Idle;
    return true;
  }

  /** @alloc-effect none */
  encodedBatchBytes(): number {
    return TELEMETRY_BATCH_HEADER_BYTES + this.length * TELEMETRY_RECORD_BYTES;
  }

  /**
   * Encode the Rust crate's `AGTB` v1 batch into caller-owned storage. Returns
   * bytes written, or `-needed` when `output` is too small.
   * @alloc-effect none
   */
  encodeBatchInto(output: Uint8Array, sourceId: number, clockDomain: number): number {
    if (this.captureState !== TelemetryCaptureState.Frozen ||
        !Number.isInteger(sourceId) || sourceId < 0 || sourceId > U32_MAX ||
        !Number.isInteger(clockDomain) || clockDomain < 0 || clockDomain > U32_MAX) return 0;
    const needed = this.encodedBatchBytes();
    if (output.length < needed) return -needed;
    output[0] = 0x41; output[1] = 0x47; output[2] = 0x54; output[3] = 0x42;
    this.writeU16(output, 4, TELEMETRY_BATCH_VERSION);
    this.writeU16(output, 6, TELEMETRY_BATCH_HEADER_BYTES);
    this.writeU32(output, 8, sourceId);
    this.writeU32(output, 12, this.captureEpoch);
    this.writeU32(output, 16, clockDomain);
    this.writeU32(output, 20, 0);
    this.writeU32(output, 24, this.length);
    this.writeU32(output, 28, Math.min(U32_MAX, this.droppedRecords));
    this.writeU64Number(output, 32, this.ticksPerSecond);
    const payloadBytes = this.length * TELEMETRY_RECORD_BYTES;
    for (let index = 0; index < payloadBytes; index++)
      output[TELEMETRY_BATCH_HEADER_BYTES + index] = this.bytes[index] ?? 0;
    return needed;
  }

  // @hot-no-alloc-begin TelemetryRecorder.instant
  instant(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.Instant, TelemetryPhase.Instant, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.instant

  // @hot-no-alloc-begin TelemetryRecorder.spanBegin
  spanBegin(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.Span, TelemetryPhase.SpanBegin, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.spanBegin

  // @hot-no-alloc-begin TelemetryRecorder.spanEnd
  spanEnd(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.Span, TelemetryPhase.SpanEnd, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.spanEnd

  // @hot-no-alloc-begin TelemetryRecorder.asyncBegin
  asyncBegin(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.AsyncSpan, TelemetryPhase.AsyncBegin, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.asyncBegin

  // @hot-no-alloc-begin TelemetryRecorder.asyncEnd
  asyncEnd(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.AsyncSpan, TelemetryPhase.AsyncEnd, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.asyncEnd

  // @hot-no-alloc-begin TelemetryRecorder.flowStart
  flowStart(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.Flow, TelemetryPhase.FlowStart, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.flowStart

  // @hot-no-alloc-begin TelemetryRecorder.flowStep
  flowStep(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.Flow, TelemetryPhase.FlowStep, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.flowStep

  // @hot-no-alloc-begin TelemetryRecorder.flowEnd
  flowEnd(descriptor: number, correlation: number, argument0 = 0, argument1 = 0): TelemetryRecordStatus {
    return this.record(descriptor, TelemetryDescriptorKind.Flow, TelemetryPhase.FlowEnd, correlation, argument0, argument1);
  }
  // @hot-no-alloc-end TelemetryRecorder.flowEnd

  /** @alloc-effect none */
  private record(
    descriptor: number,
    expectedKind: TelemetryDescriptorKind,
    phase: TelemetryPhase,
    correlation: number,
    argument0: number,
    argument1: number,
  ): TelemetryRecordStatus {
    if (this.captureState !== TelemetryCaptureState.Armed) return TelemetryRecordStatus.Disabled;
    if (!Number.isInteger(descriptor) || descriptor < 0 || descriptor >= this.descriptors.length)
      return TelemetryRecordStatus.InvalidDescriptor;
    if (this.descriptors[descriptor]?.kind !== expectedKind)
      return TelemetryRecordStatus.WrongDescriptorKind;
    if (this.enabledDescriptors[descriptor] === 0)
      return TelemetryRecordStatus.CategoryDisabled;
    if (this.length === this.capacity) {
      this.droppedRecords++;
      return TelemetryRecordStatus.CapacityExceeded;
    }
    const base = this.length * TELEMETRY_RECORD_WORDS;
    this.writeU53(base, this.clock());
    this.writeU53(base + 2, correlation);
    this.writeU53(base + 4, argument0);
    this.writeU53(base + 6, argument1);
    this.words[base + 8] = descriptor;
    this.words[base + 9] = phase;
    this.length++;
    return TelemetryRecordStatus.Recorded;
  }

  /** @alloc-effect none */
  private writeU53(word: number, value: number): void {
    const nonNegative = Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
    this.words[word] = nonNegative >>> 0;
    this.words[word + 1] = Math.floor(nonNegative / U32_SCALE) >>> 0;
  }

  /** @alloc-effect none */
  private writeU16(output: Uint8Array, offset: number, value: number): void {
    output[offset] = value;
    output[offset + 1] = value >>> 8;
  }

  /** @alloc-effect none */
  private writeU32(output: Uint8Array, offset: number, value: number): void {
    output[offset] = value;
    output[offset + 1] = value >>> 8;
    output[offset + 2] = value >>> 16;
    output[offset + 3] = value >>> 24;
  }

  /** @alloc-effect none */
  private writeU64Number(output: Uint8Array, offset: number, value: number): void {
    const nonNegative = Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
    this.writeU32(output, offset, nonNegative);
    this.writeU32(output, offset + 4, Math.floor(nonNegative / U32_SCALE));
  }
}

export const enum TelemetryMetricKind {
  Counter = 1,
  Gauge = 2,
  Maximum = 3,
  HistogramLog2 = 4,
}

export const enum TelemetryMetricStatus {
  Updated = 0,
  InvalidMetric = 1,
  WrongMetricKind = 2,
}

export interface TelemetryMetricDescriptor {
  readonly category: number;
  readonly categoryName: string;
  readonly name: string;
  readonly kind: TelemetryMetricKind;
  readonly unit?: string;
}

/** Fixed producer-local metric cells. No update allocates or grows storage. */
export class TelemetryMetricBank {
  private readonly offsets: Uint32Array;
  readonly requiredCells: number;

  constructor(
    readonly descriptors: readonly TelemetryMetricDescriptor[],
    readonly cells: Float64Array,
  ) {
    this.offsets = new Uint32Array(descriptors.length);
    let required = 0;
    for (let index = 0; index < descriptors.length; index++) {
      this.offsets[index] = required;
      required += descriptors[index]?.kind === TelemetryMetricKind.HistogramLog2
        ? TELEMETRY_HISTOGRAM_BUCKETS : 1;
    }
    if (cells.length < required)
      throw new RangeError(`telemetry metrics require ${required} cells, received ${cells.length}`);
    this.requiredCells = required;
  }

  // @hot-no-alloc-begin TelemetryMetricBank.counterAdd
  counterAdd(metric: number, delta: number): TelemetryMetricStatus {
    const offset = this.scalarOffset(metric, TelemetryMetricKind.Counter);
    if (offset < 0) return this.metricFailure(metric, TelemetryMetricKind.Counter);
    this.cells[offset] = (this.cells[offset] ?? 0) + delta;
    return TelemetryMetricStatus.Updated;
  }
  // @hot-no-alloc-end TelemetryMetricBank.counterAdd

  // @hot-no-alloc-begin TelemetryMetricBank.gaugeSet
  gaugeSet(metric: number, value: number): TelemetryMetricStatus {
    const offset = this.scalarOffset(metric, TelemetryMetricKind.Gauge);
    if (offset < 0) return this.metricFailure(metric, TelemetryMetricKind.Gauge);
    this.cells[offset] = value;
    return TelemetryMetricStatus.Updated;
  }
  // @hot-no-alloc-end TelemetryMetricBank.gaugeSet

  // @hot-no-alloc-begin TelemetryMetricBank.maximum
  maximum(metric: number, value: number): TelemetryMetricStatus {
    const offset = this.scalarOffset(metric, TelemetryMetricKind.Maximum);
    if (offset < 0) return this.metricFailure(metric, TelemetryMetricKind.Maximum);
    if (value > (this.cells[offset] ?? 0)) this.cells[offset] = value;
    return TelemetryMetricStatus.Updated;
  }
  // @hot-no-alloc-end TelemetryMetricBank.maximum

  // @hot-no-alloc-begin TelemetryMetricBank.histogramLog2
  histogramLog2(metric: number, value: number): TelemetryMetricStatus {
    if (!Number.isInteger(metric) || metric < 0 || metric >= this.descriptors.length)
      return TelemetryMetricStatus.InvalidMetric;
    if (this.descriptors[metric]?.kind !== TelemetryMetricKind.HistogramLog2)
      return TelemetryMetricStatus.WrongMetricKind;
    const bucket = value <= 0 ? 0 : Math.min(TELEMETRY_HISTOGRAM_BUCKETS - 1, Math.floor(Math.log2(value)));
    const offset = (this.offsets[metric] ?? 0) + bucket;
    this.cells[offset] = (this.cells[offset] ?? 0) + 1;
    return TelemetryMetricStatus.Updated;
  }
  // @hot-no-alloc-end TelemetryMetricBank.histogramLog2

  /** @alloc-effect none */
  readCell(metric: number, bucket = 0): number {
    if (!Number.isInteger(metric) || metric < 0 || metric >= this.descriptors.length) return 0;
    const descriptor = this.descriptors[metric];
    const count = descriptor?.kind === TelemetryMetricKind.HistogramLog2
      ? TELEMETRY_HISTOGRAM_BUCKETS : 1;
    if (!Number.isInteger(bucket) || bucket < 0 || bucket >= count) return 0;
    return this.cells[(this.offsets[metric] ?? 0) + bucket] ?? 0;
  }

  /** @alloc-effect none */
  private scalarOffset(metric: number, expected: TelemetryMetricKind): number {
    if (!Number.isInteger(metric) || metric < 0 || metric >= this.descriptors.length) return -1;
    return this.descriptors[metric]?.kind === expected ? (this.offsets[metric] ?? -1) : -1;
  }

  /** @alloc-effect none */
  private metricFailure(metric: number, expected: TelemetryMetricKind): TelemetryMetricStatus {
    if (!Number.isInteger(metric) || metric < 0 || metric >= this.descriptors.length)
      return TelemetryMetricStatus.InvalidMetric;
    return this.descriptors[metric]?.kind === expected
      ? TelemetryMetricStatus.InvalidMetric : TelemetryMetricStatus.WrongMetricKind;
  }
}

/** One engine-facing facade over the trace and metrics planes. */
export class EngineTelemetry {
  readonly trace: TelemetryRecorder;
  readonly metrics: TelemetryMetricBank;
  private correlationCounter = 1;

  constructor(
    traceDescriptors: readonly TelemetryDescriptor[],
    metricDescriptors: readonly TelemetryMetricDescriptor[],
    traceBuffer: ArrayBuffer,
    metricCells: Float64Array,
    clock?: TelemetryClock,
  ) {
    this.trace = new TelemetryRecorder(traceDescriptors, traceBuffer, clock);
    this.metrics = new TelemetryMetricBank(metricDescriptors, metricCells);
  }

  // @hot-no-alloc-begin EngineTelemetry.nextCorrelation
  nextCorrelation(namespace = 0): number {
    const safeNamespace = Number.isInteger(namespace) && namespace >= 0
      ? Math.min(0x0f_ffff, namespace) : 0;
    const local = this.correlationCounter;
    this.correlationCounter = local === U32_MAX ? 1 : local + 1;
    return safeNamespace * U32_SCALE + local;
  }
  // @hot-no-alloc-end EngineTelemetry.nextCorrelation
}

export const TelemetryRes: Resource<EngineTelemetry> = defineResource<EngineTelemetry>('telemetry', () => {
  throw new Error('Telemetry not initialized. Set TelemetryRes during bootstrap.');
});
