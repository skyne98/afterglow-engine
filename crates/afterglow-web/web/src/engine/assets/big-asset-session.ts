import {
  BigContainerAssetLoader,
  createPageDataProvider,
  readBigHeader,
  type BigHeader,
  type FetchRangeLoader,
  type VirtualTexturePageProvider,
} from './big-parser.ts';
import { createPlatformRangeLoader } from './platform-range-loader.ts';
import { createPlatformMeshOptimizer, createPlatformTextureTranscoder } from './platform-workers.ts';
import { AssetStore, type MeshOptimizer } from './asset-store.ts';
import type { PersistentBlobCache } from './persistent-blob-cache.ts';
import { VirtualTextureStore, VirtualTextureTuning } from '../virtual-texturing/virtual-texture.ts';
import { EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

export interface TextureTranscoder {
  transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
}

export interface OwnedTextureTranscoder extends TextureTranscoder {
  close(): void | Promise<void>;
}

export interface OwnedMeshOptimizer extends MeshOptimizer {
  close(): void | Promise<void>;
}

export interface BigAssetSessionOptions {
  containerPath: string;
  format: number;
  workerCount: number;
  transcodeQueueCapacity: number;
  maxHeaderBytes: number;
  /** Test/platform injection; games use the engine-owned default workers. */
  createTranscoder?(index: number): Promise<OwnedTextureTranscoder>;
  createMeshOptimizer?(): Promise<OwnedMeshOptimizer>;
  source?: FetchRangeLoader;
  cache?: PersistentBlobCache;
  telemetry?: EngineTelemetry;
}

/** Bootstrap owner for one BIG source, its header, workers, and page provider. */
export class BigAssetSession {
  readonly rawAssets: BigContainerAssetLoader;
  readonly pageProvider: VirtualTexturePageProvider;
  readonly stats = { workersStarted: 0, closeErrors: 0, closed: false };

  private closed = false;
  private assetStore: AssetStore | null = null;
  private meshOptimizer: OwnedMeshOptimizer | null = null;
  private store: VirtualTextureStore | null = null;

  private constructor(
    readonly source: FetchRangeLoader,
    readonly containerPath: string,
    readonly header: BigHeader,
    readonly format: number,
    private readonly workers: OwnedTextureTranscoder[],
    private readonly createMeshOptimizer: (() => Promise<OwnedMeshOptimizer>) | undefined,
    private readonly telemetry: EngineTelemetry | undefined,
    pageProvider: VirtualTexturePageProvider,
  ) {
    this.rawAssets = new BigContainerAssetLoader(source, containerPath, header);
    this.pageProvider = pageProvider;
    this.stats.workersStarted = workers.length;
  }

  static async open(options: BigAssetSessionOptions): Promise<BigAssetSession> {
    if (!options.containerPath) throw new RangeError('BIG session requires a container path');
    if (!Number.isInteger(options.workerCount) || options.workerCount <= 0)
      throw new RangeError('BIG session workerCount must be positive');
    if (!Number.isInteger(options.transcodeQueueCapacity) || options.transcodeQueueCapacity <= 0)
      throw new RangeError('BIG session transcode queue capacity must be positive');
    const correlation = options.telemetry?.nextCorrelation(EngineTelemetryCategory.Asset) ?? 0;
    options.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.SessionOpen, correlation, options.workerCount, 0,
    );
    const source = options.source ?? createPlatformRangeLoader('', options.telemetry);
    const workers: OwnedTextureTranscoder[] = [];
    try {
      const header = await readBigHeader(source, options.containerPath, options.maxHeaderBytes);

      for (let index = 0; index < options.workerCount; index++)
        workers.push(options.createTranscoder
          ? await options.createTranscoder(index)
          : await createPlatformTextureTranscoder(index, options.telemetry));
      const clients = workers;
      const containerLoader = {
        load: (path: string): Promise<Uint8Array> => source.load(path),
        size: (path: string): Promise<number> => source.size(path),
        read: (_path: string, offset: number, length: number): Promise<Uint8Array> =>
          source.read(options.containerPath, offset, length),
        readBulk: source.readBulk
          ? (ranges: Parameters<NonNullable<FetchRangeLoader['readBulk']>>[1]): Promise<Uint8Array[]> =>
              source.readBulk!(options.containerPath, ranges)
          : undefined,
      };
      const pageProvider = createPageDataProvider(
        containerLoader,
        header,
        clients,
        options.format,
        options.cache,
        options.transcodeQueueCapacity,
        options.telemetry,
      );
      const session = new BigAssetSession(
        source, options.containerPath, header, options.format, workers,
        options.createMeshOptimizer, options.telemetry, pageProvider,
      );
      options.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.SessionOpen, correlation, workers.length, 0,
      );
      return session;
    } catch (error) {
      for (let index = workers.length - 1; index >= 0; index--) {
        try { await workers[index]?.close(); }
        catch (closeError) {
          if (error instanceof Error && error.cause === undefined) error.cause = closeError;
        }
      }
      options.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.SessionOpen, correlation, workers.length, 1,
      );
      throw error;
    }
  }

  async createAssetStore(
    capacity = 64,
    maxCompletionsPerPoll = 32,
  ): Promise<AssetStore> {
    if (this.closed) throw new Error('cannot create an asset store from a closed BIG session');
    if (this.assetStore || this.meshOptimizer) throw new Error('BIG session already created its asset store');
    const optimizer = this.createMeshOptimizer
      ? await this.createMeshOptimizer()
      : await createPlatformMeshOptimizer(this.telemetry);
    if (this.closed) { await optimizer.close(); throw new Error('BIG session closed while creating its asset store'); }
    try {
      this.assetStore = new AssetStore(
        this.rawAssets, optimizer, capacity, maxCompletionsPerPoll, this.telemetry,
      );
      this.meshOptimizer = optimizer;
      this.stats.workersStarted++;
      return this.assetStore;
    } catch (error) {
      try { await optimizer.close(); }
      catch (closeError) { if (error instanceof Error && error.cause === undefined) error.cause = closeError; }
      throw error;
    }
  }

  createVirtualTextureStore(device?: GPUDevice, tuning?: VirtualTextureTuning): VirtualTextureStore {
    if (this.closed) throw new Error('cannot create a VT store from a closed BIG session');
    if (this.store) throw new Error('BIG session already created its VT store');
    const loader = {
      read: (_path: string, offset: number, length: number): Promise<Uint8Array> =>
        this.source.read(this.containerPath, offset, length),
      poll(): void {},
    };
    this.store = new VirtualTextureStore(
      loader,
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
    for (let index = this.workers.length - 1; index >= 0; index--) {
      try { await this.workers[index]?.close(); }
      catch (error) {
        this.stats.closeErrors++;
        if (firstError === null) firstError = error;
      }
    }
    this.workers.length = 0;
    this.stats.closed = true;
    if (firstError !== null) throw firstError;
  }
}
