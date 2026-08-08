import type { BigHeader } from './big-format.ts';
import {
  DeadlineRangeBatcher,
  type ContainerRangeReader,
  type PageLoadTier,
} from './deadline-range-batcher.ts';
import { BoundedTranscoderPool } from './bounded-transcoder-pool.ts';
import {
  hasSourceTextureTranscoder,
  type TextureTranscoder,
} from './service-types.ts';
import { VtPageDirectory } from './vt-page-directory.ts';
import { EngineTelemetryCategory } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

export interface PageProviderStats {
  reads: number;
  averageReadMs: number;
  maxReadMs: number;
  bulkQueued: number;
  bulkInFlight: number;
  bulkInFlightBytes: number;
  urgentBatches: number;
  focusBatches: number;
  peripheralBatches: number;
  bulkRejected: number;
  bulkCanceled: number;
  workerCount: number;
  activeTranscodes: number;
  queuedTranscodes: number;
  completedTranscodes: number;
  averageTranscodeQueueMs: number;
  maxTranscodeQueueMs: number;
  averageTranscodeMs: number;
  maxTranscodeMs: number;
}

export interface PagePipelineConfig {
  /** Waiting jobs only; active workers are separate. */
  transcodeQueueCapacity: number;
  urgentBatchDeadlineMs: number;
  focusBatchDeadlineMs: number;
  peripheralBatchDeadlineMs: number;
}

export type VirtualTexturePageProvider = ((
  path: string,
  req: { mip: number; x: number; y: number; tail?: boolean; batchTier?: PageLoadTier },
  signal?: AbortSignal,
) => Promise<Uint8Array>) & {
  getStats(): Readonly<PageProviderStats>;
  close(): void;
};

export function createPageDataProvider(
  loader: ContainerRangeReader,
  header: BigHeader,
  textureWorkers: readonly TextureTranscoder[],
  format: number,
  config: Readonly<PagePipelineConfig>,
  telemetry?: EngineTelemetry,
): VirtualTexturePageProvider {
  if (!Number.isInteger(config.transcodeQueueCapacity) || config.transcodeQueueCapacity < 1 ||
      !Number.isInteger(config.urgentBatchDeadlineMs) || config.urgentBatchDeadlineMs < 0 ||
      !Number.isInteger(config.focusBatchDeadlineMs) || config.focusBatchDeadlineMs < 0 ||
      !Number.isInteger(config.peripheralBatchDeadlineMs) || config.peripheralBatchDeadlineMs < 0 ||
      config.urgentBatchDeadlineMs > config.focusBatchDeadlineMs ||
      config.focusBatchDeadlineMs > config.peripheralBatchDeadlineMs) {
    throw new RangeError('invalid VT page-pipeline configuration');
  }
  const directory = new VtPageDirectory(header);
  const transcoder = new BoundedTranscoderPool(textureWorkers, config.transcodeQueueCapacity, telemetry);
  const sourceBackedWorkers = textureWorkers.every(hasSourceTextureTranscoder);
  const bulkReads = new DeadlineRangeBatcher(
    loader,
    config.urgentBatchDeadlineMs,
    config.focusBatchDeadlineMs,
    config.peripheralBatchDeadlineMs,
    telemetry,
  );
  const stats: PageProviderStats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0,
    bulkQueued: 0, bulkInFlight: 0, bulkInFlightBytes: 0,
    urgentBatches: 0, focusBatches: 0, peripheralBatches: 0,
    bulkRejected: 0, bulkCanceled: 0,
    workerCount: textureWorkers.length, activeTranscodes: 0, queuedTranscodes: 0,
    completedTranscodes: 0, averageTranscodeQueueMs: 0, maxTranscodeQueueMs: 0,
    averageTranscodeMs: 0, maxTranscodeMs: 0,
  };

  const provider = (async (
    path: string,
    req: { mip: number; x: number; y: number; tail?: boolean; batchTier?: PageLoadTier },
    signal?: AbortSignal,
  ) => {
    if (signal?.aborted) throw new Error('VT page load canceled before read');
    // The store's owned request record may carry its generated runtime path as
    // an extra field. The explicit provider path is authoritative.
    const page = directory.resolve({ ...req, path });
    const correlation = (req as { cacheKey?: number }).cacheKey ??
      telemetry?.nextCorrelation(EngineTelemetryCategory.VirtualTexture) ?? 0;

    let transcoded: Uint8Array;
    if (page.encoding !== 'RawRgba8' && sourceBackedWorkers) {
      // Native workers own the confined source and consume only this fixed
      // descriptor. Encoded Basis bytes never enter V8.
      transcoded = await transcoder.submitSourceRange(
        page.offset, page.length, format, signal, correlation,
      );
    } else {
      const pageData = await bulkReads.read(
        page.offset,
        page.length,
        req.batchTier ?? 'urgent',
        signal,
        correlation,
      );
      if (signal?.aborted) throw new Error('VT page load canceled after read');

      if (page.encoding === 'RawRgba8') {
        if (format !== 4) {
          throw new Error(`VT page ${path} is raw RGBA8 but GPU format ${format} requires Basis encoding`);
        }
        return pageData;
      }

      // The worker returns [count][width][height][length][data]...; a VT page
      // consumes only the first image payload, never the serialization header.
      if (pageData.byteLength < 2 || pageData[0] !== 0x73 || pageData[1] !== 0x42)
        throw new Error(`invalid Basis page range for ${path}: bytes=${pageData.byteLength}, magic=${pageData[0]},${pageData[1]}`);
      // Each SPSC worker permits one in-flight transcode. The fixed pool dispatches
      // independently without an unbounded Promise chain and drops canceled jobs
      // before they reach a worker.
      transcoded = await transcoder.submit(pageData, format, signal, correlation);
    }
    if (signal?.aborted) throw new Error('VT page load canceled after transcode');
    if (transcoded.byteLength < 16) throw new Error('truncated transcoded VT page');
    const view = new DataView(transcoded.buffer, transcoded.byteOffset, transcoded.byteLength);
    const count = view.getUint32(0, true);
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const length = view.getUint32(12, true);
    if (count < 1 || width !== 136 || height !== 136 || 16 + length > transcoded.byteLength)
      throw new Error(`invalid transcoded VT page header: count=${count}, size=${width}x${height}, bytes=${length}`);
    return transcoded.slice(16, 16 + length);
  }) as VirtualTexturePageProvider;
  provider.close = () => bulkReads.close();
  provider.getStats = () => {
    const transcode = transcoder.getStats();
    const read = bulkReads.getStats();
    stats.reads = read.reads;
    stats.averageReadMs = read.averageReadMs;
    stats.maxReadMs = read.maxReadMs;
    stats.bulkQueued = read.queued;
    stats.bulkInFlight = read.inFlight;
    stats.bulkInFlightBytes = read.inFlightBytes;
    stats.urgentBatches = read.urgentBatches;
    stats.focusBatches = read.focusBatches;
    stats.peripheralBatches = read.peripheralBatches;
    stats.bulkRejected = read.rejected;
    stats.bulkCanceled = read.canceled;
    stats.workerCount = transcode.workerCount;
    stats.activeTranscodes = transcode.active;
    stats.queuedTranscodes = transcode.queued;
    stats.completedTranscodes = transcode.completed;
    stats.averageTranscodeQueueMs = transcode.averageQueueMs;
    stats.maxTranscodeQueueMs = transcode.maxQueueMs;
    stats.averageTranscodeMs = transcode.averageTranscodeMs;
    stats.maxTranscodeMs = transcode.maxTranscodeMs;
    return stats;
  };
  return provider;
}
