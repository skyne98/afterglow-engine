import {
  createPageDataProvider,
  type FetchRangeLoader,
  type VirtualTexturePageProvider,
} from './big-parser.ts';
import { BigContainer } from './big-container.ts';
import { createPlatformRangeLoader } from './platform-range-loader.ts';
import {
  createPlatformMeshOptimizer,
  createPlatformTextureTranscoder,
  platformTextureWorkerCount,
} from './platform-workers.ts';
import { AssetStore } from './asset-store.ts';
import { OwnedWorkerPool } from './owned-worker-pool.ts';
import {
  VirtualTextureStore, VirtualTextureTuning, type VirtualTextureRuntimeCapacities,
} from '../virtual-texturing/virtual-texture.ts';
import { EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import type { OwnedMeshOptimizer, OwnedTextureTranscoder } from './service-types.ts';

export type { OwnedMeshOptimizer, OwnedTextureTranscoder } from './service-types.ts';

export interface EngineAssetsOptions {
  containerPath: string;
  format: number;
  /** Test/profile override. Production selects the bounded platform worker profile. */
  workerCount?: number;
  transcodeQueueCapacity: number;
  urgentBatchDeadlineMs: number;
  focusBatchDeadlineMs: number;
  peripheralBatchDeadlineMs: number;
  maxPendingPages: number;
  maxPendingBytes: number;
  maxHeaderBytes: number;
  /** Test/platform injection; games use engine-owned platform services. */
  createTranscoder?(index: number): Promise<OwnedTextureTranscoder>;
  createMeshOptimizer?(): Promise<OwnedMeshOptimizer>;
  source?: FetchRangeLoader;
  telemetry?: EngineTelemetry;
}

/** Public asset-system owner.
 *
 * `BigContainer` owns immutable format/index state, `OwnedWorkerPool` owns
 * service lifetime, and the VT store owns residency policy. This class only
 * composes those public primitives and enforces reverse shutdown. */
export class EngineAssets {
  readonly pageProvider: VirtualTexturePageProvider;
  readonly stats = { workersStarted: 0, closeErrors: 0, closed: false };

  private closed = false;
  private assetStore: AssetStore | null = null;
  private meshOptimizer: OwnedMeshOptimizer | null = null;
  private store: VirtualTextureStore | null = null;

  private constructor(
    readonly container: BigContainer,
    readonly format: number,
    private readonly textureWorkers: OwnedWorkerPool<OwnedTextureTranscoder>,
    private readonly createMeshOptimizer: (() => Promise<OwnedMeshOptimizer>) | undefined,
    private readonly telemetry: EngineTelemetry | undefined,
    private readonly vtCapacities: Readonly<VirtualTextureRuntimeCapacities>,
    pageProvider: VirtualTexturePageProvider,
  ) {
    this.pageProvider = pageProvider;
    this.stats.workersStarted = textureWorkers.size;
  }

  get source(): FetchRangeLoader { return this.container.source; }
  get containerPath(): string { return this.container.path; }
  get header() { return this.container.header; }
  get rawAssets() { return this.container.rawAssets; }

  static async open(options: EngineAssetsOptions): Promise<EngineAssets> {
    const workerCount = validateOptions(options);
    const correlation = options.telemetry?.nextCorrelation(EngineTelemetryCategory.Asset) ?? 0;
    options.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.SessionOpen, correlation, workerCount, 0,
    );
    const source = options.source ?? createPlatformRangeLoader('', options.telemetry);
    let workerPool: OwnedWorkerPool<OwnedTextureTranscoder> | null = null;
    try {
      const container = await BigContainer.open(
        source, options.containerPath, options.maxHeaderBytes,
      );
      workerPool = await OwnedWorkerPool.start(
        workerCount,
        index => options.createTranscoder
          ? options.createTranscoder(index)
          : createPlatformTextureTranscoder(
              index, container.path, options.telemetry,
            ),
      );
      const containerLoader = {
        load: (path: string): Promise<Uint8Array> => source.load(path),
        size: (path: string): Promise<number> => source.size(path),
        read: (_path: string, offset: number, length: number): Promise<Uint8Array> =>
          source.read(container.path, offset, length),
        readBulk: source.readBulk
          ? (ranges: Parameters<NonNullable<FetchRangeLoader['readBulk']>>[1]): Promise<Uint8Array[]> =>
              source.readBulk!(container.path, ranges)
          : undefined,
      };
      const pageProvider = createPageDataProvider(
        containerLoader,
        container.header,
        workerPool.workers,
        options.format,
        {
          transcodeQueueCapacity: options.transcodeQueueCapacity,
          urgentBatchDeadlineMs: options.urgentBatchDeadlineMs,
          focusBatchDeadlineMs: options.focusBatchDeadlineMs,
          peripheralBatchDeadlineMs: options.peripheralBatchDeadlineMs,
        },
        options.telemetry,
      );
      const assets = new EngineAssets(
        container, options.format, workerPool,
        options.createMeshOptimizer, options.telemetry,
        { maxPendingPages: options.maxPendingPages, maxPendingBytes: options.maxPendingBytes },
        pageProvider,
      );
      options.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.SessionOpen, correlation, workerPool.size, 0,
      );
      return assets;
    } catch (error) {
      try { await workerPool?.close(); }
      catch (closeError) {
        if (error instanceof Error && error.cause === undefined) error.cause = closeError;
      }
      options.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.SessionOpen, correlation, workerPool?.size ?? 0, 1,
      );
      throw error;
    }
  }

  async createAssetStore(
    capacity = 64,
    maxCompletionsPerPoll = 32,
  ): Promise<AssetStore> {
    if (this.closed) throw new Error('cannot create an asset store from closed EngineAssets');
    if (this.assetStore || this.meshOptimizer)
      throw new Error('EngineAssets already created its asset store');
    const optimizer = this.createMeshOptimizer
      ? await this.createMeshOptimizer()
      : await createPlatformMeshOptimizer(this.telemetry);
    if (this.closed) {
      await optimizer.close();
      throw new Error('EngineAssets closed while creating its asset store');
    }
    try {
      this.assetStore = new AssetStore(
        this.rawAssets, optimizer, capacity, maxCompletionsPerPoll, this.telemetry,
      );
      this.meshOptimizer = optimizer;
      this.stats.workersStarted++;
      return this.assetStore;
    } catch (error) {
      try { await optimizer.close(); }
      catch (closeError) {
        if (error instanceof Error && error.cause === undefined) error.cause = closeError;
      }
      throw error;
    }
  }

  createVirtualTextureStore(
    device?: GPUDevice,
    tuning?: VirtualTextureTuning,
  ): VirtualTextureStore {
    if (this.closed) throw new Error('cannot create a VT store from closed EngineAssets');
    if (this.store) throw new Error('EngineAssets already created its VT store');
    const loader = {
      read: (_path: string, offset: number, length: number): Promise<Uint8Array> =>
        this.source.read(this.containerPath, offset, length),
      poll(): void {},
    };
    this.store = new VirtualTextureStore(
      loader,
      this.vtCapacities,
      this.pageProvider,
      this.format,
      device,
      tuning ?? new VirtualTextureTuning(),
      this.telemetry,
    );
    return this.store;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.assetStore?.dispose();
    this.assetStore = null;
    this.store?.dispose();
    this.pageProvider.close();
    this.store = null;
    let firstError: unknown = null;
    if (this.meshOptimizer) {
      try { await this.meshOptimizer.close(); }
      catch (error) { this.stats.closeErrors++; firstError = error; }
      this.meshOptimizer = null;
    }
    try { await this.textureWorkers.close(); }
    catch (error) { if (firstError === null) firstError = error; }
    this.stats.closeErrors += this.textureWorkers.stats.closeErrors;
    this.stats.closed = true;
    if (firstError !== null) throw firstError;
  }
}

function validateOptions(options: EngineAssetsOptions): number {
  if (!options.containerPath) throw new RangeError('EngineAssets requires a container path');
  if (options.workerCount !== undefined &&
      (!Number.isInteger(options.workerCount) || options.workerCount <= 0))
    throw new RangeError('EngineAssets workerCount must be positive');
  if (!Number.isInteger(options.transcodeQueueCapacity) || options.transcodeQueueCapacity <= 0)
    throw new RangeError('EngineAssets transcode queue capacity must be positive');
  if (!Number.isInteger(options.urgentBatchDeadlineMs) || options.urgentBatchDeadlineMs < 0 ||
      !Number.isInteger(options.focusBatchDeadlineMs) || options.focusBatchDeadlineMs < 0 ||
      !Number.isInteger(options.peripheralBatchDeadlineMs) || options.peripheralBatchDeadlineMs < 0 ||
      options.urgentBatchDeadlineMs > options.focusBatchDeadlineMs ||
      options.focusBatchDeadlineMs > options.peripheralBatchDeadlineMs)
    throw new RangeError('EngineAssets bulk deadlines are invalid');
  if (!Number.isInteger(options.maxPendingPages) || options.maxPendingPages <= 0 ||
      !Number.isInteger(options.maxPendingBytes) || options.maxPendingBytes <= 0)
    throw new RangeError('EngineAssets VT pending capacities must be positive integers');
  const workerCount = options.workerCount ??
    platformTextureWorkerCount(options.maxPendingPages);
  if (options.transcodeQueueCapacity + workerCount < options.maxPendingPages)
    throw new RangeError(
      'EngineAssets transcode capacity must cover every admitted VT page',
    );
  return workerCount;
}
