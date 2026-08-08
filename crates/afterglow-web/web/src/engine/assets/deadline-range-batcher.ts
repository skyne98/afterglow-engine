import {
  BULK_IN_FLIGHT_MAX_BYTES,
  BULK_RANGE_CAPACITY,
  BULK_RESPONSE_MAX_BYTES,
  estimatedBulkResponseBytes,
  type AssetByteRange,
} from './bulk-range.ts';
import { EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

export type PageLoadTier = 'urgent' | 'focus' | 'peripheral';

export interface ContainerRangeReader {
  read(offset: number, len: number): Promise<Uint8Array>;
  readBulk?: ((ranges: readonly AssetByteRange[]) => Promise<Uint8Array[]>) | undefined;
}

interface BulkReadSlot {
  offset: number;
  correlation: number;
  length: number;
  signal: AbortSignal | undefined;
  resolve: ((bytes: Uint8Array) => void) | null;
  reject: ((error: unknown) => void) | null;
}

/** Fixed-capacity three-deadline raw-byte queue. Timers are opened by the first
 * miss and never reset, so continuous arrivals cannot postpone a lane's ready
 * deadline. Dispatch still gives ready urgent work strict priority; sustained
 * urgent demand can defer the focus and peripheral lanes. */
export class DeadlineRangeBatcher {
  private readonly slots: BulkReadSlot[] = new Array(BULK_RANGE_CAPACITY);
  private readonly free = new Uint16Array(BULK_RANGE_CAPACITY);
  private freeTop = 0;
  private readonly queued = [
    new Uint16Array(BULK_RANGE_CAPACITY),
    new Uint16Array(BULK_RANGE_CAPACITY),
    new Uint16Array(BULK_RANGE_CAPACITY),
  ];
  private readonly heads = new Uint16Array(3);
  private readonly tails = new Uint16Array(3);
  private readonly counts = new Uint16Array(3);
  private readonly ready = new Uint8Array(3);
  private readonly timers: Array<ReturnType<typeof setTimeout> | null> = [null, null, null];
  private inFlight = 0;
  private inFlightBytes = 0;
  private closed = false;
  private reads = 0;
  private totalReadMs = 0;
  private maxReadMs = 0;
  private urgentBatches = 0;
  private focusBatches = 0;
  private peripheralBatches = 0;
  private rejected = 0;
  private canceled = 0;
  private readonly stats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0, queued: 0,
    inFlight: 0, inFlightBytes: 0,
    urgentBatches: 0, focusBatches: 0, peripheralBatches: 0,
    rejected: 0, canceled: 0,
  };

  constructor(
    private readonly loader: ContainerRangeReader,
    private readonly urgentDeadlineMs: number,
    private readonly focusDeadlineMs: number,
    private readonly peripheralDeadlineMs: number,
    private readonly telemetry?: EngineTelemetry,
  ) {
    for (let index = BULK_RANGE_CAPACITY - 1; index >= 0; index--) {
      this.slots[index] = {
        offset: 0, correlation: 0, length: 0, signal: undefined, resolve: null, reject: null,
      };
      this.free[this.freeTop++] = index;
    }
  }

  private tierIndex(tier: PageLoadTier): number {
    return tier === 'urgent' ? 0 : tier === 'focus' ? 1 : 2;
  }
  private deadlineMs(tier: number): number {
    return tier === 0 ? this.urgentDeadlineMs :
      tier === 1 ? this.focusDeadlineMs : this.peripheralDeadlineMs;
  }

  read(
    offset: number,
    length: number,
    tier: PageLoadTier,
    signal?: AbortSignal,
    correlation = 0,
  ): Promise<Uint8Array> {
    const traceCorrelation = correlation || this.telemetry?.nextCorrelation(EngineTelemetryCategory.VirtualTexture) || 0;
    return new Promise<Uint8Array>((resolve, reject) => {
      if (this.closed) { this.rejected++; reject(new Error('bulk page reader is closed')); return; }
      if (signal?.aborted) {
        this.canceled++;
        reject(new Error('VT page load canceled before batching'));
        return;
      }
      if (this.freeTop === 0) {
        this.rejected++;
        reject(new Error('bulk page queue capacity exceeded'));
        return;
      }
      const slotIndex = this.free[--this.freeTop];
      const slot = this.slots[slotIndex];
      slot.offset = offset;
      slot.correlation = traceCorrelation;
      slot.length = length;
      this.telemetry?.trace.asyncBegin(
        EngineTraceDescriptor.VtBulkWait, traceCorrelation, length, this.tierIndex(tier),
      );
      slot.signal = signal;
      slot.resolve = resolve;
      slot.reject = reject;
      const lane = this.tierIndex(tier);
      this.queued[lane][this.tails[lane]] = slotIndex;
      this.tails[lane] = (this.tails[lane] + 1) % BULK_RANGE_CAPACITY;
      this.counts[lane]++;
      if (this.timers[lane] === null) {
        this.timers[lane] = setTimeout(() => {
          this.timers[lane] = null;
          this.ready[lane] = 1;
          this.pump();
        }, this.deadlineMs(lane));
      }
      if (this.counts[lane] === BULK_RANGE_CAPACITY) {
        this.ready[lane] = 1;
        this.pump();
      }
    });
  }

  private release(slotIndex: number): void {
    const slot = this.slots[slotIndex];
    slot.signal = undefined;
    slot.resolve = null;
    slot.reject = null;
    this.free[this.freeTop++] = slotIndex;
  }

  private pop(lane: number): number {
    const index = this.queued[lane][this.heads[lane]];
    this.heads[lane] = (this.heads[lane] + 1) % BULK_RANGE_CAPACITY;
    this.counts[lane]--;
    return index;
  }

  private clearLaneTimer(lane: number): void {
    const timer = this.timers[lane];
    if (timer !== null) clearTimeout(timer);
    this.timers[lane] = null;
  }

  private pump(): void {
    while (this.inFlight < 2 && this.inFlightBytes < BULK_IN_FLIGHT_MAX_BYTES) {
      const lane = this.ready[0] !== 0 && this.counts[0] !== 0
        ? 0
        : this.ready[1] !== 0 && this.counts[1] !== 0
          ? 1
          : this.ready[2] !== 0 && this.counts[2] !== 0 ? 2 : -1;
      if (lane < 0) return;
      const indices: number[] = [];
      const ranges: AssetByteRange[] = [];
      while (this.counts[lane] !== 0 && indices.length < BULK_RANGE_CAPACITY) {
        const slotIndex = this.queued[lane][this.heads[lane]];
        const slot = this.slots[slotIndex];
        if (slot.signal?.aborted) {
          this.pop(lane);
          this.canceled++;
          this.telemetry?.trace.asyncEnd(
            EngineTraceDescriptor.VtBulkWait, slot.correlation, 0, lane,
          );
          slot.reject?.(new Error('VT page load canceled while batched'));
          this.release(slotIndex);
          continue;
        }
        const candidate = { offset: slot.offset, length: slot.length };
        ranges.push(candidate);
        if (estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES) {
          ranges.pop();
          if (indices.length === 0) {
            this.pop(lane);
            this.rejected++;
            this.telemetry?.trace.asyncEnd(
              EngineTraceDescriptor.VtBulkWait, slot.correlation, 0, lane,
            );
            slot.reject?.(new RangeError('one VT page exceeds bulk response capacity'));
            this.release(slotIndex);
            continue;
          }
          break;
        }
        indices.push(this.pop(lane));
      }
      if (this.counts[lane] === 0) {
        this.ready[lane] = 0;
        this.clearLaneTimer(lane);
      }
      if (indices.length === 0) continue;
      const expectedBytes = estimatedBulkResponseBytes(ranges);
      if (this.inFlightBytes + expectedBytes > BULK_IN_FLIGHT_MAX_BYTES) return;
      this.dispatch(indices, ranges, expectedBytes, lane);
    }
  }

  private dispatch(
    indices: number[],
    ranges: AssetByteRange[],
    expectedBytes: number,
    lane: number,
  ): void {
    this.inFlight++;
    this.inFlightBytes += expectedBytes;
    if (lane === 0) this.urgentBatches++;
    else if (lane === 1) this.focusBatches++;
    else this.peripheralBatches++;
    const startedAt = performance.now();
    const batchCorrelation = this.telemetry?.nextCorrelation(EngineTelemetryCategory.Asset) ?? 0;
    for (let index = 0; index < indices.length; index++) {
      const slot = this.slots[indices[index]];
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.VtBulkWait, slot.correlation, slot.length, lane,
      );
    }
    this.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.VtBulkDispatch, batchCorrelation, expectedBytes, indices.length,
    );
    const request = this.loader.readBulk
      ? this.loader.readBulk(ranges)
      : Promise.all(indices.map((slotIndex, index) => {
          const slot = this.slots[slotIndex];
          const range = ranges[index];
          return this.loader.read(range.offset, range.length);
        }));
    request.then(parts => {
      if (parts.length !== indices.length)
        throw new Error(`bulk response returned ${parts.length} parts; expected ${indices.length}`);
      const readMs = performance.now() - startedAt;
      let receivedBytes = 0;
      for (let index = 0; index < parts.length; index++)
        receivedBytes += parts[index]?.byteLength ?? 0;
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.VtBulkDispatch, batchCorrelation, receivedBytes, parts.length,
      );
      this.reads++;
      this.totalReadMs += readMs;
      this.maxReadMs = Math.max(this.maxReadMs, readMs);
      for (let index = 0; index < indices.length; index++) {
        const slotIndex = indices[index];
        const slot = this.slots[slotIndex];
        const bytes = parts[index];
        if (bytes.byteLength !== slot.length)
          slot.reject?.(new Error(`bulk page returned ${bytes.byteLength} bytes; expected ${slot.length}`));
        else if (this.closed || slot.signal?.aborted) {
          this.canceled++;
          slot.reject?.(new Error('VT page load canceled after bulk read'));
        } else slot.resolve?.(bytes);
        this.release(slotIndex);
      }
    }).catch(error => {
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.VtBulkDispatch, batchCorrelation, 0, 0,
      );
      for (const slotIndex of indices) {
        this.slots[slotIndex].reject?.(error);
        this.release(slotIndex);
      }
    }).finally(() => {
      this.inFlight--;
      this.inFlightBytes -= expectedBytes;
      this.pump();
    });
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (let lane = 0; lane < 3; lane++) {
      this.clearLaneTimer(lane);
      while (this.counts[lane] !== 0) {
        const slotIndex = this.pop(lane);
        const slot = this.slots[slotIndex];
        this.canceled++;
        this.telemetry?.trace.asyncEnd(
          EngineTraceDescriptor.VtBulkWait, slot.correlation, 0, lane,
        );
        slot.reject?.(new Error('bulk page reader closed'));
        this.release(slotIndex);
      }
      this.ready[lane] = 0;
    }
  }

  getStats(): Readonly<{
    reads: number;
    averageReadMs: number;
    maxReadMs: number;
    queued: number;
    inFlight: number;
    inFlightBytes: number;
    urgentBatches: number;
    focusBatches: number;
    peripheralBatches: number;
    rejected: number;
    canceled: number;
  }> {
    const stats = this.stats;
    stats.reads = this.reads;
    stats.averageReadMs = this.reads === 0 ? 0 : this.totalReadMs / this.reads;
    stats.maxReadMs = this.maxReadMs;
    stats.queued = this.counts[0] + this.counts[1] + this.counts[2];
    stats.inFlight = this.inFlight;
    stats.inFlightBytes = this.inFlightBytes;
    stats.urgentBatches = this.urgentBatches;
    stats.focusBatches = this.focusBatches;
    stats.peripheralBatches = this.peripheralBatches;
    stats.rejected = this.rejected;
    stats.canceled = this.canceled;
    return stats;
  }
}

