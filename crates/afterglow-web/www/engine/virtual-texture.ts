// VirtualTextureStore — manages the shared physical atlas, page table,
// LRU cache, GPU-feedback residency, and upload budget for all virtual textures.
//
// ALL sampled textures in the engine go through this store. There are no
// separate "normal" textures — everything is a page in the atlas.
//
// Architecture:
//   .big file → [page chunks at seekable offsets]
//                    ↓
//   AssetLoader.read(path, offset, len) → raw page data
//                    ↓
//   GPU feedback → deduplicate/capacity-fit → copy missing pages to atlas
//                    ↓
//   Page table texture (updated) → GPU shader samples via vtSample()
//
// The atlas and page table are THREE.DataTextures shared across all materials.
// Each virtual texture has its own page table binding (UV offset + scale).

import * as THREE from 'three';
import { AssetHandle } from './asset-handle.ts';
import { Resource, defineResource } from './resource.ts';
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

export const PAGE_SIZE = 128;           // texels per page (payload, excluding border)
export const PAGE_BORDER = 4;           // border texels PER SIDE (for bilinear/aniso)
export const SLOT_SIZE = PAGE_SIZE + PAGE_BORDER * 2; // 136 texels per physical slot
// Conservative dimensions used when a device is not supplied (notably tests).
// Runtime stores with a GPUDevice use its maximum 2D dimension instead.
export const ATLAS_PAGES_X = 15;
export const ATLAS_PAGES_Y = 15;
export const ATLAS_WIDTH = ATLAS_PAGES_X * SLOT_SIZE;
export const ATLAS_HEIGHT = ATLAS_PAGES_Y * SLOT_SIZE;
const MAX_MIP = 10;              // supports up to 2^10 = 1024 pages per side
const PRIORITY_LANE_COUNT = (MAX_MIP + 1) * 2; // exact quality rung × center/edge
const FEEDBACK_SCALE = 0.125;    // feedback at 1/8 screen resolution

// ============================================================================
// Types
// ============================================================================

/** A page request: which virtual page at which mip level is needed. */
export interface PageRequest {
  mip: number;
  x: number;
  y: number;
  /** Selects the packed sub-page mip tail instead of a regular virtual page. */
  tail?: boolean;
}

/** Globally unique page request emitted by feedback. */
export interface VirtualPageRequest extends PageRequest {
  /** Stable numeric hot-path identity. External callers may omit before admission. */
  textureId?: number;
  /** Asset path retained for providers and game-facing diagnostics. */
  path: string;
  /** 0 is screen center and 255 is the farthest edge/corner. */
  screenPriority?: number;
  /** Number of feedback pixels covered by this page in the latest readback. */
  coverage?: number;
  /** Internal fixed-lane admission priority; 0 is highest. */
  priorityTier?: number;
}

interface CachedPage extends VirtualPageRequest {
  /** Precomputed numeric identity; never rebuilt while touching/evicting. */
  cacheKey: number;
  /** Pinned pages are never selected for eviction. */
  pinned: boolean;
}

/** A physical page slot in the atlas. */
interface PageSlot {
  x: number;
  y: number;
}

/** Page table entry — bit-packed u32 (matches WGSL shader format). */
function packedPageCoordinates(
  textureId: number,
  mip: number,
  x: number,
  y: number,
  tail = false,
): number {
  const local = tail
    ? 0x10000000
    : ((mip & 0x3f) | ((x & 0x7ff) << 6) | ((y & 0x7ff) << 17)) >>> 0;
  // 29 local bits leave exact integer space for over 16 million texture IDs.
  return textureId * 0x20000000 + local;
}

function packedPageIdentity(textureId: number, req: PageRequest): number {
  return packedPageCoordinates(textureId, req.mip, req.x, req.y, req.tail);
}

function packEntry(resident: boolean, physX: number, physY: number): number {
  return (resident ? 1 : 0) | ((physX & 0xFF) << 1) | ((physY & 0xFF) << 9);
}
function isResident(entry: number): boolean { return (entry & 1) !== 0; }
function getPhysX(entry: number): number { return (entry >> 1) & 0xFF; }
function getPhysY(entry: number): number { return (entry >> 9) & 0xFF; }

// Compressed format block sizes (4×4 texel blocks)
const BLOCK_SIZE = 4;
// BC7 = 16 bytes per 4×4 block. ASTC 4×4 = 16 bytes. RGBA8 = 64 bytes (4×4×4).
const BC7_BYTES_PER_BLOCK = 16;
const ASTC_BYTES_PER_BLOCK = 16;
const RGBA_BYTES_PER_BLOCK = BLOCK_SIZE * BLOCK_SIZE * 4; // 64

// Slots in compressed blocks
const SLOT_BLOCKS_X = SLOT_SIZE / BLOCK_SIZE; // 34
const SLOT_BLOCKS_Y = SLOT_SIZE / BLOCK_SIZE; // 34


/** Texture format constants — match afterglow-texture worker */
export const FORMAT_BC7 = 0;
export const FORMAT_ASTC = 1;
export const FORMAT_RGBA = 4;

/** Detect the best texture format for the current GPU. */
export async function detectBestTextureFormat(adapter?: GPUAdapter | null): Promise<number> {
  if (adapter) {
    const f = adapter.features;
    if (f.has('texture-compression-bc')) return FORMAT_BC7;
    if (f.has('texture-compression-astc')) return FORMAT_ASTC;
  }
  return FORMAT_RGBA;
}

/** Get bytes per compressed block for a format. */
function bytesPerBlock(format: number): number {
  if (format === FORMAT_BC7 || format === FORMAT_ASTC) return BC7_BYTES_PER_BLOCK;
  return RGBA_BYTES_PER_BLOCK;
}

/** Get the Three.js format constant for a format. */
function threeFormat(format: number): THREE.CompressedPixelFormat {
  if (format === FORMAT_BC7) return THREE.RGBA_BPTC_Format;
  if (format === FORMAT_ASTC) return THREE.RGBA_ASTC_4x4_Format;
  throw new RangeError(`unsupported compressed texture format ${format}`);
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
}

interface PageDataProviderTelemetry {
  reads: number;
  averageReadMs: number;
  maxReadMs: number;
  workerCount: number;
  activeTranscodes: number;
  queuedTranscodes: number;
  completedTranscodes: number;
  averageTranscodeQueueMs: number;
  maxTranscodeQueueMs: number;
  averageTranscodeMs: number;
  maxTranscodeMs: number;
  cacheEnabled: boolean;
  cacheBackend: string;
  cacheEntries: number;
  cacheBytes: number;
  cacheLiveBytes: number;
  cacheQueuedWrites: number;
  cacheEvictions: number;
  cacheCompactions: number;
  cacheReclaimedBytes: number;
  cacheMaintenance: boolean;
  cacheHits: number;
  cacheMisses: number;
  cacheWrites: number;
  cacheRejected: number;
  cacheErrors: number;
  averageCacheReadMs: number;
  maxCacheReadMs: number;
  averageCacheWriteMs: number;
  maxCacheWriteMs: number;
}

type PageDataProvider = ((path: string, req: PageRequest, signal?: AbortSignal) => Promise<Uint8Array>) & {
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

// ============================================================================
// Page Table
// ============================================================================

class PageTable {
  private count = 0;

  constructor(
    private readonly layout: PackedPageTableLayout,
    private readonly entries: Uint32Array,
  ) {}

  private index(req: PageRequest): number {
    return packedPageTableIndex(this.layout, req.mip, req.x, req.y);
  }

  get(req: PageRequest): number {
    return this.entries[this.index(req)];
  }

  setResident(req: PageRequest, slot: PageSlot): void {
    const index = this.index(req);
    if (!isResident(this.entries[index])) this.count++;
    this.entries[index] = packEntry(true, slot.x, slot.y);
  }

  setEvicted(req: PageRequest): void {
    const index = this.index(req);
    if (isResident(this.entries[index])) this.count--;
    this.entries[index] = 0;
  }

  isResident(req: PageRequest): boolean {
    return isResident(this.get(req));
  }

  isResidentAt(mip: number, x: number, y: number): boolean {
    return isResident(this.entries[packedPageTableIndex(this.layout, mip, x, y)]);
  }

  get residentCount(): number { return this.count; }
}

// ============================================================================
// Page Cache (Physical Atlas + fixed clock)
// ============================================================================

/** Fixed open-addressed numeric page-key map. Never resizes after bootstrap. */
class FixedPageSlotMap {
  private readonly keys: Float64Array;
  private readonly values: Uint32Array;
  private readonly states: Uint8Array; // 0 empty, 1 occupied, 2 tombstone
  private readonly mask: number;

  constructor(minCapacity: number) {
    let capacity = 1;
    while (capacity < minCapacity * 2) capacity <<= 1;
    this.keys = new Float64Array(capacity);
    this.values = new Uint32Array(capacity);
    this.states = new Uint8Array(capacity);
    this.mask = capacity - 1;
  }

  private hash(key: number): number {
    return (((key >>> 0) ^ Math.floor(key / 0x100000000)) * 2654435761) >>> 0;
  }

  // @hot-no-alloc-begin FixedPageSlotMap.get
  get(key: number): number | undefined {
    let index = this.hash(key) & this.mask;
    for (let probe = 0; probe <= this.mask; probe++) {
      const state = this.states[index];
      if (state === 0) return undefined;
      if (state === 1 && this.keys[index] === key) return this.values[index];
      index = (index + 1) & this.mask;
    }
    return undefined;
  }
  // @hot-no-alloc-end FixedPageSlotMap.get

  // @hot-no-alloc-begin FixedPageSlotMap.set
  set(key: number, value: number): void {
    let index = this.hash(key) & this.mask;
    let tombstone = -1;
    for (let probe = 0; probe <= this.mask; probe++) {
      const state = this.states[index];
      if (state === 1 && this.keys[index] === key) {
        this.values[index] = value;
        return;
      }
      if (state === 2 && tombstone < 0) tombstone = index;
      if (state === 0) {
        const target = tombstone < 0 ? index : tombstone;
        this.keys[target] = key;
        this.values[target] = value;
        this.states[target] = 1;
        return;
      }
      index = (index + 1) & this.mask;
    }
    if (tombstone >= 0) {
      this.keys[tombstone] = key;
      this.values[tombstone] = value;
      this.states[tombstone] = 1;
      return;
    }
    throw new Error('fixed VT page map capacity exceeded');
  }
  // @hot-no-alloc-end FixedPageSlotMap.set

  // @hot-no-alloc-begin FixedPageSlotMap.delete
  clear(): void {
    for (let index = 0; index < this.states.length; index++) this.states[index] = 0;
  }

  delete(key: number): boolean {
    let index = this.hash(key) & this.mask;
    for (let probe = 0; probe <= this.mask; probe++) {
      const state = this.states[index];
      if (state === 0) return false;
      if (state === 1 && this.keys[index] === key) {
        this.states[index] = 2;
        return true;
      }
      index = (index + 1) & this.mask;
    }
    return false;
  }
  // @hot-no-alloc-end FixedPageSlotMap.delete
}

class PageCache {
  private format: number;
  readonly pagesX: number;
  readonly pagesY: number;
  readonly width: number;
  readonly height: number;
  /** Atlas pixel data (compressed blocks or RGBA8). */
  atlas: Uint8Array;
  /** Bytes per row in the atlas (for the selected format). */
  atlasBytesPerRow: number;
  /** Bytes per slot row. */
  slotBytesPerRow: number;
  /** Slot data size in bytes (for one page). */
  slotDataSize: number;
  /** Fixed slot records and O(1) page-key lookup. */
  private slots: CachedPage[] = [];
  private slotActive: Uint8Array;
  private slotCoords: PageSlot[] = [];
  private slotByKey: FixedPageSlotMap;
  /** Reservation and second-chance clock state. */
  private reserved: Uint8Array;
  private referenced: Uint8Array;
  private clockHand = 0;
  /** Fixed-capacity free-index stack. */
  private freeSlots: Uint32Array;
  private freeTop = 0;
  private usedCount = 0;
  private pinnedCount = 0;
  private acquireResult: { slot: PageSlot; evicted: CachedPage | null };
  constructor(format: number = FORMAT_RGBA, maxTextureDimension = ATLAS_WIDTH) {
    this.format = format;
    this.pagesX = Math.max(1, Math.floor(maxTextureDimension / SLOT_SIZE));
    this.pagesY = this.pagesX;
    this.width = this.pagesX * SLOT_SIZE;
    this.height = this.pagesY * SLOT_SIZE;
    const blocksX = this.width / BLOCK_SIZE, blocksY = this.height / BLOCK_SIZE;
    const bpb = bytesPerBlock(format);
    if (format === FORMAT_RGBA) {
      this.atlasBytesPerRow = this.width * 4;
      this.slotBytesPerRow = SLOT_SIZE * 4;
      this.slotDataSize = SLOT_SIZE * SLOT_SIZE * 4;
      this.atlas = new Uint8Array(this.width * this.height * 4);
    } else {
      this.atlasBytesPerRow = blocksX * bpb;
      this.slotBytesPerRow = SLOT_BLOCKS_X * bpb;
      this.slotDataSize = SLOT_BLOCKS_X * SLOT_BLOCKS_Y * bpb;
      this.atlas = new Uint8Array(blocksX * blocksY * bpb);
    }

    this.slotByKey = new FixedPageSlotMap(this.pagesX * this.pagesY);
    this.reserved = new Uint8Array(this.pagesX * this.pagesY);
    this.referenced = new Uint8Array(this.pagesX * this.pagesY);
    this.slotActive = new Uint8Array(this.pagesX * this.pagesY);
    this.freeSlots = new Uint32Array(this.pagesX * this.pagesY);
    for (let y = 0; y < this.pagesY; y++) {
      for (let x = 0; x < this.pagesX; x++) {
        this.slots.push({ path: '', mip: 0, x: 0, y: 0, pinned: false, cacheKey: 0 });
        this.slotCoords.push({ x, y });
        this.freeSlots[this.freeTop++] = y * this.pagesX + x;
      }
    }
    this.acquireResult = { slot: this.slotCoords[0], evicted: null };
  }

  /** O(1) cache touch: set one second-chance reference bit. */
  // @hot-no-alloc-begin PageCache.touch
  touch(cacheKey: number): void {
    const slot = this.slotByKey.get(cacheKey);
    if (slot !== undefined) this.referenced[slot] = 1;
  }
  // @hot-no-alloc-end PageCache.touch

  /** Acquire from the free stack or a bounded second-chance clock scan. */
  // @hot-no-alloc-begin PageCache.acquire
  acquire(req: CachedPage): { slot: PageSlot; evicted: CachedPage | null } {
    let slotIdx: number | undefined = this.freeTop === 0 ? undefined : this.freeSlots[--this.freeTop];
    let evicted: CachedPage | null = null;
    if (slotIdx === undefined) {
      // Two bounded passes are sufficient: pass one clears reference bits;
      // pass two selects the first unpinned resident page.
      const limit = this.slots.length * 2;
      for (let scanned = 0; scanned < limit; scanned++) {
        const candidate = this.clockHand;
        this.clockHand = (this.clockHand + 1) % this.slots.length;
        const page = this.slots[candidate];
        if (this.slotActive[candidate] === 0 || this.reserved[candidate] !== 0 || page.pinned) continue;
        if (this.referenced[candidate] !== 0) {
          this.referenced[candidate] = 0;
          continue;
        }
        slotIdx = candidate;
        evicted = page;
        this.slotByKey.delete(page.cacheKey);
        this.slotActive[candidate] = 0;
        this.usedCount--;
        break;
      }
    }
    if (slotIdx === undefined) throw new Error('No evictable VT atlas slot');
    this.reserved[slotIdx] = 1;
    this.acquireResult.slot = this.slotCoords[slotIdx];
    this.acquireResult.evicted = evicted;
    return this.acquireResult;
  }
  // @hot-no-alloc-end PageCache.acquire

  /** Write page data into a slot and mark as resident. */
  commit(req: CachedPage, slot: PageSlot, data: Uint8Array) {
    const rows = this.format === FORMAT_RGBA ? SLOT_SIZE : SLOT_BLOCKS_Y;
    const dstXBytes = this.format === FORMAT_RGBA
      ? slot.x * SLOT_SIZE * 4
      : slot.x * SLOT_BLOCKS_X * (this.slotBytesPerRow / SLOT_BLOCKS_X);
    const dstY = this.format === FORMAT_RGBA ? slot.y * SLOT_SIZE : slot.y * SLOT_BLOCKS_Y;

    for (let row = 0; row < rows; row++) {
      const srcOffset = row * this.slotBytesPerRow;
      const dstOffset = (dstY + row) * this.atlasBytesPerRow + dstXBytes;
      this.atlas.set(data.subarray(srcOffset, srcOffset + this.slotBytesPerRow), dstOffset);
    }

    const slotIdx = slot.y * this.pagesX + slot.x;
    if (this.reserved[slotIdx] === 0 || this.slotActive[slotIdx] !== 0)
      throw new Error('VT slot commit without a unique reservation');
    this.reserved[slotIdx] = 0;
    this.referenced[slotIdx] = 1;
    const resident = this.slots[slotIdx];
    resident.textureId = req.textureId;
    resident.path = req.path;
    resident.mip = req.mip;
    resident.x = req.x;
    resident.y = req.y;
    resident.tail = req.tail;
    resident.pinned = req.pinned;
    resident.cacheKey = req.cacheKey;
    this.slotActive[slotIdx] = 1;
    this.slotByKey.set(req.cacheKey, slotIdx);
    this.usedCount++;
    if (req.pinned) this.pinnedCount++;
  }


  /** Remove one committed unpinned page by numeric identity. */
  evictByKey(cacheKey: number): CachedPage | null {
    const index = this.slotByKey.get(cacheKey);
    if (index === undefined) return null;
    const page = this.slots[index];
    if (this.slotActive[index] === 0 || page.pinned || this.reserved[index] !== 0) return null;
    this.slotByKey.delete(cacheKey);
    this.slotActive[index] = 0;
    this.referenced[index] = 0;
    this.usedCount--;
    this.freeSlots[this.freeTop++] = index;
    return page;
  }

  /** Remove every resident page owned by one virtual texture (structural path). */
  removeTexture(path: string): void {
    for (let index = 0; index < this.slots.length; index++) {
      const page = this.slots[index];
      if (this.slotActive[index] === 0 || page.path !== path) continue;
      this.slotByKey.delete(page.cacheKey);
      this.slotActive[index] = 0;
      this.referenced[index] = 0;
      this.usedCount--;
      if (page.pinned) this.pinnedCount--;
      this.freeSlots[this.freeTop++] = index;
    }
  }

  get usedSlots(): number { return this.usedCount; }
  get pinnedSlots(): number { return this.pinnedCount; }
  get freeSlotCount(): number { return this.freeTop; }
  get totalSlots(): number { return this.pagesX * this.pagesY; }
}

// ============================================================================
// Central VT upload tuning
// ============================================================================

/** Bootstrap-only policy for bounded per-frame atlas/page-table commits. */
export interface VirtualTextureTuningConfig {
  minUploadsPerPoll: number;
  baselineUploadsPerPoll: number;
  maxUploadsPerPoll: number;
  minUploadBudgetMs: number;
  baselineUploadBudgetMs: number;
  maxUploadBudgetMs: number;
  uploadBudgetStepMs: number;
  targetFrameMs: number;
  overloadMultiplier: number;
  overloadSamples: number;
  sampleWindow: number;
  stableWindowsBeforeProbe: number;
  probeCooldownWindows: number;
}

export const DEFAULT_VIRTUAL_TEXTURE_TUNING: Readonly<VirtualTextureTuningConfig> = {
  minUploadsPerPoll: 1,
  baselineUploadsPerPoll: 2,
  maxUploadsPerPoll: 4,
  minUploadBudgetMs: 0.10,
  baselineUploadBudgetMs: 0.20,
  maxUploadBudgetMs: 0.35,
  uploadBudgetStepMs: 0.05,
  targetFrameMs: 1000 / 60,
  overloadMultiplier: 1.25,
  overloadSamples: 2,
  sampleWindow: 15,
  stableWindowsBeforeProbe: 1,
  probeCooldownWindows: 60,
};

/**
 * Central, allocation-free VT upload tuner. It discovers throughput only under
 * real backlog: after several clean windows it probes one bounded step above
 * the configured device caps. A repeated presentation miss rolls any promoted
 * setting back immediately to the independently validated baseline and applies
 * a cooldown, so powerful devices can climb while
 * weaker devices do not continuously oscillate around a failing cap.
 */
export class VirtualTextureTuning {
  readonly minUploadsPerPoll: number;
  readonly baselineUploadsPerPoll: number;
  readonly maxUploadsPerPoll: number;
  readonly minUploadBudgetMs: number;
  readonly baselineUploadBudgetMs: number;
  readonly maxUploadBudgetMs: number;
  readonly uploadBudgetStepMs: number;
  readonly targetFrameMs: number;
  readonly overloadFrameMs: number;
  readonly overloadSamples: number;
  readonly sampleWindow: number;
  readonly stableWindowsBeforeProbe: number;
  readonly probeCooldownWindows: number;
  uploadsPerPoll: number;
  uploadBudgetMs: number;
  bestSafeUploadsPerPoll: number;
  bestSafeUploadBudgetMs: number;
  private samples = 0;
  private overloaded = 0;
  private stableWindows = 0;
  private cooldownWindows = 0;
  private probing = false;
  downshifts = 0;
  recoveries = 0;
  probes = 0;
  probeRejections = 0;

  constructor(config: Readonly<VirtualTextureTuningConfig> = DEFAULT_VIRTUAL_TEXTURE_TUNING) {
    const integers = [config.minUploadsPerPoll, config.baselineUploadsPerPoll,
      config.maxUploadsPerPoll, config.overloadSamples, config.sampleWindow,
      config.stableWindowsBeforeProbe, config.probeCooldownWindows];
    if (integers.some(value => !Number.isInteger(value) || value < 1) ||
        config.minUploadsPerPoll > config.baselineUploadsPerPoll ||
        config.baselineUploadsPerPoll > config.maxUploadsPerPoll ||
        !Number.isFinite(config.minUploadBudgetMs) || !Number.isFinite(config.baselineUploadBudgetMs) ||
        !Number.isFinite(config.maxUploadBudgetMs) || config.minUploadBudgetMs <= 0 ||
        config.minUploadBudgetMs > config.baselineUploadBudgetMs ||
        config.baselineUploadBudgetMs > config.maxUploadBudgetMs ||
        !Number.isFinite(config.uploadBudgetStepMs) || config.uploadBudgetStepMs <= 0 ||
        !Number.isFinite(config.targetFrameMs) || config.targetFrameMs <= 0 ||
        !Number.isFinite(config.overloadMultiplier) || config.overloadMultiplier <= 1)
      throw new RangeError('invalid virtual-texture tuning configuration');
    this.minUploadsPerPoll = config.minUploadsPerPoll;
    this.baselineUploadsPerPoll = config.baselineUploadsPerPoll;
    this.maxUploadsPerPoll = config.maxUploadsPerPoll;
    this.minUploadBudgetMs = config.minUploadBudgetMs;
    this.baselineUploadBudgetMs = config.baselineUploadBudgetMs;
    this.maxUploadBudgetMs = config.maxUploadBudgetMs;
    this.uploadBudgetStepMs = config.uploadBudgetStepMs;
    this.targetFrameMs = config.targetFrameMs;
    this.overloadFrameMs = config.targetFrameMs * config.overloadMultiplier;
    this.overloadSamples = config.overloadSamples;
    this.sampleWindow = config.sampleWindow;
    this.stableWindowsBeforeProbe = config.stableWindowsBeforeProbe;
    this.probeCooldownWindows = config.probeCooldownWindows;
    this.uploadsPerPoll = config.baselineUploadsPerPoll;
    this.uploadBudgetMs = config.baselineUploadBudgetMs;
    this.bestSafeUploadsPerPoll = config.baselineUploadsPerPoll;
    this.bestSafeUploadBudgetMs = config.baselineUploadBudgetMs;
  }

  private resetWindow(): void {
    this.samples = 0;
    this.overloaded = 0;
  }

  private lowerOneStep(): void {
    if (this.uploadsPerPoll > this.minUploadsPerPoll) {
      this.uploadsPerPoll--;
    } else {
      this.uploadBudgetMs = Math.max(this.minUploadBudgetMs, this.uploadBudgetMs - this.uploadBudgetStepMs);
    }
    this.bestSafeUploadsPerPoll = this.uploadsPerPoll;
    this.bestSafeUploadBudgetMs = this.uploadBudgetMs;
    this.downshifts++;
  }

  private probeOneStep(): boolean {
    if (this.uploadsPerPoll < this.maxUploadsPerPoll) {
      this.uploadsPerPoll++;
    } else if (this.uploadBudgetMs < this.maxUploadBudgetMs) {
      this.uploadBudgetMs = Math.min(this.maxUploadBudgetMs, this.uploadBudgetMs + this.uploadBudgetStepMs);
    } else {
      return false;
    }
    this.probing = true;
    this.probes++;
    return true;
  }

  // @hot-no-alloc-begin VirtualTextureTuning.recordFrameTime
  recordFrameTime(frameMs: number, backlog: number): void {
    if (!Number.isFinite(frameMs) || frameMs <= 0) {
      this.stableWindows = 0;
      this.resetWindow();
      return;
    }
    // Preserve evidence across short empty gaps: gameplay streaming arrives in
    // bursts, and resetting here prevented any burst from calibrating the cap.
    if (backlog <= 0) return;
    this.samples++;
    if (frameMs > this.overloadFrameMs) this.overloaded++;
    if (this.samples < this.sampleWindow) return;

    if (this.overloaded >= this.overloadSamples) {
      // The bootstrap baseline was independently validated. Any promoted cap
      // that starts missing presentation deadlines abandons its whole probe
      // ladder at once; only a failing baseline tightens one step further.
      if (this.probing || this.uploadsPerPoll > this.baselineUploadsPerPoll ||
          this.uploadBudgetMs > this.baselineUploadBudgetMs) {
        this.uploadsPerPoll = this.baselineUploadsPerPoll;
        this.uploadBudgetMs = this.baselineUploadBudgetMs;
        this.bestSafeUploadsPerPoll = this.baselineUploadsPerPoll;
        this.bestSafeUploadBudgetMs = this.baselineUploadBudgetMs;
        this.probing = false;
        this.probeRejections++;
      } else {
        this.lowerOneStep();
      }
      this.stableWindows = 0;
      this.cooldownWindows = this.probeCooldownWindows;
    } else if (this.overloaded === 0) {
      if (this.probing) {
        this.bestSafeUploadsPerPoll = this.uploadsPerPoll;
        this.bestSafeUploadBudgetMs = this.uploadBudgetMs;
        this.probing = false;
        this.recoveries++;
        this.stableWindows = 0;
      } else if (this.cooldownWindows > 0) {
        this.cooldownWindows--;
        this.stableWindows = 0;
      } else {
        this.stableWindows++;
        if (this.stableWindows >= this.stableWindowsBeforeProbe) {
          this.probeOneStep();
          this.stableWindows = 0;
        }
      }
    } else {
      this.stableWindows = 0;
    }
    this.resetWindow();
  }
  // @hot-no-alloc-end VirtualTextureTuning.recordFrameTime
}

export const VirtualTextureTuningRes = defineResource<VirtualTextureTuning>(
  'virtualTextureTuning',
  () => new VirtualTextureTuning(),
);

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
 * Usage:
 *   const vt = VirtualTextureRes.get(world);
 *   const handle = vt.loadTexture('terrain');
 *   // handle.asset = shared atlas texture
 *   // handle.generation increments when pages are loaded/evicted
 *
 * The VT store reads page data from the .big file via the pageDataProvider.
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
  private materialGroupIdsById: (readonly number[] | null)[] = [null];
  private materialGroupByPath = new Map<string, string[]>();
  private nextTextureId = 1;

  /** Per-path page tables (for lookup). */
  private pageTables = new Map<string, PageTable>();

  /** The shared page cache (atlas + fixed second-chance clock). */
  private cache: PageCache;

  /** The asset loader (for reading page data from .big). */
  private loader: { read(path: string, offset: number, len: number): Promise<Uint8Array>; poll(): void };

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
  /** Shared central policy; created during bootstrap and sampled each frame. */
  readonly tuning: VirtualTextureTuning;
  private readonly maxPendingPages = 64;
  private readonly maxPendingBytes = 8 * 1024 * 1024;
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
  private scheduledSince: Uint32Array;
  private scheduledPriority: Uint8Array;
  private scheduledNext: Int32Array;
  private scheduledPrevious: Int32Array;
  private readonly priorityHeads = new Int32Array(PRIORITY_LANE_COUNT);
  private readonly priorityTails = new Int32Array(PRIORITY_LANE_COUNT);
  private scheduledFree: Uint32Array;
  private scheduledFreeTop = 0;
  private scheduledByKey: FixedPageSlotMap;
  private scheduledCount = 0;
  private feedbackEpoch = 0;
  /** Two absent 36 Hz feedback epochs tolerate readback jitter but drop old views quickly. */
  private readonly staleFeedbackEpochs = 2;
  private staleCancellations = 0;
  private priorityPreemptions = 0;
  private schedulerOverflows = 0;
  private feedbackScratch: VirtualPageRequest[];
  private feedbackScratchKeys: FixedPageSlotMap;
  private feedbackScratchCount = 0;
  private cacheHits = 0;
  private cacheMisses = 0;
  private cacheEvictions = 0;
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
    textureCount: 0, atlasSlotsUsed: 0, atlasSlotsTotal: 0, trackedPages: 0,
    pendingPages: 0, lodBias: 0, budget: 0, readyUploads: 0,
    maxPendingPages: 0, pendingBytes: 0, maxPendingBytes: 0,
    scheduledRequests: 0, schedulerCapacity: 0, schedulerOverflows: 0,
    staleCancellations: 0, priorityPreemptions: 0, rejectedAdmissions: 0,
    cacheHits: 0, cacheMisses: 0, cacheEvictions: 0, completedLoads: 0, failedLoads: 0,
    averageLoadMs: 0, maxLoadMs: 0, completedUploads: 0,
    averageUploadMs: 0, maxUploadMs: 0, scheduleBudgetMs: 0,
    uploadBudgetMs: 0, uploadsPerPoll: 0, tuningDownshifts: 0, tuningRecoveries: 0,
    tuningProbes: 0, tuningProbeRejections: 0,
    tuningBestSafeUploadsPerPoll: 0, tuningBestSafeUploadBudgetMs: 0,
    scheduleBudgetExhaustions: 0, uploadBudgetExhaustions: 0,
    pageReads: 0, averagePageReadMs: 0, maxPageReadMs: 0,
    transcodeWorkers: 0, activeTranscodes: 0, queuedTranscodes: 0,
    completedTranscodes: 0, averageTranscodeQueueMs: 0, maxTranscodeQueueMs: 0,
    averageTranscodeMs: 0, maxTranscodeMs: 0,
    cacheEnabled: false, cacheBackend: '', cacheEntries: 0, cacheBytes: 0,
    cacheLiveBytes: 0, cacheQueuedWrites: 0, cacheEvictions: 0, cacheCompactions: 0,
    cacheReclaimedBytes: 0, cacheMaintenance: false,
    cacheHits: 0, cacheMisses: 0, cacheWrites: 0, cacheRejected: 0, cacheErrors: 0,
    averageCacheReadMs: 0, maxCacheReadMs: 0,
    averageCacheWriteMs: 0, maxCacheWriteMs: 0,
  };

  constructor(
    loader: { read(path: string, offset: number, len: number): Promise<Uint8Array>; poll(): void },
    pageDataProvider?: PageDataProvider,
    format?: number,
    device?: GPUDevice,
    tuning?: VirtualTextureTuning,
  ) {
    this.loader = loader;
    this.pageDataProvider = pageDataProvider;
    this.format = format ?? FORMAT_RGBA;
    this.device = device ?? null;
    this.tuning = tuning ?? new VirtualTextureTuning();
    this.cache = new PageCache(this.format, device?.limits.maxTextureDimension2D ?? ATLAS_WIDTH);
    this.atlasWidth = this.cache.width;
    this.atlasHeight = this.cache.height;
    this.atlasPagesX = this.cache.pagesX;
    this.atlasPagesY = this.cache.pagesY;

    const schedulerCapacity = this.cache.totalSlots;
    this.scheduledKeys = new Float64Array(schedulerCapacity);
    this.scheduledRequests = new Array(schedulerCapacity);
    this.scheduledActive = new Uint8Array(schedulerCapacity);
    this.scheduledLastSeen = new Uint32Array(schedulerCapacity);
    this.scheduledSince = new Uint32Array(schedulerCapacity);
    this.scheduledPriority = new Uint8Array(schedulerCapacity);
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
        priorityTier: 7,
        controller: null,
        canceled: false,
      };
      this.pendingFree[this.pendingFreeTop++] = index;
    }
    this.readyUploads = new Array(64);
    for (let index = 0; index < this.readyUploads.length; index++) {
      const page = { path: '', mip: 0, x: 0, y: 0, pinned: false, cacheKey: 0 };
      this.readyUploads[index] = { key: 0, generation: 0, page, req: page, data: new Uint8Array(0) };
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

    // Create the shared atlas texture
    if (this.format === FORMAT_RGBA) {
      // Uncompressed: use DataTexture (simple, works everywhere)
      this.atlasTexture = new THREE.DataTexture(
        this.cache.atlas,
        this.atlasWidth,
        this.atlasHeight,
        THREE.RGBAFormat,
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
    options?: { width?: number; height?: number; mipTail?: boolean },
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

    const textureId = this.nextTextureId++;
    if (textureId > 0xffffffff) throw new Error('virtual texture ID space exhausted');
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
    this.materialGroupIdsById[textureId] = null;

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
    options?: { width?: number; height?: number; mipTail?: boolean },
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
    this.linkMaterialSet(set);
    return set;
  }

  /** Link an already-loaded aligned PBR set so one feedback request restores every channel. */
  linkMaterialSet(set: VirtualMaterialSet): void {
    if (set.masks && (set.roughness || set.ao))
      throw new Error('packed masks and separate roughness/AO paths are mutually exclusive');
    const channels = [set.albedo, set.normal, set.masks, set.roughness, set.ao, set.emissive]
      .filter((entry): entry is VirtualTextureEntry => entry !== undefined);
    if (channels.some(entry => entry.width !== set.albedo.width || entry.height !== set.albedo.height ||
        entry.pageGridX !== set.albedo.pageGridX || entry.pageGridY !== set.albedo.pageGridY ||
        entry.maxMip !== set.albedo.maxMip))
      throw new Error('linked virtual material channels must have identical page layouts');
    const groupIds: number[] = [];
    const addId = (textureId: number): void => {
      if (!groupIds.includes(textureId)) groupIds.push(textureId);
    };
    for (const channel of channels) {
      addId(channel.textureId);
      const existing = this.materialGroupIdsById[channel.textureId];
      if (existing) for (const textureId of existing) addId(textureId);
    }
    const groupPaths = groupIds.map(textureId => this.entriesById[textureId]!.path);
    for (let index = 0; index < groupIds.length; index++) {
      this.materialGroupByPath.set(groupPaths[index], groupPaths);
      this.materialGroupIdsById[groupIds[index]] = groupIds;
    }
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
    priorityTier = 7,
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
    // `req` may be a reusable scheduler scratch record. `page` is the owned
    // immutable copy retained for this asynchronous generation.
    this.pageDataProvider(path, page, controller.signal).then(data => {
      if (data.byteLength !== this.cache.slotDataSize) {
        throw new RangeError(`VT page ${key} has ${data.byteLength} bytes; expected ${this.cache.slotDataSize}`);
      }
      const pending = this.getPending(key);
      if (!pending || pending.generation !== generation) return;
      if (pending.canceled) {
        this.deletePending(key);
        return;
      }
      const loadMs = performance.now() - pending.startedAt;
      this.completedLoads++;
      this.totalLoadMs += loadMs;
      this.maxLoadMs = Math.max(this.maxLoadMs, loadMs);
      if (!pending.page.pinned && this.feedbackEpoch - pending.lastSeen >= this.staleFeedbackEpochs) {
        this.deletePending(key);
        this.staleCancellations++;
        return;
      }
      // Completion can happen at any point between frames. Defer atlas and
      // page-table writes to poll() so a fast worker cannot burst dozens of
      // GPU queue submissions into one presentation interval.
      if (this.readyUploadCount >= this.readyUploads.length)
        throw new Error('VT ready-upload ring capacity exceeded');
      const ready = this.readyUploads[this.readyUploadTail];
      ready.key = key;
      ready.generation = generation;
      ready.page = page;
      ready.req = page;
      ready.data = data;
      this.readyUploadTail = (this.readyUploadTail + 1) % this.readyUploads.length;
      this.readyUploadCount++;
    }).catch(error => {
      const pending = this.getPending(key);
      const canceled = controller.signal.aborted ||
        (pending?.generation === generation && pending.canceled);
      if (pending?.generation === generation) this.deletePending(key);
      if (canceled) return;
      this.failedLoads++;
      console.error(`[VT] Failed to load page ${path} mip=${page.mip} (${page.x},${page.y}):`, error);
    });
    return true;
  }

  /** Load a single page asynchronously (used for pinned startup pages). */
  private loadPage(path: string, req: PageRequest, pageTable: PageTable, pinned = false): void {
    const entry = this.entries.get(path);
    if (entry) this.queuePageLoad(entry, req, pageTable, pinned);
  }

  // @hot-no-alloc-begin VirtualTextureStore.addFeedbackRequest
  private addFeedbackPage(
    textureId: number,
    entry: VirtualTextureEntry,
    source: VirtualPageRequest,
    mip: number,
    x: number,
    y: number,
    tail: boolean,
    capacity: number,
  ): boolean {
    const key = packedPageCoordinates(entry.textureId, mip, x, y, tail);
    const existing = this.feedbackScratchKeys.get(key);
    if (existing !== undefined) {
      const request = this.feedbackScratch[existing];
      request.screenPriority = Math.min(request.screenPriority ?? 255, source.screenPriority ?? 0);
      request.coverage = Math.min(0xffff, (request.coverage ?? 1) + (source.coverage ?? 1));
      if (request.screenPriority <= 96 || request.coverage >= 4)
        request.priorityTier = (request.priorityTier ?? 1) & ~1;
      return true;
    }
    if (this.feedbackScratchCount >= capacity) return false;
    const index = this.feedbackScratchCount++;
    const request = this.feedbackScratch[index];
    const qualityDepth = tail ? 0 : Math.min(MAX_MIP, entry.maxMip - mip);
    const centralOrLarge = (source.screenPriority ?? 0) <= 96 || (source.coverage ?? 1) >= 4;
    request.textureId = textureId;
    request.path = entry.path;
    request.mip = mip;
    request.x = x;
    request.y = y;
    request.tail = tail ? true : undefined;
    request.screenPriority = source.screenPriority ?? 0;
    request.coverage = source.coverage ?? 1;
    request.priorityTier = qualityDepth * 2 + (centralOrLarge ? 0 : 1);
    this.feedbackScratchKeys.set(key, index);
    return true;
  }

  private addFeedbackRequest(textureId: number, source: VirtualPageRequest, bias: number, capacity: number): boolean {
    const entry = this.entriesById[textureId];
    const pageTable = this.pageTablesById[textureId];
    if (!entry || !pageTable || !this.isValidEntryRequest(entry, source)) return true;
    if (source.tail === true)
      return this.addFeedbackPage(textureId, entry, source, source.mip, 0, 0, true, capacity);

    const desiredMip = Math.min(entry.maxMip, source.mip + bias);
    const desiredX = source.x >> (desiredMip - source.mip);
    const desiredY = source.y >> (desiredMip - source.mip);
    if (pageTable.isResidentAt(desiredMip, desiredX, desiredY))
      return this.addFeedbackPage(textureId, entry, source, desiredMip, desiredX, desiredY, false, capacity);

    let fallbackMip = desiredMip + 1;
    while (fallbackMip <= entry.maxMip &&
           !pageTable.isResidentAt(
             fallbackMip,
             desiredX >> (fallbackMip - desiredMip),
             desiredY >> (fallbackMip - desiredMip),
           )) fallbackMip++;

    // Enqueue the whole missing chain now instead of waiting for another GPU
    // feedback roundtrip after every rung. Fixed priority lanes still dispatch
    // coarse restoration before middle/high/ultra upgrades.
    for (let mip = Math.min(entry.maxMip, fallbackMip - 1); mip >= desiredMip; mip--) {
      const shift = mip - desiredMip;
      if (!this.addFeedbackPage(
        textureId, entry, source, mip, desiredX >> shift, desiredY >> shift, false, capacity,
      )) return false;
    }
    return true;
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
        const group = this.materialGroupIdsById[textureId];
        if (group) {
          for (const channelId of group) {
            if (!this.addFeedbackRequest(channelId, source, bias, capacity)) {
              fits = false;
              break outer;
            }
          }
        } else if (!this.addFeedbackRequest(textureId, source, bias, capacity)) {
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
          const group = this.materialGroupIdsById[textureId];
          if (group) {
            for (const channelId of group) {
              if (!this.addFeedbackRequest(channelId, source, bias, capacity)) {
                fits = false;
                break outer;
              }
            }
          } else if (!this.addFeedbackRequest(textureId, source, bias, capacity)) {
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
    target.priorityTier = source.priorityTier;
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

  private removeScheduled(index: number): void {
    if (this.scheduledActive[index] === 0) return;
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
      this.cacheHits++;
      this.cache.touch(key);
      const scheduled = this.scheduledByKey.get(key);
      if (scheduled !== undefined) this.removeScheduled(scheduled);
      return;
    }
    this.cacheMisses++;
    const pending = this.getPending(key);
    if (pending) {
      pending.lastSeen = this.feedbackEpoch;
      pending.priorityTier = Math.min(pending.priorityTier, request.priorityTier ?? 7);
      return;
    }
    const existing = this.scheduledByKey.get(key);
    if (existing !== undefined) {
      this.copyRequest(this.scheduledRequests[existing], request);
      const agePromotion = (this.feedbackEpoch - this.scheduledSince[existing]) >> 2;
      const priority = Math.max(0, (request.priorityTier ?? 7) - agePromotion);
      this.moveScheduled(existing, priority);
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
    this.scheduledSince[index] = this.feedbackEpoch;
    this.linkScheduledTail(index, request.priorityTier ?? 7);
    this.scheduledByKey.set(key, index);
    this.scheduledCount++;
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
      if (this.feedbackEpoch - this.scheduledLastSeen[index] >= this.staleFeedbackEpochs) {
        this.removeScheduled(index);
        this.staleCancellations++;
        continue;
      }
      const textureId = request.textureId ?? 0;
      const entry = this.entriesById[textureId];
      const pageTable = this.pageTablesById[textureId];
      if (!entry || !pageTable || (request.tail ? isResident(entry.tailEntry) : pageTable.isResident(request))) {
        this.removeScheduled(index);
        continue;
      }
      this.cache.touch(this.scheduledKeys[index]);
      if (this.queuePageLoad(entry, request, pageTable, false, priority)) { // @alloc-allowed reason=AssetFetch
        this.removeScheduled(index);
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
      if (pending.page.pinned || pending.canceled ||
          this.feedbackEpoch - pending.lastSeen < this.staleFeedbackEpochs) continue;
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

  /** Evict one logical material page across every resident PBR channel. */
  private evictPage(page: CachedPage): void {
    this.clearEvictedPage(page);
    if (page.pinned || page.tail) return;
    const group = this.materialGroupByPath.get(page.path);
    if (!group) return;
    for (const path of group) {
      if (path === page.path) continue;
      const entry = this.entries.get(path);
      if (!entry) continue;
      const key = packedPageCoordinates(entry.textureId, page.mip, page.x, page.y);
      const sibling = this.cache.evictByKey(key);
      if (!sibling) continue;
      this.clearEvictedPage(sibling);
      this.cacheEvictions++;
    }
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
    const materialGroup = this.materialGroupByPath.get(path);
    if (materialGroup) {
      for (const channelPath of materialGroup) {
        const channel = this.entries.get(channelPath);
        if (channel) this.materialGroupIdsById[channel.textureId] = null;
        this.materialGroupByPath.delete(channelPath);
      }
    }
    for (let index = 0; index < this.scheduledRequests.length; index++) {
      if (this.scheduledActive[index] !== 0 && this.scheduledRequests[index].path === path)
        this.removeScheduled(index);
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
      this.materialGroupIdsById[entry.textureId] = null;
      entry.pageTableTexture.dispose();
    }
    this.entries.delete(path);
    this.pageTables.delete(path);
    this.gpuPageTables.delete(path);
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
        this.format === FORMAT_RGBA
          ? { bytesPerRow: SLOT_SIZE * 4, rowsPerImage: SLOT_SIZE }
          : { bytesPerRow: this.cache.slotBytesPerRow, rowsPerImage: SLOT_BLOCKS_Y },
        { width: SLOT_SIZE, height: SLOT_SIZE },
      );
    } else {
      // Startup only. Once attachRenderer() succeeds, both compressed and RGBA
      // pages use the same queue.writeTexture subregion path above.
      this.atlasTexture.needsUpdate = true;
    }
  }

  /** Feed the presentation interval into the central bounded upload policy. */
  // @hot-no-alloc-begin VirtualTextureStore.recordFrameTime
  recordFrameTime(frameMs: number): void {
    this.tuning.recordFrameTime(frameMs, this.pendingCount + this.readyUploadCount + this.scheduledCount);
  }
  // @hot-no-alloc-end VirtualTextureStore.recordFrameTime

  /** Poll workers and commit a bounded number of completed uploads per frame. */
  poll() {
    this.loader.poll();
    const deadline = performance.now() + this.tuning.uploadBudgetMs;
    for (let count = 0; count < this.tuning.uploadsPerPoll && this.readyUploadCount !== 0; count++) {
      if (count !== 0 && performance.now() >= deadline) {
        this.uploadBudgetExhaustions++;
        break;
      }
      const ready = this.readyUploads[this.readyUploadHead];
      this.readyUploadHead = (this.readyUploadHead + 1) % this.readyUploads.length;
      this.readyUploadCount--;
      const pending = this.getPending(ready.key);
      if (!pending || pending.generation !== ready.generation) continue;
      if (pending.canceled) {
        this.deletePending(ready.key);
        continue;
      }
      this.deletePending(ready.key);
      const textureId = ready.page.textureId ?? 0;
      const entry = this.entriesById[textureId];
      const pageTable = this.pageTablesById[textureId];
      if (!entry || !pageTable || (ready.req.tail ? isResident(entry.tailEntry) : pageTable.isResident(ready.req))) continue;

      const uploadStartedAt = performance.now();
      let slot: PageSlot;
      try {
        const acquired = this.cache.acquire(ready.page);
        slot = acquired.slot;
        if (acquired.evicted) {
          this.evictPage(acquired.evicted);
          this.cacheEvictions++;
        }
      } catch {
        this.rejectedAdmissions++;
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
      this.completedUploads++;
      this.totalUploadMs += uploadMs;
      this.maxUploadMs = Math.max(this.maxUploadMs, uploadMs);
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
    stats.cacheHits = this.cacheHits;
    stats.cacheMisses = this.cacheMisses;
    stats.cacheEvictions = this.cacheEvictions;
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
      stats.transcodeWorkers = provider.workerCount;
      stats.activeTranscodes = provider.activeTranscodes;
      stats.queuedTranscodes = provider.queuedTranscodes;
      stats.completedTranscodes = provider.completedTranscodes;
      stats.averageTranscodeQueueMs = provider.averageTranscodeQueueMs;
      stats.maxTranscodeQueueMs = provider.maxTranscodeQueueMs;
      stats.averageTranscodeMs = provider.averageTranscodeMs;
      stats.maxTranscodeMs = provider.maxTranscodeMs;
      stats.cacheEnabled = provider.cacheEnabled;
      stats.cacheBackend = provider.cacheBackend;
      stats.cacheEntries = provider.cacheEntries;
      stats.cacheBytes = provider.cacheBytes;
      stats.cacheLiveBytes = provider.cacheLiveBytes;
      stats.cacheQueuedWrites = provider.cacheQueuedWrites;
      stats.cacheEvictions = provider.cacheEvictions;
      stats.cacheCompactions = provider.cacheCompactions;
      stats.cacheReclaimedBytes = provider.cacheReclaimedBytes;
      stats.cacheMaintenance = provider.cacheMaintenance;
      stats.cacheHits = provider.cacheHits;
      stats.cacheMisses = provider.cacheMisses;
      stats.cacheWrites = provider.cacheWrites;
      stats.cacheRejected = provider.cacheRejected;
      stats.cacheErrors = provider.cacheErrors;
      stats.averageCacheReadMs = provider.averageCacheReadMs;
      stats.maxCacheReadMs = provider.maxCacheReadMs;
      stats.averageCacheWriteMs = provider.averageCacheWriteMs;
      stats.maxCacheWriteMs = provider.maxCacheWriteMs;
    }
    return stats;
  }
  // @hot-no-alloc-end VirtualTextureStore.getStats
}

// ============================================================================
// ECS Resource
// ============================================================================

export const VirtualTextureRes = defineResource<VirtualTextureStore>('virtualTexture', () => {
  throw new Error('VirtualTextureStore not initialized. Call VirtualTextureRes.set(world, new VirtualTextureStore(loader)).');
});

// ============================================================================
// WGSL Shaders (for the material system)
// ============================================================================

/**
 * The VT sampling shader. Override material.colorNode with this.
 *
 * Source: [SHLOM] material.frag, validated in prototype.
 * Uses wgslFn() for pure WGSL — TSL handles binding plumbing.
 */
export const VT_SAMPLE_WGSL = /* wgsl */ `
fn vtSample(
  pageTable: texture_2d<u32>,
  atlas: texture_2d<f32>,
  atlasSampler: sampler,
  uv: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  pageSize: f32,
  pageBorder: f32,
  atlasSize: vec2f,
  maxMip: f32,
  textureMaxMip: f32,
  addressMode: u32
) -> vec4f {
  // 0 = clamp, 1 = repeat, 2 = mirrored repeat.
  var addressed_uv = clamp(uv, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressed_uv = fract(uv);
  } else if (addressMode == 2u) {
    let period = uv - floor(uv * 0.5) * 2.0;
    addressed_uv = select(period, 2.0 - period, period > vec2f(1.0));
    addressed_uv = clamp(addressed_uv, vec2f(0.0), vec2f(0.99999994));
  }

  // Compute desired mip level from the original continuous derivatives.
  let dx = dpdx(uv * virtualSize);
  let dy = dpdy(uv * virtualSize);
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_float = clamp(0.5 * log2(max(texel_footprint, 1e-8)), 0.0, textureMaxMip);
  let desired_level = i32(mip_float);

  // Mips below 128x128 share one pinned physical slot. The entry is stored in
  // the otherwise-unused x=1 texel of the terminal page-table row.
  if (desired_level > i32(maxMip)) {
    var tail_offset = 0.0;
    for (var level = 0; level < i32(maxMip); level = level + 1) {
      tail_offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    let tail_entry = textureLoad(pageTable, vec2i(1, i32(tail_offset)), 0).r;
    if ((tail_entry & 1u) != 0u) {
      let delta = desired_level - i32(maxMip);
      var rect_origin = vec2f(0.0);
      if (delta == 2) { rect_origin = vec2f(72.0, 0.0); }
      else if (delta == 3) { rect_origin = vec2f(112.0, 0.0); }
      else if (delta == 4) { rect_origin = vec2f(72.0, 40.0); }
      else if (delta == 5) { rect_origin = vec2f(88.0, 40.0); }
      else if (delta == 6) { rect_origin = vec2f(100.0, 40.0); }
      else if (delta >= 7) { rect_origin = vec2f(110.0, 40.0); }
      let tail_size = max(vec2f(1.0), floor(virtualSize / exp2(f32(desired_level))));
      let tail_x = (tail_entry >> 1) & 0xFFu;
      let tail_y = (tail_entry >> 9) & 0xFFu;
      let slot_origin = vec2f(f32(tail_x), f32(tail_y)) * (pageSize + pageBorder * 2.0);
      let tail_texel = slot_origin + rect_origin + pageBorder + addressed_uv * tail_size;
      let tail_uv = tail_texel / atlasSize;
      let tail_scale = tail_size / atlasSize;
      return textureSampleGrad(atlas, atlasSampler, tail_uv, dpdx(uv) * tail_scale, dpdy(uv) * tail_scale);
    }
  }

  var mip_level = min(desired_level, i32(maxMip));
  let max_level = i32(maxMip);

  // Walk from desired mip up, looking for resident page
  var is_resident = false;
  var entry = 0u;
  var curr_page_grid = vec2f(0.0);
  var curr_mip_size = virtualSize;
  var page_coords = vec2i(0);

  for (var m = mip_level; m <= max_level; m = m + 1) {
    let mip_scale = exp2(-f32(m));
    curr_page_grid = max(ceil(pageGrid * mip_scale), vec2f(1.0));
    curr_mip_size = max(floor(virtualSize * mip_scale), vec2f(1.0));
    page_coords = vec2i(min(floor(addressed_uv * curr_mip_size / pageSize), curr_page_grid - 1.0));
    var mip_offset = 0.0;
    for (var level = 0; level < m; level = level + 1) {
      mip_offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    entry = textureLoad(pageTable, vec2i(page_coords.x, page_coords.y + i32(mip_offset)), 0).r;
    if ((entry & 1u) != 0u) {
      is_resident = true;
      mip_level = m;
      break;
    }
  }

  if (!is_resident) {
    return vec4f(0.5, 0.5, 0.5, 1.0);
  }

  // Compute physical atlas UV
  let physX = (entry >> 1) & 0xFFu;
  let physY = (entry >> 9) & 0xFFu;
  let local_texel = addressed_uv * curr_mip_size - vec2f(page_coords) * pageSize;
  let page_origin = vec2f(f32(physX), f32(physY)) * (pageSize + pageBorder * 2.0);
  let sample_texel = page_origin + pageBorder + local_texel;
  let atlas_uv = sample_texel / atlasSize;

  // Atlas-space gradients preserve anisotropy without allowing the GPU to
  // derive across an unrelated neighboring physical slot.
  let gradient_scale = curr_mip_size / atlasSize;
  let atlas_dx = dpdx(uv) * gradient_scale;
  let atlas_dy = dpdy(uv) * gradient_scale;
  return textureSampleGrad(atlas, atlasSampler, atlas_uv, atlas_dx, atlas_dy);
}
`;

/** Resolve one fallback level that is resident in all four PBR channels. */
export const VT_RESOLVE_MATERIAL_MIP4_WGSL = /* wgsl */ `
fn vtResolveMaterialMip4(
  pageTable0: texture_2d<u32>,
  pageTable1: texture_2d<u32>,
  pageTable2: texture_2d<u32>,
  pageTable3: texture_2d<u32>,
  uv: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  pageSize: f32,
  maxMip: f32,
  textureMaxMip: f32,
  addressMode: u32
) -> f32 {
  var addressed_uv = clamp(uv, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressed_uv = fract(uv);
  } else if (addressMode == 2u) {
    let period = uv - floor(uv * 0.5) * 2.0;
    addressed_uv = select(period, 2.0 - period, period > vec2f(1.0));
    addressed_uv = clamp(addressed_uv, vec2f(0.0), vec2f(0.99999994));
  }

  let dx = dpdx(uv * virtualSize);
  let dy = dpdy(uv * virtualSize);
  let footprint = max(dot(dx, dx), dot(dy, dy));
  let desired = i32(clamp(0.5 * log2(max(footprint, 1e-8)), 0.0, textureMaxMip));
  let max_level = i32(maxMip);

  if (desired > max_level) {
    var tail_offset = 0.0;
    for (var level = 0; level < max_level; level = level + 1) {
      tail_offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    let coord = vec2i(1, i32(tail_offset));
    let e0 = textureLoad(pageTable0, coord, 0).r;
    let e1 = textureLoad(pageTable1, coord, 0).r;
    let e2 = textureLoad(pageTable2, coord, 0).r;
    let e3 = textureLoad(pageTable3, coord, 0).r;
    if ((e0 & 1u) != 0u && (e1 & 1u) != 0u &&
        (e2 & 1u) != 0u && (e3 & 1u) != 0u) {
      return f32(desired);
    }
  }

  for (var mip = min(desired, max_level); mip <= max_level; mip = mip + 1) {
    let scale = exp2(-f32(mip));
    let grid = max(ceil(pageGrid * scale), vec2f(1.0));
    let mip_size = max(floor(virtualSize * scale), vec2f(1.0));
    let page = vec2i(min(floor(addressed_uv * mip_size / pageSize), grid - 1.0));
    var offset = 0.0;
    for (var level = 0; level < mip; level = level + 1) {
      offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    let coord = vec2i(page.x, page.y + i32(offset));
    let e0 = textureLoad(pageTable0, coord, 0).r;
    let e1 = textureLoad(pageTable1, coord, 0).r;
    let e2 = textureLoad(pageTable2, coord, 0).r;
    let e3 = textureLoad(pageTable3, coord, 0).r;
    if ((e0 & 1u) != 0u && (e1 & 1u) != 0u &&
        (e2 & 1u) != 0u && (e3 & 1u) != 0u) {
      return f32(mip);
    }
  }
  return maxMip;
}
`;

/** Sample exactly one level chosen by a shared material fallback resolve. */
export const VT_SAMPLE_LEVEL_WGSL = /* wgsl */ `
fn vtSampleLevel(
  pageTable: texture_2d<u32>,
  atlas: texture_2d<f32>,
  atlasSampler: sampler,
  uv: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  pageSize: f32,
  pageBorder: f32,
  atlasSize: vec2f,
  maxMip: f32,
  resolvedMip: f32,
  addressMode: u32
) -> vec4f {
  var addressed_uv = clamp(uv, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressed_uv = fract(uv);
  } else if (addressMode == 2u) {
    let period = uv - floor(uv * 0.5) * 2.0;
    addressed_uv = select(period, 2.0 - period, period > vec2f(1.0));
    addressed_uv = clamp(addressed_uv, vec2f(0.0), vec2f(0.99999994));
  }

  let mip = i32(resolvedMip);
  if (mip > i32(maxMip)) {
    var tail_offset = 0.0;
    for (var level = 0; level < i32(maxMip); level = level + 1) {
      tail_offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    let entry = textureLoad(pageTable, vec2i(1, i32(tail_offset)), 0).r;
    if ((entry & 1u) == 0u) { return vec4f(0.5, 0.5, 0.5, 1.0); }
    let delta = mip - i32(maxMip);
    var rect_origin = vec2f(0.0);
    if (delta == 2) { rect_origin = vec2f(72.0, 0.0); }
    else if (delta == 3) { rect_origin = vec2f(112.0, 0.0); }
    else if (delta == 4) { rect_origin = vec2f(72.0, 40.0); }
    else if (delta == 5) { rect_origin = vec2f(88.0, 40.0); }
    else if (delta == 6) { rect_origin = vec2f(100.0, 40.0); }
    else if (delta >= 7) { rect_origin = vec2f(110.0, 40.0); }
    let tail_size = max(vec2f(1.0), floor(virtualSize / exp2(f32(mip))));
    let px = (entry >> 1) & 0xFFu;
    let py = (entry >> 9) & 0xFFu;
    let slot = vec2f(f32(px), f32(py)) * (pageSize + pageBorder * 2.0);
    let texel = slot + rect_origin + pageBorder + addressed_uv * tail_size;
    let scale = tail_size / atlasSize;
    return textureSampleGrad(atlas, atlasSampler, texel / atlasSize, dpdx(uv) * scale, dpdy(uv) * scale);
  }

  let mip_scale = exp2(-f32(mip));
  let grid = max(ceil(pageGrid * mip_scale), vec2f(1.0));
  let mip_size = max(floor(virtualSize * mip_scale), vec2f(1.0));
  let page = vec2i(min(floor(addressed_uv * mip_size / pageSize), grid - 1.0));
  var offset = 0.0;
  for (var level = 0; level < mip; level = level + 1) {
    offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
  }
  let entry = textureLoad(pageTable, vec2i(page.x, page.y + i32(offset)), 0).r;
  if ((entry & 1u) == 0u) { return vec4f(0.5, 0.5, 0.5, 1.0); }
  let px = (entry >> 1) & 0xFFu;
  let py = (entry >> 9) & 0xFFu;
  let local = addressed_uv * mip_size - vec2f(page) * pageSize;
  let origin = vec2f(f32(px), f32(py)) * (pageSize + pageBorder * 2.0);
  let atlas_uv = (origin + pageBorder + local) / atlasSize;
  let gradient_scale = mip_size / atlasSize;
  return textureSampleGrad(
    atlas, atlasSampler, atlas_uv,
    dpdx(uv) * gradient_scale,
    dpdy(uv) * gradient_scale
  );
}
`;

/** Sample a displaced UV from a resolved level, walking to coarser pages. */
export const VT_SAMPLE_FROM_LEVEL_WGSL = /* wgsl */ `
fn vtSampleFromLevel(
  pageTable: texture_2d<u32>, atlas: texture_2d<f32>, atlasSampler: sampler,
  sampleUV: vec2f, gradientUV: vec2f,
  virtualSize: vec2f, pageGrid: vec2f, pageSize: f32, pageBorder: f32,
  atlasSize: vec2f, maxMip: f32, resolvedMip: f32, addressMode: u32
) -> vec4f {
  var addressedUV = clamp(sampleUV, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressedUV = fract(sampleUV);
  } else if (addressMode == 2u) {
    let period = sampleUV - floor(sampleUV * 0.5) * 2.0;
    addressedUV = select(period, 2.0 - period, period > vec2f(1.0));
    addressedUV = clamp(addressedUV, vec2f(0.0), vec2f(0.99999994));
  }

  let requested = i32(resolvedMip);
  let maxLevel = i32(maxMip);
  var entry = 0u;
  var selected = -1;
  var selectedPage = vec2i(0);
  var selectedSize = vec2f(1.0);
  if (requested <= maxLevel) {
    for (var mip = max(0, requested); mip <= maxLevel; mip = mip + 1) {
      let scale = exp2(-f32(mip));
      let grid = max(ceil(pageGrid * scale), vec2f(1.0));
      let mipSize = max(floor(virtualSize * scale), vec2f(1.0));
      let page = vec2i(min(floor(addressedUV * mipSize / pageSize), grid - 1.0));
      var offset = 0.0;
      for (var level = 0; level < mip; level = level + 1) {
        offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
      }
      let candidate = textureLoad(pageTable, vec2i(page.x, page.y + i32(offset)), 0).r;
      if ((candidate & 1u) != 0u) {
        entry = candidate; selected = mip; selectedPage = page; selectedSize = mipSize;
        break;
      }
    }
  }

  if (selected >= 0) {
    let local = addressedUV * selectedSize - vec2f(selectedPage) * pageSize;
    let origin = vec2f(f32((entry >> 1) & 0xFFu), f32((entry >> 9) & 0xFFu)) * (pageSize + pageBorder * 2.0);
    let atlasUV = (origin + pageBorder + local) / atlasSize;
    let gradientScale = selectedSize / atlasSize;
    return textureSampleGrad(atlas, atlasSampler, atlasUV,
      dpdx(gradientUV) * gradientScale, dpdy(gradientUV) * gradientScale);
  }

  var tailOffset = 0.0;
  for (var level = 0; level < maxLevel; level = level + 1) {
    tailOffset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
  }
  let tailEntry = textureLoad(pageTable, vec2i(1, i32(tailOffset)), 0).r;
  if ((tailEntry & 1u) == 0u) { return vec4f(0.5, 0.5, 0.5, 1.0); }
  let tailMip = max(maxLevel + 1, requested);
  let delta = tailMip - maxLevel;
  var rectOrigin = vec2f(0.0);
  if (delta == 2) { rectOrigin = vec2f(72.0, 0.0); }
  else if (delta == 3) { rectOrigin = vec2f(112.0, 0.0); }
  else if (delta == 4) { rectOrigin = vec2f(72.0, 40.0); }
  else if (delta == 5) { rectOrigin = vec2f(88.0, 40.0); }
  else if (delta == 6) { rectOrigin = vec2f(100.0, 40.0); }
  else if (delta >= 7) { rectOrigin = vec2f(110.0, 40.0); }
  let tailSize = max(vec2f(1.0), floor(virtualSize / exp2(f32(tailMip))));
  let slot = vec2f(f32((tailEntry >> 1) & 0xFFu), f32((tailEntry >> 9) & 0xFFu)) * (pageSize + pageBorder * 2.0);
  let atlasUV = (slot + rectOrigin + pageBorder + addressedUV * tailSize) / atlasSize;
  let gradientScale = tailSize / atlasSize;
  return textureSampleGrad(atlas, atlasSampler, atlasUV,
    dpdx(gradientUV) * gradientScale, dpdy(gradientUV) * gradientScale);
}
`;

/**
 * The feedback shader. Renders to a low-res target, writing page IDs.
 *
 * Source: [SHLOM] feedback.frag, validated in prototype.
 */
export const VT_FEEDBACK_WGSL = /* wgsl */ `
fn vtFeedback(
  sampleUV: vec2f,
  gradientUV: vec2f,
  feedbackPixelScale: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  maxMip: f32,
  qualityBias: f32,
  addressMode: u32,
  textureId: u32
) -> vec2u {
  // Derivatives are measured per reduced-resolution feedback pixel. Convert
  // them back to physical display-pixel derivatives before selecting a mip.
  // Keeping gradientUV separate prevents repeat/POM discontinuities from
  // corrupting the screen-space footprint.
  let dx = dpdx(gradientUV * virtualSize) * feedbackPixelScale.x;
  let dy = dpdy(gradientUV * virtualSize) * feedbackPixelScale.y;
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_level = u32(clamp(0.5 * log2(max(texel_footprint, 1e-8)) + qualityBias, 0.0, maxMip));

  var addressed_uv = clamp(sampleUV, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressed_uv = fract(sampleUV);
  } else if (addressMode == 2u) {
    let period = sampleUV - floor(sampleUV * 0.5) * 2.0;
    addressed_uv = select(period, 2.0 - period, period > vec2f(1.0));
    addressed_uv = clamp(addressed_uv, vec2f(0.0), vec2f(0.99999994));
  }
  let mip_scale = exp2(-f32(mip_level));
  let curr_page_grid = max(ceil(pageGrid * mip_scale), vec2f(1.0));
  let mip_size = max(floor(virtualSize * mip_scale), vec2f(1.0));
  let page_coords = min(floor(addressed_uv * mip_size / 128.0), curr_page_grid - 1.0);

  // RG32Uint: word 0 carries valid + 6-bit mip + 11-bit X/Y;
  // word 1 carries the full virtual-texture identity.
  let packed = 0x80000000u |
               (mip_level & 0x3Fu) |
               ((u32(page_coords.x) & 0x7FFu) << 6) |
               ((u32(page_coords.y) & 0x7FFu) << 17);
  return vec2u(packed, textureId);
}
`;
