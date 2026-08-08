// VirtualTextureStore — manages the shared physical atlas, page table,
// second-chance cache, GPU-feedback residency, and upload budget for all virtual textures.
//
// ALL sampled textures in the engine go through this store. There are no
// separate "normal" textures — everything is a page in the atlas.
//
// Architecture:
//   .big file → [page chunks at seekable offsets]
//                    ↓
//   PageDataProvider(path, request) → final GPU-format page data
//                    ↓
//   GPU feedback → deduplicate/capacity-fit → copy missing pages to atlas
//                    ↓
//   Page table texture (updated) → GPU shader samples via vtSample()
//
// The atlas and page table are THREE.DataTextures shared across all materials.
// Each virtual texture has its own page table binding (UV offset + scale).

import * as THREE from 'three';
import { AssetHandle } from '../assets/asset-handle.ts';
import type { FixedByteLease } from '../streaming/fixed-byte-lease-pool.ts';
import { EngineMetric, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import { TelemetryRecordStatus, type EngineTelemetry } from '../telemetry/telemetry.ts';
import { FixedPageSlotMap } from './fixed-page-slot-map.ts';
import {
  ATLAS_WIDTH,
  FORMAT_ASTC,
  FORMAT_BC7,
  FORMAT_R16F,
  FORMAT_RGBA,
  PAGE_SIZE,
  SLOT_BLOCKS_Y,
  SLOT_SIZE,
  isCompressedTextureFormat,
  isPageTableEntryResident as isResident,
  packPageTableEntry as packEntry,
  uncompressedBytesPerTexel,
  threeFormat,
} from './virtual-texture-format.ts';
import {
  PageCache,
  PageTable,
  type PageSlot,
} from './virtual-texture-residency.ts';
import {
  MATERIAL_CHANNEL_PRIORITY_COUNT,
  MAX_MIP,
  MAX_PAGE_SCORE,
  MAX_PERCEPTUAL_WEIGHT,
  PAGE_KIND_PRIORITY_COUNT,
  PRIORITY_LANE_COUNT,
  SCORE_COVERAGE_CAP,
  packedPageCoordinates,
  packedPageIdentity,
  pageBatchTier,
  perceptualPriority,
  sourcePerceptualWeight,
  type CachedPage,
  type PageRequest,
  type VirtualPageRequest,
} from './virtual-texture-request.ts';
import {
  VirtualTextureTuning,
  type VirtualTextureRuntimeCapacities,
} from './virtual-texture-tuning.ts';
import {
  PackedPageTableLayout,
  assertVirtualTextureDimensions,
  createPackedPageTableLayout,
  pageGridAtMip,
  packedMipTailIndex,
  packedPageTableIndex,
} from './virtual-texture-layout.ts';

// ============================================================================
// Constants
// ============================================================================

const FEEDBACK_SCALE = 0.125;

const enum SchedulerWaitStatus {
  Admitted = 0,
  Resident = 1,
  Stale = 2,
  Invalid = 3,
  Unloaded = 4,
}

/** A virtual texture descriptor — created by loadTexture(). */
export interface VirtualTextureEntry {
  /** Stable u32 identity written into RG32Uint feedback. */
  textureId: number;
  /** Path in the .big file (or loader key). */
  path: string;
  /** Independent source dimensions in texels. */
  width: number;
  height: number;
  /** Independent page counts at mip zero. */
  pageGridX: number;
  pageGridY: number;
  /** Last independently paged mip (the 128x128 level). */
  maxMip: number;
  /** Last image mip, including packed 64x64 through 1x1 tail levels. */
  textureMaxMip: number;
  /** First mip in the packed tail, or null when no tail is provided. */
  tailFirstMip: number | null;
  /** Packed resident entry for the tail's physical atlas slot. */
  tailEntry: number;
  /** Vertically packed r32uint page-table layout for every virtual mip. */
  pageTableLayout: PackedPageTableLayout;
  /** One u32 per packed page-table texel. */
  pageTable: Uint32Array;
  /** GPU-readable r32uint page table; virtual mips are packed in mip level 0. */
  pageTableTexture: THREE.DataTexture;
}

/** PBR channels sharing dimensions and page coordinates. */
interface PendingPageRecord {
  generation: number;
  page: CachedPage;
  lastSeen: number;
  startedAt: number;
  priorityTier: number;
  controller: AbortController | null;
  canceled: boolean;
}

interface ReadyPageUpload {
  key: number;
  generation: number;
  page: CachedPage;
  req: PageRequest;
  data: Uint8Array;
  lease: FixedByteLease | null;
}

interface PageDataProviderTelemetry {
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

export type PageDataResult = Uint8Array | FixedByteLease;

export type PageDataProvider = ((path: string, req: PageRequest, signal?: AbortSignal) => Promise<PageDataResult>) & {
  getStats?(): Readonly<PageDataProviderTelemetry>;
};

export interface VirtualMaterialSet {
  albedo: VirtualTextureEntry;
  normal?: VirtualTextureEntry;
  /** Packed linear masks: R=roughness, G=ambient occlusion. */
  masks?: VirtualTextureEntry;
  roughness?: VirtualTextureEntry;
  ao?: VirtualTextureEntry;
  emissive?: VirtualTextureEntry;
}

/** Per-channel coarsening relative to the material's albedo feedback mip. */
export interface VirtualMaterialMipBiases {
  albedo: number;
  normal: number;
  masks: number;
  roughness: number;
  ao: number;
  emissive: number;
}

/** Albedo-first defaults: full color, one-level-coarser vector data, two-level-coarser scalars. */
export const DEFAULT_VIRTUAL_MATERIAL_MIP_BIASES: Readonly<VirtualMaterialMipBiases> = {
  albedo: 0,
  normal: 1,
  masks: 2,
  roughness: 2,
  ao: 2,
  emissive: 1,
};

interface MaterialStreamChannel {
  textureId: number;
  mipBias: number;
  channelPriority: number;
}

// ============================================================================
// Virtual Texture Store
// ============================================================================

/**
 * Manages the shared physical atlas, page tables, clock cache, GPU-feedback
 * residency, and bounded page uploads for all virtual textures.
 *
 * ALL sampled textures go through this store. The atlas and page table are
 * shared GPU textures. Each virtual texture has its own page table.
 *
 * This is the physical pool behind `VirtualTextureSystem`; game code obtains
 * generational system handles and never installs the pool as an ECS resource.
 * The pool reads page data from the .big file via the pageDataProvider.
 * Pages are transcoded from Basis to the optimal GPU format on load.
 */
export class VirtualTextureStore {
  /** The shared physical atlas texture (all VT textures sample from this). */
  readonly atlasTexture: THREE.Texture;
  readonly atlasWidth: number;
  readonly atlasHeight: number;
  readonly atlasPagesX: number;
  readonly atlasPagesY: number;
  /** The raw WebGPU GPUTexture for the atlas (for writeTexture). */
  gpuAtlasTexture: GPUTexture | null = null;
  /** Native page-table textures obtained from the Three.js WebGPU backend. */
  private gpuPageTables = new Map<string, GPUTexture>();
  /** The detected texture format (FORMAT_BC7, FORMAT_ASTC, or FORMAT_RGBA). */
  readonly format: number;
  /** The WebGPU device (for compressed texture writes). */
  private device: GPUDevice | null = null;

  /** Page tables per virtual texture path and feedback identity. */
  private entries = new Map<string, VirtualTextureEntry>();
  private entriesById: (VirtualTextureEntry | null)[] = [null];
  private pageTablesById: (PageTable | null)[] = [null];
  /** One albedo feedback identity expands to independently resident channel requests. */
  private materialChannelsBySourceId: (readonly MaterialStreamChannel[] | null)[] = [null];
  private nextTextureId = 1;

  /** Per-path page tables (for lookup). */
  private pageTables = new Map<string, PageTable>();

  /** The shared page cache (atlas + fixed second-chance clock). */
  private cache: PageCache;

  /** Page data loader (returns raw page bytes for a given request). */
  private pageDataProvider?: PageDataProvider;

  /** Fixed in-flight table; slots are acquired at ready time. */
  private readonly pendingRecords: PendingPageRecord[];
  private readonly pendingActive: Uint8Array;
  private readonly pendingFree: Uint32Array;
  private pendingFreeTop = 0;
  private readonly pendingByKey: FixedPageSlotMap;
  private pendingCount = 0;
  private readonly readyUploads: ReadyPageUpload[];
  private readyUploadHead = 0;
  private readyUploadTail = 0;
  private readyUploadCount = 0;
  private loadGeneration = 0;
  private disposed = false;
  /** Shared central policy; created during bootstrap and sampled each frame. */
  readonly tuning: VirtualTextureTuning;
  private readonly maxPendingPages: number;
  private readonly maxPendingBytes: number;
  private pendingBytes = 0;
  private readonly scheduleBudgetMs = 0.25;
  private rejectedAdmissions = 0;
  private scheduleBudgetExhaustions = 0;
  private uploadBudgetExhaustions = 0;
  private readonly pageTableUploadScratch = new Uint32Array(1);

  /** Fixed persistent request set; survives individual feedback frames. */
  private scheduledKeys: Float64Array;
  private scheduledRequests: VirtualPageRequest[];
  private scheduledActive: Uint8Array;
  private scheduledLastSeen: Uint32Array;
  private scheduledPriority: Uint8Array;
  private scheduledTraceActive: Uint8Array;
  private scheduledNext: Int32Array;
  private scheduledPrevious: Int32Array;
  private readonly priorityHeads = new Int32Array(PRIORITY_LANE_COUNT);
  private readonly priorityTails = new Int32Array(PRIORITY_LANE_COUNT);
  private scheduledFree: Uint32Array;
  private scheduledFreeTop = 0;
  private scheduledByKey: FixedPageSlotMap;
  private scheduledCount = 0;
  private feedbackEpoch = 0;
  private publicationFrameId = 0;
  /** Drop after two absent snapshots; elapsed time depends on the caller's feedback cadence. */
  private readonly staleFeedbackEpochs = 2;
  private staleCancellations = 0;
  private priorityPreemptions = 0;
  private schedulerOverflows = 0;
  private feedbackScratch: VirtualPageRequest[];
  private feedbackScratchKeys: FixedPageSlotMap;
  private feedbackScratchCount = 0;
  private residentHits = 0;
  private residentMisses = 0;
  private residentEvictions = 0;
  private completedLoads = 0;
  private failedLoads = 0;
  private totalLoadMs = 0;
  private maxLoadMs = 0;
  private completedUploads = 0;
  private totalUploadMs = 0;
  private maxUploadMs = 0;

  private capacityLodBias = 0;
  private pageBudget = 8;
  private debugPaused = false;
  private debugPageBudget: number | null = null;
  /** Stable mutable view updated by getStats(); callers must treat it as read-only. */
  private readonly feedbackResult = {
    loaded: 0, evicted: 0, totalRequests: 0, queuedRequests: 0, lodBias: 0,
  };
  private readonly stats = {
    textureCount: 0, atlasSlotsUsed: 0, atlasSlotsFree: 0, atlasSlotsReserved: 0, atlasSlotsTotal: 0, trackedPages: 0,
    pendingPages: 0, lodBias: 0, budget: 0, readyUploads: 0,
    maxPendingPages: 0, pendingBytes: 0, maxPendingBytes: 0,
    scheduledRequests: 0, schedulerCapacity: 0, schedulerOverflows: 0,
    staleCancellations: 0, priorityPreemptions: 0, rejectedAdmissions: 0,
    residentHits: 0, residentMisses: 0, residentEvictions: 0, completedLoads: 0, failedLoads: 0,
    averageLoadMs: 0, maxLoadMs: 0, completedUploads: 0,
    averageUploadMs: 0, maxUploadMs: 0, scheduleBudgetMs: 0,
    uploadBudgetMs: 0, uploadsPerPoll: 0, tuningDownshifts: 0, tuningRecoveries: 0,
    tuningProbes: 0, tuningProbeRejections: 0,
    tuningBestSafeUploadsPerPoll: 0, tuningBestSafeUploadBudgetMs: 0,
    scheduleBudgetExhaustions: 0, uploadBudgetExhaustions: 0,
    pageReads: 0, averagePageReadMs: 0, maxPageReadMs: 0,
    bulkQueued: 0, bulkInFlight: 0, bulkInFlightBytes: 0,
    urgentBatches: 0, focusBatches: 0, peripheralBatches: 0,
    bulkRejected: 0, bulkCanceled: 0,
    transcodeWorkers: 0, activeTranscodes: 0, queuedTranscodes: 0,
    completedTranscodes: 0, averageTranscodeQueueMs: 0, maxTranscodeQueueMs: 0,
    averageTranscodeMs: 0, maxTranscodeMs: 0,
  };

  constructor(
    capacities: Readonly<VirtualTextureRuntimeCapacities>,
    pageDataProvider?: PageDataProvider,
    format?: number,
    device?: GPUDevice,
    tuning?: VirtualTextureTuning,
    private readonly telemetry?: EngineTelemetry,
  ) {
    this.pageDataProvider = pageDataProvider;
    this.format = format ?? FORMAT_RGBA;
    this.device = device ?? null;
    this.tuning = tuning ?? new VirtualTextureTuning();
    const deviceMax = device?.limits.maxTextureDimension2D ?? ATLAS_WIDTH;
    const atlasDim = this.tuning.atlasMaxDimension > 0 ? this.tuning.atlasMaxDimension : deviceMax;
    this.cache = new PageCache(this.format, atlasDim);
    if (!Number.isInteger(capacities.maxPendingPages) || capacities.maxPendingPages < 1 ||
        !Number.isInteger(capacities.maxPendingBytes) ||
        capacities.maxPendingBytes < this.cache.slotDataSize) {
      throw new RangeError('invalid virtual-texture runtime capacities');
    }
    this.maxPendingPages = capacities.maxPendingPages;
    this.maxPendingBytes = capacities.maxPendingBytes;
    this.atlasWidth = this.cache.width;
    this.atlasHeight = this.cache.height;
    this.atlasPagesX = this.cache.pagesX;
    this.atlasPagesY = this.cache.pagesY;

    const schedulerCapacity = this.cache.totalSlots;
    this.scheduledKeys = new Float64Array(schedulerCapacity);
    this.scheduledRequests = new Array(schedulerCapacity);
    this.scheduledActive = new Uint8Array(schedulerCapacity);
    this.scheduledLastSeen = new Uint32Array(schedulerCapacity);
    this.scheduledPriority = new Uint8Array(schedulerCapacity);
    this.scheduledTraceActive = new Uint8Array(schedulerCapacity);
    this.scheduledNext = new Int32Array(schedulerCapacity);
    this.scheduledPrevious = new Int32Array(schedulerCapacity);
    this.priorityHeads.fill(-1);
    this.priorityTails.fill(-1);
    this.scheduledFree = new Uint32Array(schedulerCapacity);
    this.scheduledByKey = new FixedPageSlotMap(schedulerCapacity);
    this.pendingRecords = new Array(this.maxPendingPages);
    this.pendingActive = new Uint8Array(this.maxPendingPages);
    this.pendingFree = new Uint32Array(this.maxPendingPages);
    this.pendingByKey = new FixedPageSlotMap(this.maxPendingPages);
    for (let index = this.maxPendingPages - 1; index >= 0; index--) {
      this.pendingRecords[index] = {
        generation: 0,
        page: { path: '', mip: 0, x: 0, y: 0, pinned: false, cacheKey: 0 },
        lastSeen: 0,
        startedAt: 0,
        priorityTier: PRIORITY_LANE_COUNT - 1,
        controller: null,
        canceled: false,
      };
      this.pendingFree[this.pendingFreeTop++] = index;
    }
    this.readyUploads = new Array(this.maxPendingPages);
    for (let index = 0; index < this.readyUploads.length; index++) {
      const page = { path: '', mip: 0, x: 0, y: 0, pinned: false, cacheKey: 0 };
      this.readyUploads[index] = {
        key: 0, generation: 0, page, req: page, data: new Uint8Array(0), lease: null,
      };
    }
    this.feedbackScratch = new Array(schedulerCapacity + 1);
    this.feedbackScratchKeys = new FixedPageSlotMap(schedulerCapacity + 1);
    for (let index = schedulerCapacity; index >= 0; index--)
      this.feedbackScratch[index] = { path: '', mip: 0, x: 0, y: 0 };
    for (let index = schedulerCapacity - 1; index >= 0; index--) {
      this.scheduledRequests[index] = { path: '', mip: 0, x: 0, y: 0 };
      this.scheduledNext[index] = -1;
      this.scheduledPrevious[index] = -1;
      this.scheduledFree[this.scheduledFreeTop++] = index;
    }

    // Create the shared atlas texture.
    if (!isCompressedTextureFormat(this.format)) {
      const data = this.format === FORMAT_R16F
        ? new Uint16Array(this.cache.atlas.buffer)
        : this.cache.atlas;
      this.atlasTexture = new THREE.DataTexture(
        data,
        this.atlasWidth,
        this.atlasHeight,
        this.format === FORMAT_RGBA ? THREE.RGBAFormat : THREE.RedFormat,
        this.format === FORMAT_R16F ? THREE.HalfFloatType : THREE.UnsignedByteType,
      );
      (this.atlasTexture as THREE.DataTexture).minFilter = THREE.LinearFilter;
      (this.atlasTexture as THREE.DataTexture).magFilter = THREE.LinearFilter;
      (this.atlasTexture as THREE.DataTexture).generateMipmaps = false;
      this.atlasTexture.needsUpdate = true;
    } else {
      // Compressed (BC7/ASTC): use CompressedTexture + raw GPUTexture
      const tex = new THREE.CompressedTexture(
        [{ data: this.cache.atlas, width: this.atlasWidth, height: this.atlasHeight }],
        this.atlasWidth,
        this.atlasHeight,
        threeFormat(this.format),
      );
      tex.minFilter = THREE.LinearFilter;
      tex.magFilter = THREE.LinearFilter;
      tex.generateMipmaps = false;
      tex.needsUpdate = true;
      this.atlasTexture = tex;

      // The native texture is attached after Three.js creates it. Creating a
      // separate GPUTexture here would make shaders sample a stale allocation.
    }
  }

  /**
   * Attach the store to an initialized Three.js WebGPU renderer after at least
   * one render has caused its textures to be created. All later residency
   * changes become small GPU subregion writes.
   */
  attachRenderer(renderer: {
    backend: {
      device: GPUDevice;
      get(texture: THREE.Texture): { texture?: GPUTexture };
    };
  }): void {
    const atlas = renderer.backend.get(this.atlasTexture).texture;
    if (!atlas) throw new Error('VT atlas has not been initialized by Three.js; render once before attachRenderer()');
    this.device = renderer.backend.device;
    const limit = this.device.limits.maxTextureDimension2D;
    if (this.atlasWidth > limit || this.atlasHeight > limit)
      throw new RangeError(`VT atlas ${this.atlasWidth}x${this.atlasHeight} exceeds GPU limit ${limit}`);
    this.gpuAtlasTexture = atlas;
    this.gpuPageTables.clear();
    for (const [path, entry] of this.entries) {
      if (entry.pageTableLayout.width > limit || entry.pageTableLayout.height > limit)
        throw new RangeError(`VT page table ${path} exceeds GPU 2D texture limit ${limit}`);
      const texture = renderer.backend.get(entry.pageTableTexture).texture;
      // Three.js creates material textures lazily. Non-visible virtual textures
      // remain on the DataTexture upload path until a later attachRenderer().
      if (texture) this.gpuPageTables.set(path, texture);
    }
  }

  getLodBias(): number { return this.capacityLodBias; }
  getBudget(): number { return this.pageBudget; }

  /**
   * Load a virtual texture. Returns an AssetHandle pointing to the shared atlas.
   *
   * The handle's generation increments when pages are loaded/evicted for
   * this virtual texture. The pageDataProvider (set in constructor) reads
   * page data from the .big file and transcodes via the texture worker.
   *
   * @param path Asset name in the .big file
   * @param options Optional configuration (virtualSize overrides auto-detection)
   */
  loadTexture(
    path: string,
    options?: { width?: number; height?: number; mipTail?: boolean; textureId?: number },
  ): AssetHandle<THREE.Texture> {
    const width = options?.width ?? 4096;
    const height = options?.height ?? 4096;
    assertVirtualTextureDimensions(width, height);
    const pageGridX = Math.ceil(width / PAGE_SIZE);
    const pageGridY = Math.ceil(height / PAGE_SIZE);
    const pageTableLayout = createPackedPageTableLayout(pageGridX, pageGridY);
    const maxMip = pageTableLayout.maxMip;

    // Three.js cannot upload a custom integer mip chain reliably. Pack all
    // virtual mip tables vertically into mip zero of one r32uint texture.
    const pageTableData = new Uint32Array(pageTableLayout.storageWidth * pageTableLayout.height);
    const pageTable = new PageTable(pageTableLayout, pageTableData);
    this.pageTables.set(path, pageTable);
    const pageTableTexture = new THREE.DataTexture(
      pageTableData,
      pageTableLayout.storageWidth,
      pageTableLayout.height,
      THREE.RedIntegerFormat,
      THREE.UnsignedIntType,
    );
    pageTableTexture.minFilter = THREE.NearestFilter;
    pageTableTexture.magFilter = THREE.NearestFilter;
    pageTableTexture.generateMipmaps = false;
    pageTableTexture.needsUpdate = true;

    const textureId = options?.textureId ?? this.nextTextureId++;
    if (!Number.isInteger(textureId) || textureId < 1 || textureId > 0xffffffff ||
        this.entriesById[textureId])
      throw new Error('virtual texture ID is invalid, exhausted, or already registered');
    this.nextTextureId = Math.max(this.nextTextureId, textureId + 1);
    const entry: VirtualTextureEntry = {
      textureId,
      path,
      width,
      height,
      pageGridX,
      pageGridY,
      maxMip,
      textureMaxMip: Math.floor(Math.log2(Math.max(width, height))),
      tailFirstMip: options?.mipTail ? maxMip + 1 : null,
      tailEntry: 0,
      pageTableLayout,
      pageTable: pageTableData,
      pageTableTexture,
    };
    this.entries.set(path, entry);
    this.entriesById[textureId] = entry;
    this.pageTablesById[textureId] = pageTable;
    this.materialChannelsBySourceId[textureId] = null;

    // Pre-load pinned pages (coarsest mip levels)
    this.loadPinnedPages(path, entry);
    if (entry.tailFirstMip !== null) {
      this.loadPage(path, { mip: entry.tailFirstMip, x: 0, y: 0, tail: true }, pageTable, true);
    }

    // Create handle pointing to the shared atlas
    const handle = new AssetHandle<THREE.Texture>(path, this.atlasTexture);
    handle.generation++;
    handle.state = 'loading';
    return handle;
  }

  /**
   * Load a PBR texture set and link its channels to one albedo feedback stream.
   * All channels must use identical dimensions/page coordinates.
   */
  loadMaterialSet(
    paths: { albedo: string; normal?: string; masks?: string; roughness?: string; ao?: string; emissive?: string },
    options?: {
      width?: number;
      height?: number;
      mipTail?: boolean;
      mipBiases?: Readonly<Partial<VirtualMaterialMipBiases>>;
    },
  ): VirtualMaterialSet {
    const load = (path: string): VirtualTextureEntry => {
      if (!this.entries.has(path)) this.loadTexture(path, options);
      return this.entries.get(path)!;
    };
    const set: VirtualMaterialSet = {
      albedo: load(paths.albedo),
      normal: paths.normal ? load(paths.normal) : undefined,
      masks: paths.masks ? load(paths.masks) : undefined,
      roughness: paths.roughness ? load(paths.roughness) : undefined,
      ao: paths.ao ? load(paths.ao) : undefined,
      emissive: paths.emissive ? load(paths.emissive) : undefined,
    };
    this.linkMaterialSet(set, options?.mipBiases);
    return set;
  }

  /**
   * Expand one aligned albedo feedback stream into independently resident PBR
   * channels. Biases are non-negative integer mip offsets; channel priority is
   * albedo first, then normal/emissive, then scalar masks.
   */
  linkMaterialSet(
    set: VirtualMaterialSet,
    mipBiases: Readonly<Partial<VirtualMaterialMipBiases>> = {},
  ): void {
    if (set.masks && (set.roughness || set.ao))
      throw new Error('packed masks and separate roughness/AO paths are mutually exclusive');
    const biases: Readonly<VirtualMaterialMipBiases> = {
      ...DEFAULT_VIRTUAL_MATERIAL_MIP_BIASES,
      ...mipBiases,
    };
    for (const value of Object.values(biases)) {
      if (!Number.isInteger(value) || value < 0 || value > MAX_MIP)
        throw new RangeError('virtual material mip biases must be integers from 0 through 10');
    }
    const descriptors: Array<{
      entry: VirtualTextureEntry | undefined;
      mipBias: number;
      channelPriority: number;
    }> = [
      { entry: set.albedo, mipBias: biases.albedo, channelPriority: 0 },
      { entry: set.normal, mipBias: biases.normal, channelPriority: 1 },
      { entry: set.emissive, mipBias: biases.emissive, channelPriority: 1 },
      { entry: set.masks, mipBias: biases.masks, channelPriority: 2 },
      { entry: set.roughness, mipBias: biases.roughness, channelPriority: 2 },
      { entry: set.ao, mipBias: biases.ao, channelPriority: 2 },
    ];
    const channels = descriptors.filter(
      (descriptor): descriptor is {
        entry: VirtualTextureEntry;
        mipBias: number;
        channelPriority: number;
      } => descriptor.entry !== undefined,
    );
    if (channels.some(({ entry }) => entry.width !== set.albedo.width || entry.height !== set.albedo.height ||
        entry.pageGridX !== set.albedo.pageGridX || entry.pageGridY !== set.albedo.pageGridY ||
        entry.maxMip !== set.albedo.maxMip))
      throw new Error('material feedback channels must have identical page layouts');

    const merged: MaterialStreamChannel[] = [];
    const existing = this.materialChannelsBySourceId[set.albedo.textureId];
    if (existing) for (const channel of existing) merged.push({ ...channel });
    for (const descriptor of channels) {
      const channel = merged.find(candidate => candidate.textureId === descriptor.entry.textureId);
      if (channel) {
        channel.mipBias = Math.min(channel.mipBias, descriptor.mipBias);
        channel.channelPriority = Math.min(channel.channelPriority, descriptor.channelPriority);
      } else {
        merged.push({
          textureId: descriptor.entry.textureId,
          mipBias: descriptor.mipBias,
          channelPriority: descriptor.channelPriority,
        });
      }
    }
    merged.sort((a, b) => a.channelPriority - b.channelPriority || a.textureId - b.textureId);
    this.materialChannelsBySourceId[set.albedo.textureId] = merged;
  }

  /** Pre-load pinned (coarsest) pages. */
  private loadPinnedPages(path: string, entry: VirtualTextureEntry) {
    const pageTable = this.pageTables.get(path)!;
    const pinnedMips = entry.maxMip === 0 ? [0] : [entry.maxMip, entry.maxMip - 1];
    for (const mip of pinnedMips) {
      const grid = pageGridAtMip(entry.pageTableLayout, mip);
      for (let y = 0; y < grid.height; y++) {
        for (let x = 0; x < grid.width; x++) {
          const req: PageRequest = { mip, x, y };
          this.loadPage(path, req, pageTable, true);
        }
      }
    }
  }

  private pageKey(req: VirtualPageRequest): number {
    return req.textureId ? packedPageIdentity(req.textureId, req) : 0;
  }

  private isValidRequest(path: string, req: PageRequest): boolean {
    const entry = this.entries.get(path);
    return entry ? this.isValidEntryRequest(entry, req) : false;
  }

  private isValidEntryRequest(entry: VirtualTextureEntry, req: PageRequest): boolean {
    if (!Number.isInteger(req.mip) || !Number.isInteger(req.x) || !Number.isInteger(req.y)) return false;
    if (req.tail) return entry.tailFirstMip === req.mip && req.x === 0 && req.y === 0;
    if (req.mip < 0 || req.mip > entry.maxMip) return false;
    const grid = pageGridAtMip(entry.pageTableLayout, req.mip);
    return req.x >= 0 && req.y >= 0 && req.x < grid.width && req.y < grid.height;
  }

  private isRequestResident(path: string, req: PageRequest, pageTable: PageTable): boolean {
    if (req.tail) return isResident(this.entries.get(path)?.tailEntry ?? 0);
    return pageTable.isResident(req);
  }

  private getPending(key: number): PendingPageRecord | undefined {
    const slot = this.pendingByKey.get(key);
    return slot === undefined || this.pendingActive[slot] === 0 ? undefined : this.pendingRecords[slot];
  }

  private deletePending(key: number): PendingPageRecord | undefined {
    const slot = this.pendingByKey.get(key);
    if (slot === undefined || this.pendingActive[slot] === 0) return undefined;
    const pending = this.pendingRecords[slot];
    this.pendingByKey.delete(key);
    this.pendingActive[slot] = 0;
    this.pendingFree[this.pendingFreeTop++] = slot;
    this.pendingCount--;
    this.pendingBytes -= this.cache.slotDataSize;
    return pending;
  }

  /** Cancel one strictly worse non-pinned load so urgent visible work can enter. */
  private preemptWorstPending(priorityTier: number): boolean {
    let candidate = -1;
    let worstTier = priorityTier;
    for (let slot = 0; slot < this.pendingRecords.length; slot++) {
      if (this.pendingActive[slot] === 0) continue;
      const pending = this.pendingRecords[slot];
      if (pending.page.pinned || pending.canceled || pending.priorityTier <= worstTier) continue;
      candidate = slot;
      worstTier = pending.priorityTier;
    }
    if (candidate < 0) return false;
    const pending = this.pendingRecords[candidate];
    pending.canceled = true;
    pending.controller?.abort('VT load preempted by a higher-priority visible page');
    // Keep the fixed slot occupied until the asynchronous stage acknowledges
    // cancellation; immediate reuse can overrun its equally bounded task ring.
    this.priorityPreemptions++;
    return true;
  }

  /** Queue one deduplicated load. Physical residency is acquired only when data is ready. */
  private queuePageLoad(
    entry: VirtualTextureEntry,
    req: PageRequest,
    pageTable: PageTable,
    pinned = false,
    priorityTier = PRIORITY_LANE_COUNT - 1,
  ): boolean {
    const path = entry.path;
    if (!this.pageDataProvider || (req.tail ? isResident(entry.tailEntry) : pageTable.isResident(req)) ||
        !this.isValidEntryRequest(entry, req)) return false;
    if (this.pendingCount >= this.maxPendingPages ||
        this.pendingBytes + this.cache.slotDataSize > this.maxPendingBytes) {
      if (!this.preemptWorstPending(priorityTier)) this.rejectedAdmissions++;
      return false;
    }
    const key = packedPageIdentity(entry.textureId, req);
    if (this.pendingByKey.get(key) !== undefined) return false;

    const generation = ++this.loadGeneration;
    const controller = new AbortController();
    const slot = this.pendingFree[--this.pendingFreeTop];
    const pendingRecord = this.pendingRecords[slot];
    const page = pendingRecord.page;
    page.textureId = entry.textureId;
    page.path = path;
    page.mip = req.mip;
    page.x = req.x;
    page.y = req.y;
    page.tail = req.tail;
    page.batchTier = req.batchTier;
    page.pinned = pinned;
    page.cacheKey = key;
    pendingRecord.generation = generation;
    pendingRecord.lastSeen = this.feedbackEpoch;
    pendingRecord.startedAt = performance.now();
    pendingRecord.priorityTier = priorityTier;
    pendingRecord.controller = controller;
    pendingRecord.canceled = false;
    this.pendingActive[slot] = 1;
    this.pendingByKey.set(key, slot);
    this.pendingCount++;
    this.pendingBytes += this.cache.slotDataSize;
    this.telemetry?.metrics.counterAdd(EngineMetric.VtPagesRequested, 1);
    this.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.VtPageLoad, key, this.cache.slotDataSize, priorityTier,
    );
    // `req` may be a reusable scheduler scratch record. `page` is the owned
    // immutable copy retained for this asynchronous generation.
    this.pageDataProvider(path, page, controller.signal).then(result => {
      const lease = result instanceof Uint8Array ? null : result;
      const data = result instanceof Uint8Array ? result : result.bytes;
      if (data.byteLength !== this.cache.slotDataSize) {
        lease?.release();
        throw new RangeError(`VT page ${key} has ${data.byteLength} bytes; expected ${this.cache.slotDataSize}`);
      }
      const pending = this.getPending(key);
      if (!pending || pending.generation !== generation) {
        lease?.release();
        return;
      }
      if (pending.canceled) {
        lease?.release();
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.VtPageLoad, key, 0, 2);
        this.deletePending(key);
        return;
      }
      const loadMs = performance.now() - pending.startedAt;
      this.completedLoads++;
      this.totalLoadMs += loadMs;
      this.maxLoadMs = Math.max(this.maxLoadMs, loadMs);
      if (!pending.page.pinned && this.feedbackEpoch - pending.lastSeen >= this.staleFeedbackEpochs) {
        lease?.release();
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.VtPageLoad, key, 0, 3);
        this.deletePending(key);
        this.staleCancellations++;
        return;
      }
      // Completion can happen at any point between frames. Defer atlas and
      // page-table writes to poll() so a fast worker cannot burst dozens of
      // GPU queue submissions into one presentation interval.
      if (this.readyUploadCount >= this.readyUploads.length) {
        lease?.release();
        throw new Error('VT ready-upload ring capacity exceeded');
      }
      const ready = this.readyUploads[this.readyUploadTail]!;
      ready.key = key;
      ready.generation = generation;
      ready.page = page;
      ready.req = page;
      ready.data = data;
      ready.lease = lease;
      this.readyUploadTail = (this.readyUploadTail + 1) % this.readyUploads.length;
      this.readyUploadCount++;
      this.telemetry?.metrics.counterAdd(EngineMetric.VtPagesLoaded, 1);
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.VtPageLoad, key, data.byteLength, 0,
      );
    }).catch(error => {
      const pending = this.getPending(key);
      const canceled = controller.signal.aborted ||
        (pending?.generation === generation && pending.canceled);
      if (pending?.generation === generation) this.deletePending(key);
      if (canceled) {
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.VtPageLoad, key, 0, 2);
        return;
      }
      this.telemetry?.metrics.counterAdd(EngineMetric.VtPagesFailed, 1);
      this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.VtPageLoad, key, 0, 1);
      this.failedLoads++;
      const message = error instanceof Error ? error.message : String(error);
      console.error(`[VT] Failed to load page ${path} mip=${page.mip} (${page.x},${page.y}): ${message}`);
    });
    return true;
  }

  /** Load a single page asynchronously (used for pinned startup pages). */
  private loadPage(path: string, req: PageRequest, pageTable: PageTable, pinned = false): void {
    const entry = this.entries.get(path);
    if (!entry) return;
    if (this.queuePageLoad(entry, req, pageTable, pinned)) return;
    if (pinned && !this.isRequestResident(path, req, pageTable)) {
      this.rememberRequest({
        textureId: entry.textureId,
        path,
        mip: req.mip,
        x: req.x,
        y: req.y,
        tail: req.tail,
        batchTier: 'urgent',
        pinned: true,
        screenPriority: 0,
        coverage: 0xffff,
        channelPriority: 0,
        priorityTier: 0,
      });
    }
  }

  // @hot-no-alloc-begin VirtualTextureStore.addFeedbackRequest
  private residentMipGap(
    entry: VirtualTextureEntry,
    pageTable: PageTable,
    mip: number,
    x: number,
    y: number,
    tail: boolean,
  ): number {
    if (tail) return 0;
    for (let fallbackMip = mip; fallbackMip <= entry.maxMip; fallbackMip++) {
      const shift = fallbackMip - mip;
      if (pageTable.isResidentAt(fallbackMip, x >> shift, y >> shift))
        return Math.min(7, fallbackMip - mip);
    }
    const tailMip = entry.tailFirstMip ?? entry.maxMip + 1;
    return Math.min(7, Math.max(0, tailMip - mip));
  }

  private addFeedbackPage(
    textureId: number,
    entry: VirtualTextureEntry,
    source: VirtualPageRequest,
    mip: number,
    x: number,
    y: number,
    tail: boolean,
    capacity: number,
    channelPriority: number,
    parent: boolean,
  ): boolean {
    const key = packedPageCoordinates(entry.textureId, mip, x, y, tail);
    const sourceCoverage = source.coverage ?? 1;
    const sourceWeight = sourcePerceptualWeight(source);
    const residentMipGap = this.residentMipGap(
      entry, this.pageTablesById[textureId]!, mip, x, y, tail,
    );
    const existing = this.feedbackScratchKeys.get(key);
    if (existing !== undefined) {
      const request = this.feedbackScratch[existing];
      request.screenPriority = Math.min(request.screenPriority ?? 255, source.screenPriority ?? 255);
      request.coverage = Math.min(0xffff, (request.coverage ?? 1) + sourceCoverage);
      request.perceptualWeight = Math.min(
        MAX_PERCEPTUAL_WEIGHT,
        (request.perceptualWeight ?? 1) + sourceWeight,
      );
      request.residentMipGap = Math.max(request.residentMipGap ?? 0, residentMipGap);
      request.channelPriority = Math.min(request.channelPriority ?? channelPriority, channelPriority);
      request.priorityTier = perceptualPriority(
        request.perceptualWeight,
        request.coverage,
        request.residentMipGap,
        parent,
        request.channelPriority,
      );
      request.batchTier = pageBatchTier(parent, request.priorityTier);
      return true;
    }
    if (this.feedbackScratchCount >= capacity) return false;
    const index = this.feedbackScratchCount++;
    const request = this.feedbackScratch[index];
    request.textureId = textureId;
    request.path = entry.path;
    request.mip = mip;
    request.x = x;
    request.y = y;
    request.tail = tail ? true : undefined;
    request.screenPriority = source.screenPriority ?? 255;
    request.coverage = sourceCoverage;
    request.perceptualWeight = sourceWeight;
    request.residentMipGap = residentMipGap;
    request.channelPriority = channelPriority;
    request.priorityTier = perceptualPriority(
      sourceWeight, sourceCoverage, residentMipGap, parent, channelPriority,
    );
    request.batchTier = pageBatchTier(parent, request.priorityTier);
    this.feedbackScratchKeys.set(key, index);
    return true;
  }

  private addFeedbackRequest(
    textureId: number,
    source: VirtualPageRequest,
    bias: number,
    capacity: number,
    channelPriority: number,
  ): boolean {
    const entry = this.entriesById[textureId];
    const pageTable = this.pageTablesById[textureId];
    if (!entry || !pageTable || !this.isValidEntryRequest(entry, source)) return true;
    if (source.tail === true)
      return this.addFeedbackPage(
        textureId, entry, source, source.mip, 0, 0, true,
        capacity, channelPriority, true,
      );

    const desiredMip = Math.min(entry.maxMip, source.mip + bias);
    const desiredX = source.x >> (desiredMip - source.mip);
    const desiredY = source.y >> (desiredMip - source.mip);
    if (pageTable.isResidentAt(desiredMip, desiredX, desiredY))
      return this.addFeedbackPage(
        textureId, entry, source, desiredMip, desiredX, desiredY, false,
        capacity, channelPriority, false,
      );

    // Restore one explicitly low-quality parent quickly, then promote straight
    // to the exact requested page through the bounded focus/peripheral window. Existing
    // resident coarser/tail pages remain immediately sampleable in the shader.
    const urgentMip = Math.min(entry.maxMip, desiredMip + 2);
    const urgentShift = urgentMip - desiredMip;
    const urgentX = desiredX >> urgentShift;
    const urgentY = desiredY >> urgentShift;
    if (!pageTable.isResidentAt(urgentMip, urgentX, urgentY) &&
        !this.addFeedbackPage(
          textureId, entry, source, urgentMip, urgentX, urgentY, false,
          capacity, channelPriority, true,
        )) return false;
    if (urgentMip === desiredMip) return true;
    return this.addFeedbackPage(
      textureId, entry, source, desiredMip, desiredX, desiredY, false,
      capacity, channelPriority, false,
    );
  }
  // @hot-no-alloc-end VirtualTextureStore.addFeedbackRequest

  /** Expand, deduplicate, and coarsen into preallocated feedback records. */
  // @hot-no-alloc-begin VirtualTextureStore.buildEffectiveFeedback
  private buildEffectiveFeedback(feedback: ReadonlyMap<unknown, VirtualPageRequest>): number {
    const capacity = Math.max(1, this.cache.totalSlots - this.cache.pinnedSlots);
    for (let bias = 0; bias <= MAX_MIP; bias++) {
      this.feedbackScratchKeys.clear();
      this.feedbackScratchCount = 0;
      let fits = true;
      outer: for (const source of feedback.values()) {
        const textureId = source.textureId ?? this.entries.get(source.path)?.textureId ?? 0;
        const channels = this.materialChannelsBySourceId[textureId];
        if (channels) {
          for (const channel of channels) {
            if (!this.addFeedbackRequest(
              channel.textureId, source, bias + channel.mipBias, capacity, channel.channelPriority,
            )) {
              fits = false;
              break outer;
            }
          }
        } else if (!this.addFeedbackRequest(textureId, source, bias, capacity, 0)) {
          fits = false;
          break;
        }
      }
      if (fits || bias === MAX_MIP) {
        this.capacityLodBias = bias;
        if (!fits) this.schedulerOverflows++;
        return this.feedbackScratchCount;
      }
    }
    return 0;
  }
  // @hot-no-alloc-end VirtualTextureStore.buildEffectiveFeedback

  /** Atomic multi-pass variant: all maps form one visibility epoch. */
  // @hot-no-alloc-begin VirtualTextureStore.buildEffectiveFeedbackBatch
  private buildEffectiveFeedbackBatch(
    feedbackMaps: ReadonlyArray<ReadonlyMap<unknown, VirtualPageRequest> | null>,
    mapCount: number,
  ): number {
    const capacity = Math.max(1, this.cache.totalSlots - this.cache.pinnedSlots);
    const count = Math.min(mapCount, feedbackMaps.length);
    for (let bias = 0; bias <= MAX_MIP; bias++) {
      this.feedbackScratchKeys.clear();
      this.feedbackScratchCount = 0;
      let fits = true;
      outer: for (let mapIndex = 0; mapIndex < count; mapIndex++) {
        const feedback = feedbackMaps[mapIndex];
        if (feedback === null || feedback === undefined) continue;
        for (const source of feedback.values()) {
          const textureId = source.textureId ?? this.entries.get(source.path)?.textureId ?? 0;
          const channels = this.materialChannelsBySourceId[textureId];
          if (channels) {
            for (const channel of channels) {
              if (!this.addFeedbackRequest(
                channel.textureId, source, bias + channel.mipBias, capacity, channel.channelPriority,
              )) {
                fits = false;
                break outer;
              }
            }
          } else if (!this.addFeedbackRequest(textureId, source, bias, capacity, 0)) {
            fits = false;
            break outer;
          }
        }
      }
      if (fits || bias === MAX_MIP) {
        this.capacityLodBias = bias;
        if (!fits) this.schedulerOverflows++;
        return this.feedbackScratchCount;
      }
    }
    return 0;
  }
  // @hot-no-alloc-end VirtualTextureStore.buildEffectiveFeedbackBatch

  // @hot-no-alloc-begin VirtualTextureStore.schedulePriority
  private copyRequest(target: VirtualPageRequest, source: VirtualPageRequest): void {
    target.textureId = source.textureId;
    target.path = source.path;
    target.mip = source.mip;
    target.x = source.x;
    target.y = source.y;
    target.tail = source.tail;
    target.screenPriority = source.screenPriority;
    target.coverage = source.coverage;
    target.perceptualWeight = source.perceptualWeight;
    target.residentMipGap = source.residentMipGap;
    target.channelPriority = source.channelPriority;
    target.priorityTier = source.priorityTier;
    target.batchTier = source.batchTier;
    target.pinned = source.pinned;
  }

  private linkScheduledTail(index: number, priority: number): void {
    const tail = this.priorityTails[priority];
    this.scheduledPriority[index] = priority;
    this.scheduledPrevious[index] = tail;
    this.scheduledNext[index] = -1;
    if (tail < 0) this.priorityHeads[priority] = index;
    else this.scheduledNext[tail] = index;
    this.priorityTails[priority] = index;
  }

  private unlinkScheduled(index: number): void {
    const priority = this.scheduledPriority[index];
    const previous = this.scheduledPrevious[index];
    const next = this.scheduledNext[index];
    if (previous < 0) this.priorityHeads[priority] = next;
    else this.scheduledNext[previous] = next;
    if (next < 0) this.priorityTails[priority] = previous;
    else this.scheduledPrevious[next] = previous;
    this.scheduledPrevious[index] = -1;
    this.scheduledNext[index] = -1;
  }

  private moveScheduled(index: number, priority: number): void {
    if (this.scheduledPriority[index] === priority) return;
    this.unlinkScheduled(index);
    this.linkScheduledTail(index, priority);
  }

  private removeScheduled(index: number, status: SchedulerWaitStatus): void {
    if (this.scheduledActive[index] === 0) return;
    if (this.scheduledTraceActive[index] !== 0) {
      this.telemetry?.trace.asyncEnd(
        EngineTraceDescriptor.VtSchedulerWait,
        this.scheduledKeys[index],
        this.scheduledPriority[index],
        status,
      );
      this.scheduledTraceActive[index] = 0;
    }
    this.unlinkScheduled(index);
    this.scheduledByKey.delete(this.scheduledKeys[index]);
    this.scheduledActive[index] = 0;
    this.scheduledFree[this.scheduledFreeTop++] = index;
    this.scheduledCount--;
  }

  private rememberRequest(request: VirtualPageRequest): void {
    const textureId = request.textureId ?? 0;
    const entry = this.entriesById[textureId];
    const pageTable = this.pageTablesById[textureId];
    if (!entry || !pageTable || !this.isValidEntryRequest(entry, request)) return;
    const key = this.pageKey(request);
    if (request.tail ? isResident(entry.tailEntry) : pageTable.isResident(request)) {
      this.residentHits++;
      this.cache.touch(key);
      const scheduled = this.scheduledByKey.get(key);
      if (scheduled !== undefined) this.removeScheduled(scheduled, SchedulerWaitStatus.Resident);
      return;
    }
    this.residentMisses++;
    const pending = this.getPending(key);
    if (pending) {
      pending.lastSeen = this.feedbackEpoch;
      pending.priorityTier = request.priorityTier ?? PRIORITY_LANE_COUNT - 1;
      return;
    }
    const existing = this.scheduledByKey.get(key);
    if (existing !== undefined) {
      this.copyRequest(this.scheduledRequests[existing], request);
      this.moveScheduled(existing, request.priorityTier ?? PRIORITY_LANE_COUNT - 1);
      this.scheduledLastSeen[existing] = this.feedbackEpoch;
      return;
    }
    if (this.scheduledFreeTop === 0) {
      this.schedulerOverflows++;
      return;
    }
    const index = this.scheduledFree[--this.scheduledFreeTop];
    this.scheduledKeys[index] = key;
    this.copyRequest(this.scheduledRequests[index], request);
    this.scheduledActive[index] = 1;
    this.scheduledLastSeen[index] = this.feedbackEpoch;
    this.linkScheduledTail(index, request.priorityTier ?? PRIORITY_LANE_COUNT - 1);
    this.scheduledByKey.set(key, index);
    this.scheduledCount++;
    const priority = request.priorityTier ?? PRIORITY_LANE_COUNT - 1;
    this.telemetry?.trace.instant(
      EngineTraceDescriptor.VtFeedbackDetected, key, priority, this.feedbackEpoch,
    );
    this.scheduledTraceActive[index] = this.telemetry?.trace.asyncBegin(
      EngineTraceDescriptor.VtSchedulerWait, key, priority, 0,
    ) === TelemetryRecordStatus.Recorded ? 1 : 0;
  }

  /** Advance persistent requests through fixed priority lanes under hard budgets. */
  private schedulePendingRequests(): void {
    const operationBudget = this.debugPaused ? 0 : (this.debugPageBudget ?? this.pageBudget);
    if (operationBudget === 0 || this.scheduledCount === 0) return;
    const deadline = performance.now() + this.scheduleBudgetMs;
    const inspectionLimit = this.scheduledCount;
    let loaded = 0;
    let inspected = 0;

    while (inspected < inspectionLimit && loaded < operationBudget) {
      if ((inspected & 3) === 0 && performance.now() >= deadline) {
        this.scheduleBudgetExhaustions++;
        break;
      }
      let priority = 0;
      while (priority < this.priorityHeads.length && this.priorityHeads[priority] < 0) priority++;
      if (priority === this.priorityHeads.length) break;
      const index = this.priorityHeads[priority];
      inspected++;
      const request = this.scheduledRequests[index];
      if (!request.pinned && this.scheduledLastSeen[index] < this.feedbackEpoch &&
          priority !== PRIORITY_LANE_COUNT - 1) {
        this.moveScheduled(index, PRIORITY_LANE_COUNT - 1);
        continue;
      }
      if (!request.pinned &&
          this.feedbackEpoch - this.scheduledLastSeen[index] >= this.staleFeedbackEpochs) {
        this.removeScheduled(index, SchedulerWaitStatus.Stale);
        this.staleCancellations++;
        continue;
      }
      const textureId = request.textureId ?? 0;
      const entry = this.entriesById[textureId];
      const pageTable = this.pageTablesById[textureId];
      if (!entry || !pageTable) {
        this.removeScheduled(index, SchedulerWaitStatus.Invalid);
        continue;
      }
      if (request.tail ? isResident(entry.tailEntry) : pageTable.isResident(request)) {
        this.removeScheduled(index, SchedulerWaitStatus.Resident);
        continue;
      }
      this.cache.touch(this.scheduledKeys[index]);
      if (this.queuePageLoad(entry, request, pageTable, request.pinned === true, priority)) { // @alloc-allowed reason=AssetFetch
        this.removeScheduled(index, SchedulerWaitStatus.Admitted);
        loaded++;
      } else {
        // Capacity or its downstream worker stage is still occupied. Keep this
        // highest-priority request at the head and retry next poll.
        break;
      }
    }
  }
  // @hot-no-alloc-end VirtualTextureStore.schedulePriority

  // @hot-no-alloc-begin VirtualTextureStore.mergeFeedback
  private cancelStalePending(): void {
    for (let slot = 0; slot < this.pendingRecords.length; slot++) {
      if (this.pendingActive[slot] === 0) continue;
      const pending = this.pendingRecords[slot];
      if (pending.page.pinned || pending.canceled) continue;
      if (pending.lastSeen < this.feedbackEpoch)
        pending.priorityTier = PRIORITY_LANE_COUNT - 1;
      if (this.feedbackEpoch - pending.lastSeen < this.staleFeedbackEpochs) continue;
      pending.canceled = true;
      pending.controller?.abort('VT request left the visibility window');
      // The provider/cancellation continuation owns final slot release.
      this.staleCancellations++;
    }
  }

  /** Merge one feedback frame into the persistent bounded request scheduler. */
  processFeedback(feedback: ReadonlyMap<unknown, VirtualPageRequest>) {
    this.feedbackEpoch++;
    return this.commitEffectiveFeedback(this.buildEffectiveFeedback(feedback));
  }

  /** Commit several completed feedback passes as one visibility epoch. */
  processFeedbackBatch(
    feedbackMaps: ReadonlyArray<ReadonlyMap<unknown, VirtualPageRequest> | null>,
    mapCount: number,
  ) {
    this.feedbackEpoch++;
    return this.commitEffectiveFeedback(this.buildEffectiveFeedbackBatch(feedbackMaps, mapCount));
  }

  private commitEffectiveFeedback(effectiveCount: number) {
    for (let index = 0; index < effectiveCount; index++)
      this.rememberRequest(this.feedbackScratch[index]);
    this.cancelStalePending();
    const result = this.feedbackResult;
    result.loaded = 0;
    result.evicted = 0;
    result.totalRequests = effectiveCount;
    result.queuedRequests = this.scheduledCount;
    result.lodBias = this.capacityLodBias;
    return result;
  }
  // @hot-no-alloc-end VirtualTextureStore.mergeFeedback

  private clearEvictedPage(page: CachedPage): void {
    if (page.tail) {
      this.writeMipTailEntry(page.path, 0);
      return;
    }
    this.pageTables.get(page.path)?.setEvicted(page);
    this.writePageTableEntry(page.path, page, 0);
  }

  /** Evict only the selected physical page; material channels are independent. */
  private evictPage(page: CachedPage): void {
    this.clearEvictedPage(page);
  }

  private writeMipTailEntry(path: string, value: number): void {
    const entry = this.entries.get(path);
    if (!entry) return;
    const index = packedMipTailIndex(entry.pageTableLayout);
    entry.tailEntry = value;
    entry.pageTable[index] = value;
    const gpuTexture = this.gpuPageTables.get(path);
    if (gpuTexture && this.device) {
      const mipOffset = entry.pageTableLayout.mipOffsets[entry.maxMip];
      if (mipOffset === undefined) throw new RangeError('mip tail offset is outside the packed page table');
      this.pageTableUploadScratch[0] = value;
      this.device.queue.writeTexture(
        { texture: gpuTexture, origin: { x: 1, y: mipOffset } },
        this.pageTableUploadScratch, {},
        { width: 1, height: 1, depthOrArrayLayers: 1 },
      );
    } else {
      entry.pageTableTexture.needsUpdate = true;
    }
  }

  private updateMipTailEntry(path: string, slot: PageSlot): void {
    this.writeMipTailEntry(path, packEntry(true, slot.x, slot.y));
  }

  private writePageTableEntry(path: string, req: PageRequest, value: number): void {
    const entry = this.entries.get(path);
    if (!entry) return;
    const idx = packedPageTableIndex(entry.pageTableLayout, req.mip, req.x, req.y);
    entry.pageTable[idx] = value;
    const gpuTexture = this.gpuPageTables.get(path);
    if (gpuTexture && this.device) {
      const mipOffset = entry.pageTableLayout.mipOffsets[req.mip];
      if (mipOffset === undefined) throw new RangeError('page mip is outside the packed page table');
      this.pageTableUploadScratch[0] = value;
      this.device.queue.writeTexture(
        {
          texture: gpuTexture,
          origin: { x: req.x, y: mipOffset + req.y },
        },
        this.pageTableUploadScratch,
        {},
        { width: 1, height: 1, depthOrArrayLayers: 1 },
      );
    } else {
      entry.pageTableTexture.needsUpdate = true;
    }
  }

  /** Mark a page resident in its packed CPU and GPU page tables. */
  private updatePageTableTexture(path: string, req: PageRequest, slot: PageSlot) {
    this.writePageTableEntry(path, req, packEntry(true, slot.x, slot.y));
  }

  /**
   * Cancel pending work and release all residency owned by a virtual texture.
   * Late promises are ignored through their removed generation records.
   */
  unloadTexture(path: string): void {
    const unloading = this.entries.get(path);
    if (unloading) {
      for (let sourceId = 1; sourceId < this.materialChannelsBySourceId.length; sourceId++) {
        const channels = this.materialChannelsBySourceId[sourceId];
        if (sourceId === unloading.textureId ||
            channels?.some(channel => channel.textureId === unloading.textureId))
          this.materialChannelsBySourceId[sourceId] = null;
      }
    }
    for (let index = 0; index < this.scheduledRequests.length; index++) {
      if (this.scheduledActive[index] !== 0 && this.scheduledRequests[index].path === path)
        this.removeScheduled(index, SchedulerWaitStatus.Unloaded);
    }
    for (let slot = 0; slot < this.pendingRecords.length; slot++) {
      if (this.pendingActive[slot] === 0) continue;
      const pending = this.pendingRecords[slot];
      if (pending.page.path !== path) continue;
      pending.canceled = true;
      pending.controller?.abort('virtual texture unloaded');
      this.deletePending(pending.page.cacheKey);
    }
    this.cache.removeTexture(path);
    const entry = this.entries.get(path);
    if (entry) {
      this.entriesById[entry.textureId] = null;
      this.pageTablesById[entry.textureId] = null;
      this.materialChannelsBySourceId[entry.textureId] = null;
      entry.pageTableTexture.dispose();
    }
    this.entries.delete(path);
    this.pageTables.delete(path);
    this.gpuPageTables.delete(path);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const path of this.entries.keys()) this.unloadTexture(path);
    this.atlasTexture.dispose();
    this.gpuPageTables.clear();
    this.gpuAtlasTexture = null;
    this.device = null;
    for (let index = 0; index < this.readyUploads.length; index++)
      this.releaseReadyPayload(this.readyUploads[index]!);
    this.readyUploadCount = 0;
    this.readyUploadHead = 0;
    this.readyUploadTail = 0;
  }

  /** Get a virtual texture entry by path. */
  getEntry(path: string): VirtualTextureEntry | undefined {
    return this.entries.get(path);
  }

  /** Resolve the stable ID emitted by the RG32Uint feedback pass. */
  getEntryById(textureId: number): VirtualTextureEntry | undefined {
    return this.entriesById[textureId >>> 0] ?? undefined;
  }

  /**
   * Atomically refresh an already-resident page in its existing physical slot.
   * Nonresident pages remain source-backed and will observe the new content on
   * their next ordinary demand. Callers must bound generation and invocation.
   */
  replaceResidentPage(path: string, request: Readonly<PageRequest>, data: Uint8Array): boolean {
    const entry = this.entries.get(path);
    if (!entry) return false;
    const key = packedPageIdentity(entry.textureId, request);
    const slot = this.cache.replaceByKey(key, data);
    if (!slot) return false;
    this.writePage(slot, data);
    this.completedUploads++;
    return true;
  }

  isPagePending(path: string, request: Readonly<PageRequest>): boolean {
    const entry = this.entries.get(path);
    return entry ? this.pendingByKey.get(packedPageIdentity(entry.textureId, request)) !== undefined : false;
  }

  /**
   * Write page data to the atlas at a specific slot.
   *
   * For compressed formats (BC7/ASTC): uses device.queue.writeTexture()
   * to write compressed blocks directly to the GPU texture.
   * For RGBA: updates the DataTexture's data array + sets needsUpdate.
   */
  writePage(slot: PageSlot, data: Uint8Array) {
    if (this.gpuAtlasTexture && this.device) {
      if (!(data.buffer instanceof ArrayBuffer))
        throw new TypeError('atlas uploads require page bytes backed by an owned ArrayBuffer');
      const uploadData = data as Uint8Array<ArrayBuffer>;
      // Compressed: write directly to GPUTexture via writeTexture
      // [SHLOM] uses gl.texSubImage2D / UpdateSubregion — WebGPU equivalent
      this.device.queue.writeTexture(
        {
          texture: this.gpuAtlasTexture,
          origin: { x: slot.x * SLOT_SIZE, y: slot.y * SLOT_SIZE },
        },
        uploadData,
        isCompressedTextureFormat(this.format)
          ? { bytesPerRow: this.cache.slotBytesPerRow, rowsPerImage: SLOT_BLOCKS_Y }
          : {
              bytesPerRow: SLOT_SIZE * uncompressedBytesPerTexel(this.format),
              rowsPerImage: SLOT_SIZE,
            },
        { width: SLOT_SIZE, height: SLOT_SIZE },
      );
    } else {
      // Startup only. Once attachRenderer() succeeds, both compressed and RGBA
      // pages use the same queue.writeTexture subregion path above.
      this.atlasTexture.needsUpdate = true;
    }
  }

  private releaseReadyPayload(ready: ReadyPageUpload): void {
    ready.lease?.release();
    ready.lease = null;
  }

  /** Frame whose later render pass can first sample pages published by poll(). */
  setPublicationFrameId(frameId: number): void {
    this.publicationFrameId = Number.isSafeInteger(frameId) && frameId >= 0 ? frameId : 0;
  }

  /** Feed the presentation interval into the central bounded upload policy. */
  // @hot-no-alloc-begin VirtualTextureStore.recordFrameTime
  recordFrameTime(frameMs: number): void {
    this.tuning.recordFrameTime(frameMs, this.pendingCount + this.readyUploadCount + this.scheduledCount);
  }
  // @hot-no-alloc-end VirtualTextureStore.recordFrameTime

  /** Poll workers and commit a bounded number of completed uploads per frame. */
  poll() {
    const deadline = performance.now() + this.tuning.uploadBudgetMs;
    for (let count = 0; count < this.tuning.uploadsPerPoll && this.readyUploadCount !== 0; count++) {
      if (count !== 0 && performance.now() >= deadline) {
        this.uploadBudgetExhaustions++;
        break;
      }
      const ready = this.readyUploads[this.readyUploadHead]!;
      this.readyUploadHead = (this.readyUploadHead + 1) % this.readyUploads.length;
      this.readyUploadCount--;
      const pending = this.getPending(ready.key);
      if (!pending || pending.generation !== ready.generation) {
        this.releaseReadyPayload(ready);
        continue;
      }
      if (pending.canceled) {
        this.deletePending(ready.key);
        this.releaseReadyPayload(ready);
        continue;
      }
      this.deletePending(ready.key);
      const textureId = ready.page.textureId ?? 0;
      const entry = this.entriesById[textureId];
      const pageTable = this.pageTablesById[textureId];
      if (!entry || !pageTable || (ready.req.tail ? isResident(entry.tailEntry) : pageTable.isResident(ready.req))) {
        this.releaseReadyPayload(ready);
        continue;
      }

      const uploadStartedAt = performance.now();
      this.telemetry?.trace.spanBegin(
        EngineTraceDescriptor.VtUpload, ready.key, ready.data.byteLength, 0,
      );
      let slot: PageSlot;
      try {
        const acquired = this.cache.acquire(ready.page);
        slot = acquired.slot;
        if (acquired.evicted) {
          this.evictPage(acquired.evicted);
          this.residentEvictions++;
        }
      } catch {
        this.telemetry?.trace.spanEnd(EngineTraceDescriptor.VtUpload, ready.key, 0, 1);
        this.rejectedAdmissions++;
        this.releaseReadyPayload(ready);
        continue;
      }

      this.cache.commit(ready.page, slot, ready.data);
      this.writePage(slot, ready.data);
      if (ready.req.tail) {
        this.updateMipTailEntry(ready.page.path, slot);
      } else {
        pageTable.setResident(ready.req, slot);
        this.updatePageTableTexture(ready.page.path, ready.req, slot);
      }
      const uploadMs = performance.now() - uploadStartedAt;
      const physicalSlot = slot.y * this.atlasPagesX + slot.x;
      this.telemetry?.trace.instant(
        EngineTraceDescriptor.VtPagePublished,
        ready.key,
        physicalSlot,
        this.publicationFrameId,
      );
      this.telemetry?.trace.spanEnd(
        EngineTraceDescriptor.VtUpload, ready.key, ready.data.byteLength, physicalSlot,
      );
      this.telemetry?.metrics.histogramLog2(
        EngineMetric.VtUploadNs, Math.max(1, Math.floor(uploadMs * 1_000_000)),
      );
      this.completedUploads++;
      this.totalUploadMs += uploadMs;
      this.maxUploadMs = Math.max(this.maxUploadMs, uploadMs);
      this.releaseReadyPayload(ready);
    }
    this.schedulePendingRequests();
  }

  /** Pause residency to inspect fallback behavior without changing the cache. */
  setDebugPaused(paused: boolean): void { this.debugPaused = paused; }

  /** Override pages loaded per frame; pass null to restore the fixed budget. */
  setDebugPageBudget(pages: number | null): void {
    if (pages !== null && (!Number.isInteger(pages) || pages < 0))
      throw new RangeError('debug page budget must be a non-negative integer or null');
    this.debugPageBudget = pages;
  }

  private debugPendingForPath(path: string): number {
    let count = 0;
    for (let slot = 0; slot < this.pendingRecords.length; slot++)
      if (this.pendingActive[slot] !== 0 && this.pendingRecords[slot].page.path === path) count++;
    return count;
  }

  /** Full read-only snapshot for atlas/residency debug UIs. */
  getDebugSnapshot() {
    return {
      ...this.getStats(),
      paused: this.debugPaused,
      debugPageBudget: this.debugPageBudget,
      atlasTexture: this.atlasTexture,
      textures: [...this.entries.values()].map(entry => ({
        textureId: entry.textureId,
        path: entry.path,
        width: entry.width,
        height: entry.height,
        pageGridX: entry.pageGridX,
        pageGridY: entry.pageGridY,
        maxMip: entry.maxMip,
        residentPages: this.pageTables.get(entry.path)?.residentCount ?? 0,
        pendingPages: this.debugPendingForPath(entry.path),
      })),
    };
  }

  /** Update and return one stable, allocation-free stats view. */
  // @hot-no-alloc-begin VirtualTextureStore.getStats
  getStats() {
    const stats = this.stats;
    stats.textureCount = this.entries.size;
    stats.atlasSlotsUsed = this.cache.usedSlots;
    stats.atlasSlotsFree = this.cache.freeSlotCount;
    stats.atlasSlotsReserved = this.cache.reservedSlots;
    stats.atlasSlotsTotal = this.cache.totalSlots;
    stats.trackedPages = this.cache.usedSlots;
    stats.pendingPages = this.pendingCount;
    stats.lodBias = this.capacityLodBias;
    stats.budget = this.pageBudget;
    stats.readyUploads = this.readyUploadCount;
    stats.maxPendingPages = this.maxPendingPages;
    stats.pendingBytes = this.pendingBytes;
    stats.maxPendingBytes = this.maxPendingBytes;
    stats.scheduledRequests = this.scheduledCount;
    stats.schedulerCapacity = this.scheduledRequests.length;
    stats.schedulerOverflows = this.schedulerOverflows;
    stats.staleCancellations = this.staleCancellations;
    stats.priorityPreemptions = this.priorityPreemptions;
    stats.rejectedAdmissions = this.rejectedAdmissions;
    stats.residentHits = this.residentHits;
    stats.residentMisses = this.residentMisses;
    stats.residentEvictions = this.residentEvictions;
    stats.completedLoads = this.completedLoads;
    stats.failedLoads = this.failedLoads;
    stats.averageLoadMs = this.completedLoads === 0 ? 0 : this.totalLoadMs / this.completedLoads;
    stats.maxLoadMs = this.maxLoadMs;
    stats.completedUploads = this.completedUploads;
    stats.averageUploadMs = this.completedUploads === 0 ? 0 : this.totalUploadMs / this.completedUploads;
    stats.maxUploadMs = this.maxUploadMs;
    stats.scheduleBudgetMs = this.scheduleBudgetMs;
    stats.uploadBudgetMs = this.tuning.uploadBudgetMs;
    stats.uploadsPerPoll = this.tuning.uploadsPerPoll;
    stats.tuningDownshifts = this.tuning.downshifts;
    stats.tuningRecoveries = this.tuning.recoveries;
    stats.tuningProbes = this.tuning.probes;
    stats.tuningProbeRejections = this.tuning.probeRejections;
    stats.tuningBestSafeUploadsPerPoll = this.tuning.bestSafeUploadsPerPoll;
    stats.tuningBestSafeUploadBudgetMs = this.tuning.bestSafeUploadBudgetMs;
    stats.scheduleBudgetExhaustions = this.scheduleBudgetExhaustions;
    stats.uploadBudgetExhaustions = this.uploadBudgetExhaustions;
    const provider = this.pageDataProvider?.getStats?.();
    if (provider) {
      stats.pageReads = provider.reads;
      stats.averagePageReadMs = provider.averageReadMs;
      stats.maxPageReadMs = provider.maxReadMs;
      stats.bulkQueued = provider.bulkQueued;
      stats.bulkInFlight = provider.bulkInFlight;
      stats.bulkInFlightBytes = provider.bulkInFlightBytes;
      stats.urgentBatches = provider.urgentBatches;
      stats.focusBatches = provider.focusBatches;
      stats.peripheralBatches = provider.peripheralBatches;
      stats.bulkRejected = provider.bulkRejected;
      stats.bulkCanceled = provider.bulkCanceled;
      stats.transcodeWorkers = provider.workerCount;
      stats.activeTranscodes = provider.activeTranscodes;
      stats.queuedTranscodes = provider.queuedTranscodes;
      stats.completedTranscodes = provider.completedTranscodes;
      stats.averageTranscodeQueueMs = provider.averageTranscodeQueueMs;
      stats.maxTranscodeQueueMs = provider.maxTranscodeQueueMs;
      stats.averageTranscodeMs = provider.averageTranscodeMs;
      stats.maxTranscodeMs = provider.maxTranscodeMs;
    }
    return stats;
  }
  // @hot-no-alloc-end VirtualTextureStore.getStats
}
