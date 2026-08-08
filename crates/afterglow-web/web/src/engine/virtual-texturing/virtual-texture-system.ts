import { FixedResourceRegistry, type StreamResourceHandle } from '../streaming/fixed-resource-registry.ts';
import {
  PersistentBlobStatus,
  type PersistentBlobStore,
} from '../streaming/persistent-blob-store.ts';
import { EngineMetric, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import {
  MemoryTextureWriteStatus,
  MemoryVirtualTextureSource,
  type MemoryPageSourceOptions,
  type MemoryVirtualTextureFormat,
  type MemoryTextureDirtyPage,
} from './memory-page-source.ts';
import {
  decodeMemoryTextureSnapshot,
  encodeMemoryTextureSnapshot,
  restoreMemoryTextureSnapshot,
  type DecodedMemoryTextureSnapshot,
  type RestoreMemoryTextureCapacities,
} from './memory-texture-snapshot.ts';
import {
  FORMAT_ASTC,
  FORMAT_BC7,
  FORMAT_R16F,
  FORMAT_R8,
  FORMAT_RGBA,
} from './virtual-texture-format.ts';
import {
  VirtualTextureStore,
  type PageDataProvider,
  type VirtualMaterialMipBiases,
  type VirtualMaterialSet,
  type VirtualTextureEntry,
} from './virtual-texture.ts';
import type { VirtualPageRequest } from './virtual-texture-request.ts';
import {
  VirtualTextureTuning,
  type VirtualTextureRuntimeCapacities,
} from './virtual-texture-tuning.ts';

export type VirtualTextureHandle = StreamResourceHandle & {
  readonly __virtualTextureHandle: unique symbol;
};

export type VirtualTextureStorageFormat =
  | 'rgba8unorm'
  | 'rgba8unorm-srgb'
  | 'r8unorm'
  | 'r16float'
  | 'bc7-rgba-unorm'
  | 'bc7-rgba-unorm-srgb'
  | 'astc-4x4-unorm'
  | 'astc-4x4-unorm-srgb';

export interface VirtualTextureDescriptor {
  readonly width: number;
  readonly height: number;
  readonly format: VirtualTextureStorageFormat;
  readonly addressMode: 'clamp' | 'repeat' | 'mirror-repeat';
  /** True for cooked sources that provide the packed 64→1 mip-tail page. */
  readonly mipTail?: boolean;
  readonly label?: string;
}

export interface VirtualTexturePoolConfig {
  readonly format: VirtualTextureStorageFormat;
  readonly capacities: Readonly<VirtualTextureRuntimeCapacities>;
  readonly tuning?: VirtualTextureTuning;
}

export interface VirtualTextureSystemOptions {
  readonly maxTextures: number;
  readonly maxMutablePageRefreshesPerPoll: number;
  readonly pools: readonly VirtualTexturePoolConfig[];
  readonly device: GPUDevice;
  readonly telemetry?: EngineTelemetry;
}

export const enum MemoryTexturePersistenceStatus {
  Ok = 0,
  InvalidTexture = 1,
  StorageFailure = 2,
  CorruptSnapshot = 3,
  CapacityExceeded = 4,
}

export interface MemoryTextureLoadResult {
  readonly status: MemoryTexturePersistenceStatus;
  readonly storageStatus: PersistentBlobStatus;
  readonly handle: VirtualTextureHandle | 0;
}

export interface VirtualTextureView {
  readonly texture: VirtualTextureHandle;
  readonly descriptor: Readonly<VirtualTextureDescriptor>;
  readonly entry: VirtualTextureEntry;
  readonly store: VirtualTextureStore;
}

export interface VirtualTextureInfo {
  texture: VirtualTextureHandle | 0;
  textureId: number;
  sourceKey: string;
  width: number;
  height: number;
  pageGridX: number;
  pageGridY: number;
}

/** Public material channels use opaque texture handles, never physical pools. */
export interface VirtualTextureMaterialSet {
  readonly albedo: VirtualTextureHandle;
  readonly normal?: VirtualTextureHandle;
  readonly masks?: VirtualTextureHandle;
  readonly roughness?: VirtualTextureHandle;
  readonly ao?: VirtualTextureHandle;
  readonly emissive?: VirtualTextureHandle;
}

/** Internal capability imported only by engine binding implementations. */
export const INSPECT_VIRTUAL_TEXTURE = Symbol('afterglow.inspectVirtualTexture');
export const RESOLVE_VIRTUAL_MATERIAL = Symbol('afterglow.resolveVirtualMaterial');

interface TextureRecord {
  handle: VirtualTextureHandle;
  readonly path: string;
  readonly descriptor: Readonly<VirtualTextureDescriptor>;
  readonly source: PageDataProvider;
  memory: MemoryVirtualTextureSource | null;
  readonly pool: TexturePool;
  readonly view: VirtualTextureView;
  readonly publish: (page: Readonly<MemoryTextureDirtyPage>, bytes: Uint8Array) => boolean;
}

interface TextureSourceRoute {
  readonly provider: PageDataProvider;
  readonly sourceKey: string | null;
}

interface TexturePool {
  readonly format: VirtualTextureStorageFormat;
  readonly sources: Map<string, TextureSourceRoute>;
  readonly store: VirtualTextureStore;
}

function storeFormat(format: VirtualTextureStorageFormat): number {
  if (format === 'r8unorm') return FORMAT_R8;
  if (format === 'r16float') return FORMAT_R16F;
  if (format.startsWith('bc7-')) return FORMAT_BC7;
  if (format.startsWith('astc-')) return FORMAT_ASTC;
  return FORMAT_RGBA;
}

function memoryFormat(format: VirtualTextureStorageFormat): MemoryVirtualTextureFormat {
  if (format === 'r8unorm') return 'r8unorm';
  if (format === 'r16float') return 'r16float';
  if (format === 'rgba8unorm' || format === 'rgba8unorm-srgb') return 'rgba8unorm';
  throw new RangeError('mutable RAM virtual textures require an uncompressed pool');
}

/**
 * One bounded texture namespace over interchangeable disk, procedural, and
 * mutable RAM page sources. Each storage format owns an explicitly configured
 * atlas pool; source policy never enters residency or shader code.
 */
export class VirtualTextureSystem {
  private readonly registry: FixedResourceRegistry<TextureRecord>;
  private readonly pools = new Map<VirtualTextureStorageFormat, TexturePool>();
  private readonly poolList: TexturePool[] = [];
  private readonly entriesById: (VirtualTextureEntry | null)[] = [null];
  private readonly storesById: (VirtualTextureStore | null)[] = [null];
  private nextPath = 1;
  private nextTextureId = 1;
  private disposed = false;

  constructor(private readonly options: Readonly<VirtualTextureSystemOptions>) {
    if (!Number.isInteger(options.maxMutablePageRefreshesPerPoll) ||
        options.maxMutablePageRefreshesPerPoll < 1)
      throw new RangeError('mutable VT refresh capacity must be positive');
    this.registry = new FixedResourceRegistry<TextureRecord>(options.maxTextures);
    for (const config of options.pools) {
      if (this.pools.has(config.format)) throw new Error(`duplicate VT pool: ${config.format}`);
      const sources = new Map<string, TextureSourceRoute>();
      const provider: PageDataProvider = async (path, request, signal) => {
        const route = sources.get(path);
        if (!route) throw new Error(`virtual texture source is unavailable: ${path}`);
        return route.provider(route.sourceKey ?? path, request, signal);
      };
      const store = new VirtualTextureStore(
        config.capacities,
        provider,
        storeFormat(config.format),
        options.device,
        config.tuning ?? new VirtualTextureTuning(),
        options.telemetry,
      );
      const pool = { format: config.format, sources, store };
      this.pools.set(config.format, pool);
      this.poolList.push(pool);
    }
    if (this.pools.size === 0) throw new RangeError('at least one virtual texture pool is required');
  }

  createTexture(
    descriptor: Readonly<VirtualTextureDescriptor>,
    source: PageDataProvider,
    sourceKey?: string,
  ): VirtualTextureHandle | 0 {
    if (this.disposed) return 0;
    const pool = this.pools.get(descriptor.format);
    if (!pool || !Number.isInteger(descriptor.width) || descriptor.width < 1 ||
        !Number.isInteger(descriptor.height) || descriptor.height < 1)
      return 0;
    const ownedDescriptor: Readonly<VirtualTextureDescriptor> = { ...descriptor };
    const path = `virtual://${this.nextPath++}`;
    pool.sources.set(path, { provider: source, sourceKey: sourceKey ?? null });
    const textureId = this.nextTextureId++;
    pool.store.loadTexture(path, {
      width: ownedDescriptor.width,
      height: ownedDescriptor.height,
      mipTail: ownedDescriptor.mipTail ?? false,
      textureId,
    });
    const entry = pool.store.getEntry(path);
    if (!entry) {
      pool.sources.delete(path);
      return 0;
    }
    this.entriesById[textureId] = entry;
    this.storesById[textureId] = pool.store;
    const view = {
      texture: 0 as VirtualTextureHandle,
      descriptor: ownedDescriptor,
      entry,
      store: pool.store,
    };
    let record!: TextureRecord;
    const publish = (page: Readonly<MemoryTextureDirtyPage>, bytes: Uint8Array): boolean =>
      this.publishMemoryPage(record, page, bytes);
    record = {
      handle: 0 as VirtualTextureHandle,
      path,
      descriptor: ownedDescriptor,
      source,
      memory: null,
      pool,
      view,
      publish,
    };
    const handle = this.registry.acquire(record);
    if (handle === 0) {
      pool.store.unloadTexture(path);
      pool.sources.delete(path);
      this.entriesById[textureId] = null;
      this.storesById[textureId] = null;
      return 0;
    }
    record.handle = handle as VirtualTextureHandle;
    view.texture = record.handle;
    return record.handle;
  }

  createMemoryTexture(
    descriptor: Readonly<VirtualTextureDescriptor>,
    sourceOptions: Omit<MemoryPageSourceOptions, 'width' | 'height' | 'format' | 'addressMode'>,
  ): VirtualTextureHandle | 0 {
    if (descriptor.mipTail) throw new RangeError('mutable RAM textures use ordinary terminal pages');
    const memory = new MemoryVirtualTextureSource({
      ...sourceOptions,
      width: descriptor.width,
      height: descriptor.height,
      format: memoryFormat(descriptor.format),
      addressMode: descriptor.addressMode,
    });
    return this.registerMemorySource(descriptor, memory);
  }

  private registerMemorySource(
    descriptor: Readonly<VirtualTextureDescriptor>,
    memory: MemoryVirtualTextureSource,
  ): VirtualTextureHandle | 0 {
    const handle = this.createTexture(descriptor, memory.provider);
    if (handle === 0) return 0;
    const record = this.registry.get(handle);
    if (!record) return 0;
    record.memory = memory;
    return handle;
  }

  async saveMemoryTexture(
    handle: VirtualTextureHandle,
    key: string,
    store: PersistentBlobStore,
  ): Promise<MemoryTexturePersistenceStatus> {
    const record = this.registry.get(handle);
    if (!record?.memory) return MemoryTexturePersistenceStatus.InvalidTexture;
    const bytes = encodeMemoryTextureSnapshot(record.memory, record.descriptor);
    const result = await store.putAtomic(key, bytes);
    return result.status === PersistentBlobStatus.Ok
      ? MemoryTexturePersistenceStatus.Ok
      : MemoryTexturePersistenceStatus.StorageFailure;
  }

  async loadMemoryTexture(
    key: string,
    store: PersistentBlobStore,
    capacities: Readonly<RestoreMemoryTextureCapacities>,
    maxSnapshotBytes: number,
  ): Promise<MemoryTextureLoadResult> {
    const loaded = await store.get(key, maxSnapshotBytes);
    if (loaded.status !== PersistentBlobStatus.Ok || loaded.bytes === null)
      return {
        status: MemoryTexturePersistenceStatus.StorageFailure,
        storageStatus: loaded.status,
        handle: 0,
      };
    let snapshot: DecodedMemoryTextureSnapshot;
    try {
      snapshot = decodeMemoryTextureSnapshot(loaded.bytes, capacities.pageCapacity);
    } catch {
      return {
        status: MemoryTexturePersistenceStatus.CorruptSnapshot,
        storageStatus: PersistentBlobStatus.Ok,
        handle: 0,
      };
    }
    try {
      const memory = restoreMemoryTextureSnapshot(snapshot, capacities);
      const handle = this.registerMemorySource(snapshot.descriptor, memory);
      return handle === 0
        ? {
            status: MemoryTexturePersistenceStatus.CapacityExceeded,
            storageStatus: PersistentBlobStatus.Ok,
            handle: 0,
          }
        : {
            status: MemoryTexturePersistenceStatus.Ok,
            storageStatus: PersistentBlobStatus.Ok,
            handle,
          };
    } catch {
      return {
        status: MemoryTexturePersistenceStatus.CapacityExceeded,
        storageStatus: PersistentBlobStatus.Ok,
        handle: 0,
      };
    }
  }

  /** Internal test/engine inspection capability; absent from the public barrel. */
  [INSPECT_VIRTUAL_TEXTURE](handle: VirtualTextureHandle): VirtualTextureView | null {
    return this.registry.get(handle)?.view ?? null;
  }

  /** Copy immutable address/descriptor data without exposing a physical pool. */
  readTextureInfo(handle: VirtualTextureHandle, out: VirtualTextureInfo): boolean {
    const record = this.registry.get(handle);
    if (!record) return false;
    out.texture = record.handle;
    out.textureId = record.view.entry.textureId;
    out.sourceKey = record.view.entry.path;
    out.width = record.descriptor.width;
    out.height = record.descriptor.height;
    out.pageGridX = record.view.entry.pageGridX;
    out.pageGridY = record.view.entry.pageGridY;
    return true;
  }

  getEntryById(textureId: number): VirtualTextureEntry | undefined {
    return this.entriesById[textureId >>> 0] ?? undefined;
  }

  writeMemoryRegion(
    handle: VirtualTextureHandle,
    x: number,
    y: number,
    width: number,
    height: number,
    source: Uint8Array,
    bytesPerRow?: number,
  ): MemoryTextureWriteStatus | null {
    const memory = this.registry.get(handle)?.memory;
    if (!memory) return null;
    const status = memory.writeRegion(x, y, width, height, source, bytesPerRow);
    this.options.telemetry?.trace.instant(
      EngineTraceDescriptor.MutableTextureWrite,
      handle as number,
      source.byteLength,
      status,
    );
    if (status === MemoryTextureWriteStatus.Written) {
      this.options.telemetry?.metrics.counterAdd(EngineMetric.MutableTextureWrites, 1);
      this.options.telemetry?.metrics.counterAdd(EngineMetric.MutableTextureBytes, source.byteLength);
    }
    return status;
  }

  recordFrameTime(frameTimeMs: number): void {
    for (let pool = 0; pool < this.poolList.length; pool++)
      this.poolList[pool]!.store.recordFrameTime(frameTimeMs);
  }

  setPublicationFrameId(frameId: number): void {
    for (let pool = 0; pool < this.poolList.length; pool++)
      this.poolList[pool]!.store.setPublicationFrameId(frameId);
  }

  processFeedback(feedback: ReadonlyMap<unknown, VirtualPageRequest>): void {
    for (let pool = 0; pool < this.poolList.length; pool++)
      this.poolList[pool]!.store.processFeedback(feedback);
  }

  processFeedbackBatch(
    maps: ReadonlyArray<ReadonlyMap<unknown, VirtualPageRequest> | null>,
    count: number,
  ): void {
    for (let pool = 0; pool < this.poolList.length; pool++)
      this.poolList[pool]!.store.processFeedbackBatch(maps, count);
  }

  /** Internal engine-binding capability; absent from the public barrel. */
  [RESOLVE_VIRTUAL_MATERIAL](
    handles: Readonly<VirtualTextureMaterialSet>,
    mipBiases?: Partial<VirtualMaterialMipBiases>,
  ): { readonly store: VirtualTextureStore; readonly set: VirtualMaterialSet } {
    const resolve = (handle: VirtualTextureHandle | undefined): VirtualTextureEntry | undefined => {
      if (handle === undefined) return undefined;
      const record = this.registry.get(handle);
      if (!record) throw new Error('virtual material contains a stale texture handle');
      return record.view.entry;
    };
    const albedo = resolve(handles.albedo);
    if (!albedo) throw new Error('virtual material requires an albedo texture');
    const set: VirtualMaterialSet = { albedo };
    const normal = resolve(handles.normal);
    const masks = resolve(handles.masks);
    const roughness = resolve(handles.roughness);
    const ao = resolve(handles.ao);
    const emissive = resolve(handles.emissive);
    if (normal) set.normal = normal;
    if (masks) set.masks = masks;
    if (roughness) set.roughness = roughness;
    if (ao) set.ao = ao;
    if (emissive) set.emissive = emissive;

    let store: VirtualTextureStore | null = null;
    for (const entry of [albedo, normal, masks, roughness, ao, emissive]) {
      if (!entry) continue;
      const candidate = this.storesById[entry.textureId];
      if (!candidate) throw new Error('virtual material texture is not owned by this system');
      if (store && store !== candidate)
        throw new Error('linked virtual material channels require one physical format pool');
      store = candidate;
    }
    if (!store) throw new Error('virtual material set has no registered textures');
    store.linkMaterialSet(set, mipBiases);
    return { store, set };
  }

  private primaryStore(): VirtualTextureStore {
    if (this.poolList.length !== 1)
      throw new Error('aggregate VT diagnostics require exactly one format pool');
    return this.poolList[0]!.store;
  }

  getStats(): ReturnType<VirtualTextureStore['getStats']> {
    return this.primaryStore().getStats();
  }

  getDebugSnapshot(): ReturnType<VirtualTextureStore['getDebugSnapshot']> {
    return this.primaryStore().getDebugSnapshot();
  }

  get atlasWidth(): number { return this.primaryStore().atlasWidth; }
  get atlasHeight(): number { return this.primaryStore().atlasHeight; }
  get rendererAttached(): boolean {
    for (let pool = 0; pool < this.poolList.length; pool++)
      if (!this.poolList[pool]!.store.gpuAtlasTexture) return false;
    return this.poolList.length !== 0;
  }

  // @hot-no-alloc-begin VirtualTextureSystem.isBootstrapReady
  /** Required mip tails must be resident before the first frame is GameReady. */
  isBootstrapReady(): boolean {
    for (let pool = 0; pool < this.poolList.length; pool++) {
      const stats = this.poolList[pool]!.store.getStats();
      if (stats.failedLoads !== 0 || stats.pendingPages !== 0 ||
          stats.readyUploads !== 0 || stats.scheduledRequests !== 0 ||
          stats.bulkInFlight !== 0 || stats.atlasSlotsUsed < stats.textureCount)
        return false;
    }
    return true;
  }
  // @hot-no-alloc-end VirtualTextureSystem.isBootstrapReady

  /** Advance source revisions, in-place resident replacement, and normal VT work. */
  poll(): void {
    let remaining = this.options.maxMutablePageRefreshesPerPoll;
    let published = 0;
    let deferred = 0;
    this.options.telemetry?.trace.spanBegin(
      EngineTraceDescriptor.MutablePageRefresh, 0,
      this.options.maxMutablePageRefreshesPerPoll, 0,
    );
    for (let slot = 0; slot < this.registry.capacity; slot++) {
      const record = this.registry.valueAt(slot);
      if (!record?.memory) continue;
      if (remaining > 0) {
        const drained = record.memory.drainDirty(remaining, record.publish);
        published += drained;
        remaining -= drained;
      }
      deferred += record.memory.pendingDirtyPages;
    }
    if (published > 0)
      this.options.telemetry?.metrics.counterAdd(EngineMetric.MutablePagesPublished, published);
    if (deferred > 0)
      this.options.telemetry?.metrics.counterAdd(EngineMetric.MutablePagesDeferred, deferred);
    this.options.telemetry?.trace.spanEnd(
      EngineTraceDescriptor.MutablePageRefresh, 0, published, deferred,
    );
    for (let pool = 0; pool < this.poolList.length; pool++) this.poolList[pool]!.store.poll();
  }

  private publishMemoryPage(
    record: TextureRecord,
    page: Readonly<MemoryTextureDirtyPage>,
    bytes: Uint8Array,
  ): boolean {
    if (record.pool.store.replaceResidentPage(record.path, page, bytes)) return true;
    // Keep a dirty page queued only while an older source revision is already
    // in flight. A genuinely nonresident page will read current RAM on demand.
    return !record.pool.store.isPagePending(record.path, page);
  }

  destroyTexture(handle: VirtualTextureHandle): boolean {
    const record = this.registry.release(handle);
    if (!record) return false;
    record.pool.store.unloadTexture(record.path);
    record.pool.sources.delete(record.path);
    this.entriesById[record.view.entry.textureId] = null;
    this.storesById[record.view.entry.textureId] = null;
    return true;
  }

  attachRenderer(renderer: Parameters<VirtualTextureStore['attachRenderer']>[0]): void {
    for (let pool = 0; pool < this.poolList.length; pool++)
      this.poolList[pool]!.store.attachRenderer(renderer);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (let slot = 0; slot < this.registry.capacity; slot++) {
      const record = this.registry.valueAt(slot);
      if (record) this.destroyTexture(record.handle);
    }
    for (let pool = 0; pool < this.poolList.length; pool++) this.poolList[pool]!.store.dispose();
    this.pools.clear();
    this.poolList.length = 0;
  }
}
