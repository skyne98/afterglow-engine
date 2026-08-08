import type { FetchRangeLoader } from './asset-range.ts';
import { getVirtualTextureDimensions } from './big-format.ts';
import {
  createPageDataProvider,
  type VirtualTexturePageProvider,
} from './vt-page-provider.ts';
import { BigContainer } from './big-container.ts';
import { createPlatformRangeLoader } from './platform-range-loader.ts';
import {
  createPlatformMeshOptimizer,
  createPlatformTextureTranscoder,
  platformTextureWorkerCount,
} from './platform-workers.ts';
import { AssetStore } from './asset-store.ts';
import { OwnedWorkerPool } from './owned-worker-pool.ts';
import { EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import type { OwnedMeshOptimizer, OwnedTextureTranscoder } from './service-types.ts';
import { ModelSystem, type ModelSystemOptions } from '../presentation/model-system.ts';
import {
  VirtualTextureSystem,
  type VirtualTextureHandle,
  type VirtualTextureStorageFormat,
  type VirtualTextureSystemOptions,
} from '../virtual-texturing/virtual-texture-system.ts';

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
 * service lifetime, and `VirtualTextureSystem` owns residency policy. This class only
 * composes those public primitives and enforces reverse shutdown. */
export class EngineAssets {
  readonly pageProvider: VirtualTexturePageProvider;
  readonly stats = { servicesStarted: 0, closeErrors: 0, closed: false };

  private closed = false;
  private assetStore: AssetStore | null = null;
  private meshOptimizer: OwnedMeshOptimizer | null = null;
  private modelSystem: ModelSystem | null = null;
  private textureSystem: VirtualTextureSystem | null = null;

  private constructor(
    readonly container: BigContainer,
    readonly format: number,
    private readonly textureWorkers: OwnedWorkerPool<OwnedTextureTranscoder>,
    private readonly createMeshOptimizer: (() => Promise<OwnedMeshOptimizer>) | undefined,
    private readonly telemetry: EngineTelemetry | undefined,
    pageProvider: VirtualTexturePageProvider,
  ) {
    this.pageProvider = pageProvider;
    this.stats.servicesStarted = textureWorkers.size;
  }

  static async open(options: EngineAssetsOptions): Promise<EngineAssets> {
    const workerCount = validateOptions(options);
    const correlation = options.telemetry?.nextCorrelation(EngineTelemetryCategory.Asset) ?? 0;
    options.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.AssetCompositionOpen, correlation, workerCount, 0,
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
      const pageProvider = createPageDataProvider(
        container.ranges,
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
        options.createMeshOptimizer, options.telemetry, pageProvider,
      );
      options.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.AssetCompositionOpen, correlation, workerPool.size, 0,
      );
      return assets;
    } catch (error) {
      try { await workerPool?.close(); }
      catch (closeError) {
        if (error instanceof Error && error.cause === undefined) error.cause = closeError;
      }
      options.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.AssetCompositionOpen, correlation, workerPool?.size ?? 0, 1,
      );
      throw error;
    }
  }

  private async ensureMeshOptimizer(): Promise<OwnedMeshOptimizer> {
    if (this.meshOptimizer) return this.meshOptimizer;
    const optimizer = this.createMeshOptimizer
      ? await this.createMeshOptimizer()
      : await createPlatformMeshOptimizer(this.telemetry);
    if (this.closed) {
      await optimizer.close();
      throw new Error('EngineAssets closed while creating its mesh processor');
    }
    this.meshOptimizer = optimizer;
    this.stats.servicesStarted++;
    return optimizer;
  }

  async createAssetStore(
    capacity = 64,
    maxCompletionsPerPoll = 32,
  ): Promise<AssetStore> {
    if (this.closed) throw new Error('cannot create an asset store from closed EngineAssets');
    if (this.assetStore) throw new Error('EngineAssets already created its asset store');
    const optimizer = await this.ensureMeshOptimizer();
    this.assetStore = new AssetStore(
      this.container.rawAssets, optimizer, capacity, maxCompletionsPerPoll, this.telemetry,
    );
    return this.assetStore;
  }

  async createModelSystem(
    options: Readonly<ModelSystemOptions>,
  ): Promise<ModelSystem> {
    if (this.closed) throw new Error('cannot create a model system from closed EngineAssets');
    if (this.modelSystem) throw new Error('EngineAssets already created its model system');
    const optimizer = await this.ensureMeshOptimizer();
    this.modelSystem = new ModelSystem(optimizer, options, this.telemetry);
    return this.modelSystem;
  }

  createVirtualTextureSystem(
    options: Readonly<VirtualTextureSystemOptions>,
  ): VirtualTextureSystem {
    if (this.closed) throw new Error('cannot create a texture system from closed EngineAssets');
    if (this.textureSystem)
      throw new Error('EngineAssets already created its texture system');
    const telemetry = options.telemetry ?? this.telemetry;
    this.textureSystem = new VirtualTextureSystem(
      telemetry ? { ...options, telemetry } : options,
    );
    return this.textureSystem;
  }

  registerVirtualTexture(
    sourceKey: string,
    format: VirtualTextureStorageFormat,
    addressMode: 'clamp' | 'repeat' | 'mirror-repeat',
    mipTail = true,
  ): VirtualTextureHandle | 0 {
    if (!this.textureSystem) throw new Error('EngineAssets has no virtual texture system');
    const dimensions = getVirtualTextureDimensions(this.container.header, sourceKey);
    return this.textureSystem.createTexture(
      { ...dimensions, format, addressMode, mipTail },
      this.pageProvider,
      sourceKey,
    );
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.assetStore?.dispose();
    this.assetStore = null;
    this.modelSystem?.dispose();
    this.modelSystem = null;
    this.textureSystem?.dispose();
    this.textureSystem = null;
    this.pageProvider.close();
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
