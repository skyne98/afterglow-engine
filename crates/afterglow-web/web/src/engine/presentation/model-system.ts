import type * as THREE from 'three/webgpu';
import type { MeshOptimizer } from '../assets/asset-store.ts';
import { createPlatformMeshOptimizer } from '../assets/platform-workers.ts';
import type { OwnedMeshOptimizer } from '../assets/service-types.ts';
import { FixedResourceRegistry, type StreamResourceHandle } from '../streaming/fixed-resource-registry.ts';
import { EngineMetric, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import type { CookedModelAsset } from './cooked-model-source.ts';
import {
  GeometryArena,
  type GeometryArenaOptions,
  type GeometryArenaPublication,
  type GeometryArenaStats,
} from './geometry-arena.ts';
import {
  ModelLodBinding,
  buildModelGeometryLods,
  type ModelGeometryLod,
  type ModelLodBuildOptions,
} from './model-lod.ts';

export type ModelHandle = StreamResourceHandle & { readonly __modelHandle: unique symbol };
export type ModelResourceStatus = 'optimizing' | 'ready' | 'error';

export interface ModelResourceView {
  readonly handle: ModelHandle;
  readonly status: ModelResourceStatus;
  readonly revision: number;
  readonly levels: readonly ModelGeometryLod[];
  readonly residentBytes: number;
}

interface MutableModelView {
  handle: ModelHandle;
  status: ModelResourceStatus;
  revision: number;
  levels: readonly ModelGeometryLod[];
  residentBytes: number;
}

interface ModelRecord {
  handle: ModelHandle;
  token: number;
  pending: boolean;
  source: THREE.BufferGeometry | null;
  publication: GeometryArenaPublication | null;
  readonly view: MutableModelView;
}

export interface ModelSystemOptions extends ModelLodBuildOptions {
  readonly maxModels: number;
  readonly geometryArena: Readonly<Pick<GeometryArenaOptions, 'buckets'>>;
  readonly maxPendingOptimizations: number;
  readonly maxResidentCpuBytes: number;
  readonly completionsPerPoll: number;
}

function geometryBytes(geometry: THREE.BufferGeometry): number {
  let bytes = geometry.index?.array.byteLength ?? 0;
  for (const attribute of Object.values(geometry.attributes)) bytes += attribute.array.byteLength;
  for (const morphs of Object.values(geometry.morphAttributes))
    if (morphs) for (const morph of morphs) bytes += morph.array.byteLength;
  return bytes;
}

function disposeLevels(levels: readonly ModelGeometryLod[]): void {
  for (const level of levels) level.geometry.dispose();
}

/**
 * Fixed model ownership and atomic revision publication. Cooked disk LODs and
 * runtime RAM geometry use the same handles/views; only runtime sources pass
 * through the asynchronous deformation-aware meshopt processor.
 */
export class ModelSystem {
  private readonly registry: FixedResourceRegistry<ModelRecord>;
  private readonly completionHandles: Float64Array;
  private readonly completionTokens: Uint32Array;
  private readonly completionLevels: (readonly ModelGeometryLod[] | null)[];
  private readonly completionErrors: (unknown | null)[];
  private completionHead = 0;
  private completionTail = 0;
  private completionCount = 0;
  private pendingCount = 0;
  private residentBytes = 0;
  private disposed = false;
  private readonly options: Readonly<ModelSystemOptions>;
  private readonly geometryArena: GeometryArena;
  private closed = false;

  constructor(
    private readonly optimizer: MeshOptimizer,
    options: Readonly<ModelSystemOptions>,
    private readonly telemetry?: EngineTelemetry,
    private readonly ownedOptimizer: OwnedMeshOptimizer | null = null,
  ) {
    this.options = { ...options, ratios: Array.from(options.ratios) };
    if (!Number.isInteger(options.maxModels) || options.maxModels < 1 ||
        !Number.isInteger(options.maxPendingOptimizations) || options.maxPendingOptimizations < 1 ||
        options.maxPendingOptimizations > options.maxModels ||
        !Number.isInteger(options.maxResidentCpuBytes) || options.maxResidentCpuBytes < 1 ||
        !Number.isInteger(options.completionsPerPoll) || options.completionsPerPoll < 1)
      throw new RangeError('invalid model-system capacities');
    this.geometryArena = new GeometryArena({
      buckets: options.geometryArena.buckets,
      ...(telemetry ? { telemetry } : {}),
    });
    this.registry = new FixedResourceRegistry<ModelRecord>(options.maxModels);
    this.completionHandles = new Float64Array(options.maxModels);
    this.completionTokens = new Uint32Array(options.maxModels);
    this.completionLevels = new Array(options.maxModels).fill(null);
    this.completionErrors = new Array(options.maxModels).fill(null);
  }

  /** Standalone bounded model owner; ordinary assets use EngineAssets composition. */
  static async open(
    options: Readonly<ModelSystemOptions>,
    telemetry?: EngineTelemetry,
  ): Promise<ModelSystem> {
    const optimizer = await createPlatformMeshOptimizer(telemetry);
    try { return new ModelSystem(optimizer, options, telemetry, optimizer); }
    catch (error) { await optimizer.close(); throw error; }
  }

  createRuntimeModel(geometry: THREE.BufferGeometry): ModelHandle | 0 {
    if (this.disposed || this.pendingCount === this.options.maxPendingOptimizations) return 0;
    const view: MutableModelView = {
      handle: 0 as ModelHandle,
      status: 'optimizing',
      revision: 0,
      levels: [],
      residentBytes: 0,
    };
    const record: ModelRecord = {
      handle: 0 as ModelHandle,
      token: 0,
      pending: false,
      source: geometry,
      publication: null,
      view,
    };
    const handle = this.registry.acquire(record);
    if (handle === 0) return 0;
    record.handle = handle as ModelHandle;
    view.handle = record.handle;
    this.startOptimization(record);
    return record.handle;
  }

  /** Adopt already meshopt-processed disk LODs into the same runtime handle space. */
  adoptCookedModel(asset: CookedModelAsset): ModelHandle | 0 {
    if (this.disposed) return 0;
    const ratios = this.options.ratios;
    const levels: ModelGeometryLod[] = asset.levels.map((level, index) => ({
      geometry: level.geometry,
      ratio: ratios[index] ?? Math.max(0.01, 1 / 2 ** index),
      triangleCount: level.triangleCount,
    }));
    let bytes = 0;
    for (const level of levels) bytes += geometryBytes(level.geometry);
    if (this.residentBytes + bytes > this.options.maxResidentCpuBytes) return 0;
    const publication = this.geometryArena.publish(levels);
    if (!publication) return 0;
    const publishedLevels = publication?.levels ?? levels;
    const view: MutableModelView = {
      handle: 0 as ModelHandle,
      status: 'ready',
      revision: 1,
      levels: publishedLevels,
      residentBytes: bytes,
    };
    const record: ModelRecord = {
      handle: 0 as ModelHandle,
      token: 1,
      pending: false,
      source: null,
      publication,
      view,
    };
    const handle = this.registry.acquire(record);
    if (handle === 0) {
      this.geometryArena.release(publication);
      return 0;
    }
    record.handle = handle as ModelHandle;
    view.handle = record.handle;
    asset.takeLevels();
    disposeLevels(levels);
    this.residentBytes += bytes;
    this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsPublished, 1);
    this.telemetry?.metrics.maximum(EngineMetric.ModelCpuBytesHighWater, this.residentBytes);
    this.telemetry?.trace.instant(
      EngineTraceDescriptor.ModelPublished, record.handle as number, 1, bytes,
    );
    return record.handle;
  }

  /** Replace canonical RAM geometry; old LODs remain visible until publication. */
  reviseRuntimeModel(handle: ModelHandle, geometry: THREE.BufferGeometry): boolean {
    const record = this.registry.get(handle);
    if (!record || !record.source || record.pending ||
        this.pendingCount === this.options.maxPendingOptimizations) return false;
    record.source = geometry;
    record.view.status = 'optimizing';
    this.startOptimization(record);
    return true;
  }

  private startOptimization(record: ModelRecord): void {
    const source = record.source;
    if (!source) return;
    const token = ++record.token;
    record.pending = true;
    this.pendingCount++;
    this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsQueued, 1);
    this.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.ModelRevision,
      this.revisionCorrelation(record.handle, token),
      token,
      0,
    );
    buildModelGeometryLods(source, this.optimizer, this.options).then(
      levels => this.enqueue(record.handle, token, levels, null),
      error => this.enqueue(record.handle, token, null, error),
    );
  }

  private revisionCorrelation(handle: ModelHandle, token: number): number {
    return (handle as number) * 0x1_0000 + (token & 0xffff);
  }

  private enqueue(
    handle: ModelHandle,
    token: number,
    levels: readonly ModelGeometryLod[] | null,
    error: unknown | null,
  ): void {
    if (this.completionCount === this.completionHandles.length) {
      if (levels) disposeLevels(levels);
      const record = this.registry.get(handle);
      if (record && record.token === token) {
        record.pending = false;
        record.view.status = record.view.levels.length === 0 ? 'error' : 'ready';
        this.pendingCount--;
      }
      this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsFailed, 1);
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.ModelRevision,
        this.revisionCorrelation(handle, token), token, 4,
      );
      return;
    }
    const slot = this.completionTail;
    this.completionHandles[slot] = handle as number;
    this.completionTokens[slot] = token;
    this.completionLevels[slot] = levels;
    this.completionErrors[slot] = error;
    this.completionTail = (slot + 1) % this.completionHandles.length;
    this.completionCount++;
  }

  poll(): void {
    this.optimizer.poll();
    for (let count = 0; count < this.options.completionsPerPoll && this.completionCount !== 0; count++) {
      const slot = this.completionHead;
      const handle = this.completionHandles[slot] as ModelHandle;
      const token = this.completionTokens[slot] ?? 0;
      const levels = this.completionLevels[slot];
      const error = this.completionErrors[slot];
      this.completionLevels[slot] = null;
      this.completionErrors[slot] = null;
      this.completionHead = (slot + 1) % this.completionHandles.length;
      this.completionCount--;
      const record = this.registry.get(handle);
      if (!record || record.token !== token) {
        if (levels) disposeLevels(levels);
        this.telemetry?.trace.asyncEnd(
          EngineTraceDescriptor.ModelRevision,
          this.revisionCorrelation(handle, token), token, 2,
        );
        continue;
      }
      record.pending = false;
      this.pendingCount--;
      if (!levels || error) {
        record.view.status = record.view.levels.length === 0 ? 'error' : 'ready';
        this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsFailed, 1);
        this.telemetry?.trace.asyncEnd(
          EngineTraceDescriptor.ModelRevision,
          this.revisionCorrelation(handle, token), token, 1,
        );
        continue;
      }
      let bytes = 0;
      for (const level of levels) bytes += geometryBytes(level.geometry);
      const previousBytes = record.view.residentBytes;
      if (this.residentBytes - previousBytes + bytes > this.options.maxResidentCpuBytes) {
        disposeLevels(levels);
        record.view.status = record.view.levels.length === 0 ? 'error' : 'ready';
        this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsFailed, 1);
        this.telemetry?.trace.asyncEnd(
          EngineTraceDescriptor.ModelRevision,
          this.revisionCorrelation(handle, token), token, 3,
        );
        continue;
      }
      const publication = this.geometryArena.publish(levels);
      if (!publication) {
        disposeLevels(levels);
        record.view.status = record.view.levels.length === 0 ? 'error' : 'ready';
        this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsFailed, 1);
        this.telemetry?.trace.asyncEnd(
          EngineTraceDescriptor.ModelRevision,
          this.revisionCorrelation(handle, token), token, 5,
        );
        continue;
      }
      if (record.publication) this.geometryArena.release(record.publication);
      disposeLevels(levels);
      record.publication = publication;
      this.residentBytes = this.residentBytes - previousBytes + bytes;
      record.view.levels = publication.levels;
      record.view.residentBytes = bytes;
      record.view.revision++;
      record.view.status = 'ready';
      this.telemetry?.metrics.counterAdd(EngineMetric.ModelRevisionsPublished, 1);
      this.telemetry?.metrics.maximum(EngineMetric.ModelCpuBytesHighWater, this.residentBytes);
      this.telemetry?.trace.instant(
        EngineTraceDescriptor.ModelPublished,
        handle as number,
        record.view.revision,
        bytes,
      );
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.ModelRevision,
        this.revisionCorrelation(handle, token), token, 0,
      );
    }
  }

  getView(handle: ModelHandle): Readonly<ModelResourceView> | null {
    return this.registry.get(handle)?.view ?? null;
  }

  createBinding(
    handle: ModelHandle,
    source: THREE.Mesh,
    thresholds: Float32Array,
    hysteresis: number,
  ): ModelLodBinding | null {
    const view = this.registry.get(handle)?.view;
    return view?.status === 'ready'
      ? new ModelLodBinding(source, view.levels, thresholds, hysteresis, false)
      : null;
  }

  destroyModel(handle: ModelHandle): boolean {
    const record = this.registry.release(handle);
    if (!record) return false;
    record.token++;
    if (record.pending) {
      record.pending = false;
      this.pendingCount--;
    }
    this.residentBytes -= record.view.residentBytes;
    if (record.publication) this.geometryArena.release(record.publication);
    record.publication = null;
    record.view.levels = [];
    record.view.residentBytes = 0;
    return true;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (let slot = 0; slot < this.registry.capacity; slot++) {
      const record = this.registry.valueAt(slot);
      if (record) this.destroyModel(record.handle);
    }
    while (this.completionCount !== 0) {
      const slot = this.completionHead;
      const levels = this.completionLevels[slot];
      if (levels) disposeLevels(levels);
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.ModelRevision,
        this.revisionCorrelation(
          this.completionHandles[slot] as ModelHandle,
          this.completionTokens[slot] ?? 0,
        ),
        this.completionTokens[slot] ?? 0,
        2,
      );
      this.completionLevels[slot] = null;
      this.completionHead = (slot + 1) % this.completionHandles.length;
      this.completionCount--;
    }
    this.geometryArena.dispose();
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.dispose();
    await this.ownedOptimizer?.close();
  }

  getGeometryStats(): Readonly<GeometryArenaStats> { return this.geometryArena.getStats(); }
  get activeModels(): number { return this.registry.size; }
  get pendingOptimizations(): number { return this.pendingCount; }
  get residentCpuGeometryBytes(): number { return this.residentBytes; }
}
