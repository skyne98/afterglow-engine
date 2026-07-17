import {
  BIG_MAGIC,
  BIG_VERSION,
  BigContainerAssetLoader,
  createFetchRangeLoader,
  createPageDataProvider,
  parseBigHeader,
  type BigHeader,
  type FetchRangeLoader,
  type VirtualTexturePageProvider,
} from './big-parser.ts';
import { AssetStore } from './asset-store.ts';
import type { PersistentBlobCache } from './persistent-blob-cache.ts';
import { VirtualTextureStore, VirtualTextureTuning } from './virtual-texture.ts';

export interface TextureTranscoder {
  transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
}

export interface OwnedTextureTranscoder {
  readonly worker: TextureTranscoder;
  close(): void | Promise<void>;
}

export interface BigAssetSessionOptions {
  containerPath: string;
  format: number;
  workerCount: number;
  transcodeQueueCapacity: number;
  maxHeaderBytes: number;
  createWorker(index: number): Promise<OwnedTextureTranscoder>;
  source?: FetchRangeLoader;
  cache?: PersistentBlobCache;
}

/** Bootstrap owner for one BIG source, its header, workers, and page provider. */
export class BigAssetSession {
  readonly rawAssets: BigContainerAssetLoader;
  readonly pageProvider: VirtualTexturePageProvider;
  readonly stats = { workersStarted: 0, closeErrors: 0, closed: false };

  private closed = false;
  private assetStore: AssetStore | null = null;
  private store: VirtualTextureStore | null = null;

  private constructor(
    readonly source: FetchRangeLoader,
    readonly containerPath: string,
    readonly header: BigHeader,
    readonly format: number,
    private readonly workers: OwnedTextureTranscoder[],
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
    if (!Number.isSafeInteger(options.maxHeaderBytes) || options.maxHeaderBytes < 16)
      throw new RangeError('BIG session maxHeaderBytes must be at least 16');

    const source = options.source ?? createFetchRangeLoader();
    const workers: OwnedTextureTranscoder[] = [];
    try {
      const prefix = await source.read(options.containerPath, 0, 16);
      if (prefix.byteLength !== 16) throw new Error('BIG session received a truncated prefix');
      const view = new DataView(prefix.buffer, prefix.byteOffset, prefix.byteLength);
      if (view.getUint32(0, true) !== BIG_MAGIC) throw new Error('BIG session has invalid magic');
      if (view.getUint32(4, true) !== BIG_VERSION) throw new Error('BIG session has unsupported version');
      const dataOffset = Number(view.getBigUint64(8, true));
      if (!Number.isSafeInteger(dataOffset) || dataOffset < 16 || dataOffset > options.maxHeaderBytes)
        throw new RangeError(`BIG header size ${dataOffset} exceeds configured capacity ${options.maxHeaderBytes}`);
      const headerBytes = await source.read(options.containerPath, 0, dataOffset);
      if (headerBytes.byteLength !== dataOffset) throw new Error('BIG session received a truncated header');
      const { header } = parseBigHeader(headerBytes);

      for (let index = 0; index < options.workerCount; index++)
        workers.push(await options.createWorker(index));
      const clients = workers.map((owned) => owned.worker);
      const containerLoader = {
        load: (path: string): Promise<Uint8Array> => source.load(path),
        size: (path: string): Promise<number> => source.size(path),
        read: (_path: string, offset: number, length: number): Promise<Uint8Array> =>
          source.read(options.containerPath, offset, length),
      };
      const pageProvider = createPageDataProvider(
        containerLoader,
        header,
        clients,
        options.format,
        options.cache,
        options.transcodeQueueCapacity,
      );
      return new BigAssetSession(
        source, options.containerPath, header, options.format, workers, pageProvider,
      );
    } catch (error) {
      for (let index = workers.length - 1; index >= 0; index--) {
        try { await workers[index]?.close(); }
        catch (closeError) {
          if (error instanceof Error && error.cause === undefined) error.cause = closeError;
        }
      }
      throw error;
    }
  }

  createAssetStore(
    meshopt?: ConstructorParameters<typeof AssetStore>[1],
    capacity = 64,
    maxCompletionsPerPoll = 32,
  ): AssetStore {
    if (this.closed) throw new Error('cannot create an asset store from a closed BIG session');
    if (this.assetStore) throw new Error('BIG session already created its asset store');
    this.assetStore = new AssetStore(this.rawAssets, meshopt, capacity, maxCompletionsPerPoll);
    return this.assetStore;
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
    );
    return this.store;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.assetStore?.dispose();
    this.assetStore = null;
    this.store?.dispose();
    this.store = null;
    let firstError: unknown = null;
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
