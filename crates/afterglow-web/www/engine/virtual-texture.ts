// VirtualTextureStore — manages the shared physical atlas, page table,
// LRU cache, and smart LOD strategy for all virtual textures.
//
// ALL sampled textures in the engine go through this store. There are no
// separate "normal" textures — everything is a page in the atlas.
//
// Architecture:
//   .big file → [page chunks at seekable offsets]
//                    ↓
//   AssetLoader.read(path, offset, len) → raw page data
//                    ↓
//   VirtualTextureStore.poll() → strategy decides priority → copy to atlas
//                    ↓
//   Page table texture (updated) → GPU shader samples via vtSample()
//
// The atlas and page table are THREE.DataTextures shared across all materials.
// Each virtual texture has its own page table binding (UV offset + scale).

import * as THREE from 'three';
import { AssetHandle } from './asset-handle.js';
import { Resource, defineResource } from './resource.js';
import {
  PackedPageTableLayout,
  assertVirtualTextureSize,
  createPackedPageTableLayout,
  packedMipTailIndex,
  packedPageTableIndex,
} from './virtual-texture-layout.js';

// ============================================================================
// Constants
// ============================================================================

const PAGE_SIZE = 128;           // texels per page (payload, excluding border)
const PAGE_BORDER = 4;           // border texels PER SIDE (for bilinear/aniso)
const SLOT_SIZE = PAGE_SIZE + PAGE_BORDER * 2; // 136 texels per physical slot
const ATLAS_PAGES_X = 15;        // 15×15 = 225 page slots
const ATLAS_PAGES_Y = 15;
const ATLAS_WIDTH = ATLAS_PAGES_X * SLOT_SIZE;   // 2040
const ATLAS_HEIGHT = ATLAS_PAGES_Y * SLOT_SIZE;   // 2040
const MAX_MIP = 10;              // supports up to 2^10 = 1024 pages per side
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
  /** Virtual-texture path/ID owning this page. */
  path: string;
}

interface CachedPage extends VirtualPageRequest {
  /** Pinned pages are never selected for eviction. */
  pinned: boolean;
}

/** A physical page slot in the atlas. */
interface PageSlot {
  x: number;
  y: number;
}

/** Page table entry — bit-packed u32 (matches WGSL shader format). */
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
const ATLAS_BLOCKS_X = ATLAS_WIDTH / BLOCK_SIZE; // 510
const ATLAS_BLOCKS_Y = ATLAS_HEIGHT / BLOCK_SIZE; // 510

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
function threeFormat(format: number): number {
  if (format === FORMAT_BC7) return 36492;      // RGBA_BPTC_Format
  if (format === FORMAT_ASTC) return 37808;      // RGBA_ASTC_4x4_Format
  return THREE.RGBAFormat;
}

/** A virtual texture descriptor — created by loadTexture(). */
export interface VirtualTextureEntry {
  /** Stable u32 identity written into RG32Uint feedback. */
  textureId: number;
  /** Path in the .big file (or loader key). */
  path: string;
  /** Virtual texture size in texels (e.g., 4096 for a 4096×4096 texture). */
  virtualSize: number;
  /** Number of pages per side at mip 0 (virtualSize / PAGE_SIZE). */
  pageGrid: number;
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

/** Per-page tracking state (for the smart strategy). */
interface PageState {
  request: VirtualPageRequest;
  hitCount: number;
  consecutiveFrames: number;
  lastSeenFrame: number;
  lastMipSwitch: number;
  isPredicted: boolean;
  residentMip: number;
}

// ============================================================================
// Page Table
// ============================================================================

class PageTable {
  private entries = new Map<string, number>();
  private maxMip: number;

  constructor(maxMip: number) {
    this.maxMip = maxMip;
  }

  private key(req: PageRequest): string {
    return `${req.mip}:${req.x}:${req.y}`;
  }

  get(req: PageRequest): number {
    return this.entries.get(this.key(req)) ?? 0;
  }

  setResident(req: PageRequest, slot: PageSlot) {
    this.entries.set(this.key(req), packEntry(true, slot.x, slot.y));
  }

  setEvicted(req: PageRequest) {
    this.entries.delete(this.key(req));
  }

  isResident(req: PageRequest): boolean {
    return isResident(this.get(req));
  }

  /**
   * Find the best resident page for a virtual UV, walking from desired mip
   * up to coarser mips. Returns the entry and the mip level found.
   *
   * Source: [SHLOM] material.frag fallback loop.
   */
  findResidentPage(u: number, v: number, desiredMip: number, pageGrid: number): { entry: number; mip: number } | null {
    for (let mip = desiredMip; mip <= this.maxMip; mip++) {
      const pagesAtMip = pageGrid >> mip;
      const px = Math.min(Math.floor(u * pagesAtMip), pagesAtMip - 1);
      const py = Math.min(Math.floor(v * pagesAtMip), pagesAtMip - 1);
      const entry = this.get({ mip, x: px, y: py });
      if (isResident(entry)) {
        return { entry, mip };
      }
    }
    return null;
  }

  get residentCount(): number { return this.entries.size; }
}

// ============================================================================
// Page Cache (Physical Atlas + LRU)
// ============================================================================

class PageCache {
  /** Atlas pixel data (compressed blocks or RGBA8). */
  atlas: Uint8Array;
  /** Bytes per row in the atlas (for the selected format). */
  atlasBytesPerRow: number;
  /** Bytes per slot row. */
  slotBytesPerRow: number;
  /** Slot data size in bytes (for one page). */
  slotDataSize: number;
  /** Which globally identified virtual page is in each slot. */
  private slots: (CachedPage | null)[] = [];
  /** Free slot indices. */
  private freeSlots: number[] = [];
  /** LRU list: front = MRU, back = LRU. */
  private lru: CachedPage[] = [];
  /** Map: page key → index in LRU. */
  private lruMap = new Map<string, number>();
  constructor(format: number = FORMAT_RGBA) {
    const bpb = bytesPerBlock(format);
    this.atlasBytesPerRow = ATLAS_BLOCKS_X * bpb;
    this.slotBytesPerRow = SLOT_BLOCKS_X * bpb;
    this.slotDataSize = SLOT_BLOCKS_X * SLOT_BLOCKS_Y * bpb;
    this.atlas = new Uint8Array(ATLAS_BLOCKS_X * ATLAS_BLOCKS_Y * bpb);

    for (let y = 0; y < ATLAS_PAGES_Y; y++) {
      for (let x = 0; x < ATLAS_PAGES_X; x++) {
        this.slots.push(null);
        this.freeSlots.push(y * ATLAS_PAGES_X + x);
      }
    }
  }

  private lruKey(req: VirtualPageRequest): string {
    return `${req.path}:${req.tail ? 'tail' : req.mip}:${req.x}:${req.y}`;
  }

  /** Mark a page as recently used (move to front of LRU). */
  touch(req: VirtualPageRequest) {
    const resident = this.slots.find(page => page !== null && this.lruKey(page) === this.lruKey(req));
    if (resident?.pinned) return;
    const key = this.lruKey(req);
    const idx = this.lruMap.get(key);
    if (idx !== undefined) {
      this.lru.splice(idx, 1);
      this.lru.unshift(req);
      this.rebuildLruMap();
    }
  }

  /** Acquire a free slot, evicting LRU if necessary. */
  acquire(req: CachedPage): { slot: PageSlot; evicted: CachedPage | null } {
    // Try free slot
    if (this.freeSlots.length > 0) {
      const idx = this.freeSlots.pop()!;
      const slot = { x: idx % ATLAS_PAGES_X, y: Math.floor(idx / ATLAS_PAGES_X) };
      return { slot, evicted: null };
    }

    // Evict LRU (from back, skip pinned)
    let evictIdx = -1;
    for (let i = this.lru.length - 1; i >= 0; i--) {
      if (!this.lru[i].pinned) {
        evictIdx = i;
        break;
      }
    }

    if (evictIdx === -1) {
      throw new Error('No evictable slots available (all pages pinned)');
    }

    const evictedReq = this.lru[evictIdx];
    const slotIdx = this.slots.findIndex(s =>
      s !== null && this.lruKey(s) === this.lruKey(evictedReq)
    );
    if (slotIdx === -1) throw new Error('Slot not found for evicted page');

    this.lru.splice(evictIdx, 1);
    this.lruMap.delete(this.lruKey(evictedReq));
    this.slots[slotIdx] = null;

    const slot = { x: slotIdx % ATLAS_PAGES_X, y: Math.floor(slotIdx / ATLAS_PAGES_X) };
    return { slot, evicted: evictedReq };
  }

  /** Write page data into a slot and mark as resident. */
  commit(req: CachedPage, slot: PageSlot, data: Uint8Array) {
    const dstBlockX = slot.x * SLOT_BLOCKS_X;
    const dstBlockY = slot.y * SLOT_BLOCKS_Y;

    // Copy page data into atlas at slot position (block-aligned)
    for (let row = 0; row < SLOT_BLOCKS_Y; row++) {
      const srcOffset = row * this.slotBytesPerRow;
      const dstOffset = (dstBlockY + row) * this.atlasBytesPerRow + dstBlockX * (this.slotBytesPerRow / SLOT_BLOCKS_X);
      for (let i = 0; i < this.slotBytesPerRow; i++) {
        this.atlas[dstOffset + i] = data[srcOffset + i];
      }
    }

    const slotIdx = slot.y * ATLAS_PAGES_X + slot.x;
    this.slots[slotIdx] = req;

    const freeIdx = this.freeSlots.indexOf(slotIdx);
    if (freeIdx >= 0) this.freeSlots.splice(freeIdx, 1);

    if (!req.pinned) {
      this.lru.unshift(req);
      this.rebuildLruMap();
    }
  }

  /** Return an uncommitted reservation to the free list after load failure. */
  release(slot: PageSlot): void {
    const index = slot.y * ATLAS_PAGES_X + slot.x;
    if (this.slots[index] === null && !this.freeSlots.includes(index)) this.freeSlots.push(index);
  }

  private rebuildLruMap() {
    this.lruMap.clear();
    for (let i = 0; i < this.lru.length; i++) {
      this.lruMap.set(this.lruKey(this.lru[i]), i);
    }
  }

  /** Remove every resident page owned by one virtual texture. */
  removeTexture(path: string): void {
    for (let index = 0; index < this.slots.length; index++) {
      const page = this.slots[index];
      if (page?.path !== path) continue;
      this.slots[index] = null;
      if (!this.freeSlots.includes(index)) this.freeSlots.push(index);
    }
    this.lru = this.lru.filter(page => page.path !== path);
    this.rebuildLruMap();
  }

  get usedSlots(): number { return this.slots.filter(s => s !== null).length; }
  get freeSlotCount(): number { return this.freeSlots.length; }
  get totalSlots(): number { return ATLAS_PAGES_X * ATLAS_PAGES_Y; }
}

// ============================================================================
// Virtual Texture Store
// ============================================================================

/**
 * Manages the shared physical atlas, page tables, LRU cache, and smart LOD
 * strategy for all virtual textures.
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
  private entriesById = new Map<number, VirtualTextureEntry>();
  private nextTextureId = 1;

  /** Per-path page tables (for lookup). */
  private pageTables = new Map<string, PageTable>();

  /** The shared page cache (atlas + LRU). */
  private cache: PageCache;

  /** The asset loader (for reading page data from .big). */
  private loader: { read(path: string, offset: number, len: number): Promise<Uint8Array>; poll(): void };

  /** Page data loader (returns raw page bytes for a given request). */
  private pageDataProvider?: (path: string, req: PageRequest) => Promise<Uint8Array>;

  /** Per-page tracking state (for strategy). */
  private pageStates = new Map<string, PageState>();

  /** In-flight loads reserve a unique slot and deduplicate repeated feedback. */
  private pendingPages = new Map<string, { generation: number; slot: PageSlot; page: CachedPage }>();
  private loadGeneration = 0;

  /** Frame counter. */
  private frame = 0;

  /** Camera history for prediction. */
  private cameraHistory: Array<{ pos: [number, number]; zoom: number }> = [];

  /** Frame time history (for adaptive quality). */
  private frameTimes: number[] = [];

  // Adaptive state (separate biases combined via max)
  private frameTimeLodBias = 0;
  private oversubscriptionLodBias = 0;
  private currentBudget = 8;
  private debugPaused = false;
  private debugPageBudget: number | null = null;

  // Config
  private config = {
    maxPagesPerFrame: 8,
    maxBudget: 16,
    hysteresisFrames: 3,
    hysteresisFactor: 0.3,
    predictionEnabled: true,
    predictionFrames: 2,
    predictionPadding: 0.1,
    adaptiveQualityEnabled: true,
    targetFrameTime: 16.67,
    adaptiveLodBiasStep: 0.5,
    adaptiveBudgetStep: 1,
    highWaterMark: 0.9,
    lowWaterMark: 0.5,
    weightMipDistance: 1.0,
    weightHitCount: 0.5,
    weightScreenCoverage: 0.3,
    weightCenterBias: 0.2,
    weightPrediction: 0.3,
    weightConfidence: 0.4,
    evictionGraceFrames: 3,
  };

  constructor(
    loader: { read(path: string, offset: number, len: number): Promise<Uint8Array>; poll(): void },
    pageDataProvider?: (path: string, req: PageRequest) => Promise<Uint8Array>,
    format?: number,
    device?: GPUDevice,
  ) {
    this.loader = loader;
    this.pageDataProvider = pageDataProvider;
    this.format = format ?? FORMAT_RGBA;
    this.device = device ?? null;
    this.cache = new PageCache(this.format);

    // Create the shared atlas texture
    if (this.format === FORMAT_RGBA) {
      // Uncompressed: use DataTexture (simple, works everywhere)
      this.atlasTexture = new THREE.DataTexture(
        this.cache.atlas,
        ATLAS_WIDTH,
        ATLAS_HEIGHT,
        THREE.RGBAFormat,
      );
      (this.atlasTexture as THREE.DataTexture).minFilter = THREE.LinearFilter;
      (this.atlasTexture as THREE.DataTexture).magFilter = THREE.LinearFilter;
      (this.atlasTexture as THREE.DataTexture).generateMipmaps = false;
      this.atlasTexture.needsUpdate = true;
    } else {
      // Compressed (BC7/ASTC): use CompressedTexture + raw GPUTexture
      const tex = new THREE.CompressedTexture(
        [{ data: this.cache.atlas, width: ATLAS_WIDTH, height: ATLAS_HEIGHT }],
        ATLAS_WIDTH,
        ATLAS_HEIGHT,
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
    if (ATLAS_WIDTH > limit || ATLAS_HEIGHT > limit)
      throw new RangeError(`VT atlas ${ATLAS_WIDTH}x${ATLAS_HEIGHT} exceeds GPU limit ${limit}`);
    this.gpuAtlasTexture = atlas;
    this.gpuPageTables.clear();
    for (const [path, entry] of this.entries) {
      if (entry.pageTableLayout.width > limit || entry.pageTableLayout.height > limit)
        throw new RangeError(`VT page table ${path} exceeds GPU 2D texture limit ${limit}`);
      const texture = renderer.backend.get(entry.pageTableTexture).texture;
      if (!texture) throw new Error(`VT page table for ${path} has not been initialized by Three.js`);
      this.gpuPageTables.set(path, texture);
    }
  }

  getLodBias(): number { return Math.max(this.frameTimeLodBias, this.oversubscriptionLodBias); }
  getBudget(): number { return this.currentBudget; }

  /** Record camera state for prediction. Call every frame. */
  recordCamera(pos: [number, number], zoom: number) {
    this.cameraHistory.push({ pos: [...pos], zoom });
    if (this.cameraHistory.length > 10) this.cameraHistory.shift();
  }

  /** Record frame time for adaptive quality. Call every frame. */
  recordFrameTime(ms: number) {
    this.frameTimes.push(ms);
    if (this.frameTimes.length > 30) this.frameTimes.shift();
  }

  private getCameraVelocity(): [number, number] {
    if (this.cameraHistory.length < 2) return [0, 0];
    const last = this.cameraHistory[this.cameraHistory.length - 1];
    const prev = this.cameraHistory[this.cameraHistory.length - 2];
    return [last.pos[0] - prev.pos[0], last.pos[1] - prev.pos[1]];
  }

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
    options?: { virtualSize?: number; mipTail?: boolean },
  ): AssetHandle<THREE.Texture> {
    const virtualSize = options?.virtualSize ?? 4096;
    assertVirtualTextureSize(virtualSize, PAGE_SIZE);
    const pageGrid = virtualSize / PAGE_SIZE;
    const pageTableLayout = createPackedPageTableLayout(pageGrid);
    const maxMip = pageTableLayout.maxMip;

    const pageTable = new PageTable(maxMip);
    this.pageTables.set(path, pageTable);

    // Three.js cannot upload a custom integer mip chain reliably. Pack all
    // virtual mip tables vertically into mip zero of one r32uint texture.
    const pageTableData = new Uint32Array(pageTableLayout.storageWidth * pageTableLayout.height);
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
      virtualSize,
      pageGrid,
      maxMip,
      textureMaxMip: Math.log2(virtualSize),
      tailFirstMip: options?.mipTail ? maxMip + 1 : null,
      tailEntry: 0,
      pageTableLayout,
      pageTable: pageTableData,
      pageTableTexture,
    };
    this.entries.set(path, entry);
    this.entriesById.set(textureId, entry);

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

  /** Pre-load pinned (coarsest) pages. */
  private loadPinnedPages(path: string, entry: VirtualTextureEntry) {
    const pageTable = this.pageTables.get(path)!;
    const pinnedMips = entry.maxMip === 0 ? [0] : [entry.maxMip, entry.maxMip - 1];
    for (const mip of pinnedMips) {
      const pagesAtMip = entry.pageGrid >> mip;
      for (let y = 0; y < pagesAtMip; y++) {
        for (let x = 0; x < pagesAtMip; x++) {
          const req: PageRequest = { mip, x, y };
          this.loadPage(path, req, pageTable, true);
        }
      }
    }
  }

  private pageKey(req: VirtualPageRequest): string {
    return `${req.path}:${req.tail ? 'tail' : req.mip}:${req.x}:${req.y}`;
  }

  private isValidRequest(path: string, req: PageRequest): boolean {
    const entry = this.entries.get(path);
    if (!entry || !Number.isInteger(req.mip) || !Number.isInteger(req.x) || !Number.isInteger(req.y)) return false;
    if (req.tail) return entry.tailFirstMip === req.mip && req.x === 0 && req.y === 0;
    if (req.mip < 0 || req.mip > entry.maxMip) return false;
    const pages = Math.max(1, entry.pageGrid >> req.mip);
    return req.x >= 0 && req.y >= 0 && req.x < pages && req.y < pages;
  }

  private isRequestResident(path: string, req: PageRequest, pageTable: PageTable): boolean {
    if (req.tail) return isResident(this.entries.get(path)?.tailEntry ?? 0);
    return pageTable.isResident(req);
  }

  /** Queue one deduplicated asynchronous page load with a reserved slot. */
  private queuePageLoad(path: string, req: PageRequest, pageTable: PageTable, pinned = false): boolean {
    if (!this.pageDataProvider || this.isRequestResident(path, req, pageTable) || !this.isValidRequest(path, req)) return false;
    const page: CachedPage = { path, ...req, pinned };
    const key = this.pageKey(page);
    if (this.pendingPages.has(key)) return false;

    let slot: PageSlot;
    try {
      const acquired = this.cache.acquire(page);
      slot = acquired.slot;
      if (acquired.evicted) this.evictPage(acquired.evicted);
    } catch {
      return false;
    }

    const generation = ++this.loadGeneration;
    this.pendingPages.set(key, { generation, slot, page });
    this.pageDataProvider(path, req).then(data => {
      if (data.byteLength !== this.cache.slotDataSize) {
        throw new RangeError(`VT page ${key} has ${data.byteLength} bytes; expected ${this.cache.slotDataSize}`);
      }
      const pending = this.pendingPages.get(key);
      if (!pending || pending.generation !== generation) return;
      this.pendingPages.delete(key);
      this.cache.commit(page, slot, data);
      this.writePage(slot, data);
      if (req.tail) {
        this.updateMipTailEntry(path, slot);
      } else {
        pageTable.setResident(req, slot);
        this.updatePageTableTexture(path, req, slot);
      }
    }).catch(error => {
      const pending = this.pendingPages.get(key);
      if (pending?.generation === generation) {
        this.pendingPages.delete(key);
        this.cache.release(slot);
      }
      console.error(`[VT] Failed to load page ${path} mip=${req.mip} (${req.x},${req.y}):`, error);
    });
    return true;
  }

  /** Load a single page asynchronously (used for pinned startup pages). */
  private loadPage(path: string, req: PageRequest, pageTable: PageTable, pinned = false): void {
    this.queuePageLoad(path, req, pageTable, pinned);
  }

  /**
   * Process feedback from the GPU. This is the main per-frame VT update.
   *
   * @param feedback Map of page requests from the feedback pass
   * @param cameraPos Current camera position (for prediction)
   * @param cameraZoom Current camera zoom (for prediction)
   */
  processFeedback(
    feedback: Map<string, VirtualPageRequest>,
    cameraPos?: [number, number],
    cameraZoom?: number,
  ) {
    this.frame++;

    if (cameraPos && cameraZoom) {
      this.recordCamera(cameraPos, cameraZoom);
    }

    // Touch all resident pages seen in feedback
    for (const req of feedback.values()) {
      if (!this.isValidRequest(req.path, req)) continue;
      this.cache.touch(req);
    }

    // Update adaptive quality
    const atlasUsage = this.cache.usedSlots / this.cache.totalSlots;
    this.updateAdaptiveQuality(atlasUsage);

    // Update page states from feedback. Prediction flags are frame-local.
    for (const state of this.pageStates.values()) state.isPredicted = false;
    const activeKeys = new Set<string>();
    for (const req of feedback.values()) {
      if (!this.isValidRequest(req.path, req)) continue;
      const key = `${req.path}:${req.mip}:${req.x}:${req.y}`;
      let state = this.pageStates.get(key);
      if (!state) {
        state = {
          request: req,
          hitCount: 0,
          consecutiveFrames: 0,
          lastSeenFrame: this.frame,
          lastMipSwitch: 0,
          isPredicted: false,
          residentMip: -1,
        };
        this.pageStates.set(key, state);
      }
      state.hitCount++;
      state.lastSeenFrame = this.frame;
      state.consecutiveFrames++;
      activeKeys.add(key);
    }

    // Predict pages in the camera's direction of travel. Camera coordinates
    // are normalized virtual UVs, so velocity scales directly by mip grid.
    if (this.config.predictionEnabled) {
      const [velocityX, velocityY] = this.getCameraVelocity();
      if (velocityX !== 0 || velocityY !== 0) {
        const sourceStates = [...this.pageStates.values()].filter(state => activeKeys.has(this.pageKey(state.request)));
        for (const source of sourceStates) {
          const entry = this.entries.get(source.request.path);
          if (!entry) continue;
          const pages = Math.max(1, entry.pageGrid >> source.request.mip);
          const x = Math.max(0, Math.min(pages - 1,
            Math.round(source.request.x + velocityX * pages * this.config.predictionFrames)));
          const y = Math.max(0, Math.min(pages - 1,
            Math.round(source.request.y + velocityY * pages * this.config.predictionFrames)));
          const request: VirtualPageRequest = { ...source.request, x, y };
          const key = this.pageKey(request);
          if (activeKeys.has(key)) continue;
          const existing = this.pageStates.get(key);
          if (existing) {
            existing.isPredicted = true;
            existing.lastSeenFrame = this.frame;
          } else {
            this.pageStates.set(key, {
              request, hitCount: 0, consecutiveFrames: 0,
              lastSeenFrame: this.frame, lastMipSwitch: this.frame,
              isPredicted: true, residentMip: -1,
            });
          }
        }
      }
    }

    // Decay consecutive frames for pages not in feedback
    for (const [key, state] of this.pageStates) {
      if (!activeKeys.has(key)) {
        state.consecutiveFrames = 0;
      }
    }

    // Compute priority against the request's owning page table only.
    const toLoad: Array<{ req: VirtualPageRequest; priority: number }> = [];

    for (const state of this.pageStates.values()) {
      const pt = this.pageTables.get(state.request.path);
      if (!pt) continue;
      if (this.isRequestResident(state.request.path, state.request, pt)) {
        state.residentMip = state.request.mip;
        continue;
      }
      state.residentMip = -1;
      toLoad.push({ req: state.request, priority: this.computePriority(state) });
    }

    // Sort by priority (highest first)
    toLoad.sort((a, b) => b.priority - a.priority);

    // Load pages up to budget
    let loaded = 0;
    let evicted = 0;
    const budget = this.debugPaused ? 0 : Math.min(this.currentBudget, this.debugPageBudget ?? Number.MAX_SAFE_INTEGER);

    for (const { req } of toLoad) {
      if (loaded >= budget) break;
      const path = req.path;
      const pt = this.pageTables.get(path);
      if (!pt) continue;

      const before = this.cache.usedSlots;
      if (this.queuePageLoad(path, req, pt, false)) {
        loaded++;
        // An occupied cache can only reserve a slot by evicting one page.
        if (before === this.cache.totalSlots) evicted++;
      }
    }

    // Evict pages not seen for graceFrames
    const toEvict: VirtualPageRequest[] = [];
    for (const [key, state] of this.pageStates) {
      const framesSinceSeen = this.frame - state.lastSeenFrame;
      if (framesSinceSeen > this.config.evictionGraceFrames) {
        toEvict.push(state.request);
      }
    }

    // Clean up stale page states
    for (const [key, state] of this.pageStates) {
      if (this.frame - state.lastSeenFrame > this.config.evictionGraceFrames * 2) {
        this.pageStates.delete(key);
      }
    }

    return { loaded, evicted, totalRequests: feedback.size, lodBias: this.getLodBias() };
  }

  /** Compute priority score for a page request. */
  private computePriority(state: PageState): number {
    const c = this.config;
    const mipDistance = Math.abs(state.request.mip - state.residentMip);
    const confidence = Math.min(state.consecutiveFrames / 5, 1);
    const hitScore = c.weightHitCount * Math.log2(1 + state.hitCount);
    let hysteresis = 1.0;
    if (this.frame - state.lastMipSwitch < c.hysteresisFrames) {
      hysteresis = c.hysteresisFactor;
    }
    const prediction = state.isPredicted ? c.weightPrediction : 0;
    return (c.weightMipDistance * mipDistance + c.weightConfidence * confidence + hitScore + prediction) * hysteresis;
  }

  /** Update adaptive quality based on frame time + oversubscription. */
  private updateAdaptiveQuality(atlasUsage: number) {
    if (this.config.adaptiveQualityEnabled) {
      const avgFrameTime = this.frameTimes.length > 0
        ? this.frameTimes.reduce((a, b) => a + b, 0) / this.frameTimes.length
        : this.config.targetFrameTime;
      const target = this.config.targetFrameTime;
      if (avgFrameTime > target * 1.2) {
        this.frameTimeLodBias = Math.min(this.frameTimeLodBias + this.config.adaptiveLodBiasStep, MAX_MIP);
        this.currentBudget = Math.max(1, this.currentBudget - this.config.adaptiveBudgetStep);
      } else if (avgFrameTime < target * 0.8) {
        this.frameTimeLodBias = Math.max(0, this.frameTimeLodBias - this.config.adaptiveLodBiasStep);
        this.currentBudget = Math.min(this.config.maxBudget, this.currentBudget + this.config.adaptiveBudgetStep);
      }
    }
    if (atlasUsage > this.config.highWaterMark) {
      this.oversubscriptionLodBias = Math.min(this.oversubscriptionLodBias + this.config.adaptiveLodBiasStep, MAX_MIP);
    } else if (atlasUsage < this.config.lowWaterMark) {
      this.oversubscriptionLodBias = Math.max(0, this.oversubscriptionLodBias - this.config.adaptiveLodBiasStep);
    }
  }

  /** Clear an evicted page in its owning texture's CPU and GPU page tables. */
  private evictPage(page: CachedPage): void {
    if (page.tail) {
      this.writeMipTailEntry(page.path, 0);
      return;
    }
    const pageTable = this.pageTables.get(page.path);
    pageTable?.setEvicted(page);
    this.writePageTableEntry(page.path, page, 0);
  }

  private writeMipTailEntry(path: string, value: number): void {
    const entry = this.entries.get(path);
    if (!entry) return;
    const index = packedMipTailIndex(entry.pageTableLayout);
    entry.tailEntry = value;
    entry.pageTable[index] = value;
    const gpuTexture = this.gpuPageTables.get(path);
    if (gpuTexture && this.device) {
      this.device.queue.writeTexture(
        { texture: gpuTexture, origin: { x: 1, y: entry.pageTableLayout.mipOffsets[entry.maxMip] } },
        entry.pageTable.subarray(index, index + 1), {},
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
      this.device.queue.writeTexture(
        {
          texture: gpuTexture,
          origin: { x: req.x, y: entry.pageTableLayout.mipOffsets[req.mip] + req.y },
        },
        entry.pageTable.subarray(idx, idx + 1),
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
    for (const [key, pending] of this.pendingPages) {
      if (pending.page.path !== path) continue;
      this.pendingPages.delete(key);
      this.cache.release(pending.slot);
    }
    this.cache.removeTexture(path);
    const entry = this.entries.get(path);
    if (entry) {
      this.entriesById.delete(entry.textureId);
      entry.pageTableTexture.dispose();
    }
    this.entries.delete(path);
    this.pageTables.delete(path);
    this.gpuPageTables.delete(path);
    for (const [key, state] of this.pageStates) {
      if (state.request.path === path) this.pageStates.delete(key);
    }
  }

  /** Get a virtual texture entry by path. */
  getEntry(path: string): VirtualTextureEntry | undefined {
    return this.entries.get(path);
  }

  /** Resolve the stable ID emitted by the RG32Uint feedback pass. */
  getEntryById(textureId: number): VirtualTextureEntry | undefined {
    return this.entriesById.get(textureId >>> 0);
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
      // Compressed: write directly to GPUTexture via writeTexture
      // [SHLOM] uses gl.texSubImage2D / UpdateSubregion — WebGPU equivalent
      this.device.queue.writeTexture(
        {
          texture: this.gpuAtlasTexture,
          origin: { x: slot.x * SLOT_SIZE, y: slot.y * SLOT_SIZE },
        },
        data,
        {
          bytesPerRow: this.cache.slotBytesPerRow,
          rowsPerImage: SLOT_BLOCKS_Y,
        },
        { width: SLOT_SIZE, height: SLOT_SIZE },
      );
    } else {
      // Startup only. Once attachRenderer() succeeds, both compressed and RGBA
      // pages use the same queue.writeTexture subregion path above.
      this.atlasTexture.needsUpdate = true;
    }
  }

  /** Poll the store (call every frame before processFeedback). */
  poll() {
    this.loader.poll();
  }

  /** Pause residency to inspect fallback behavior without changing the cache. */
  setDebugPaused(paused: boolean): void { this.debugPaused = paused; }

  /** Override pages loaded per frame; pass null to restore adaptive budgeting. */
  setDebugPageBudget(pages: number | null): void {
    if (pages !== null && (!Number.isInteger(pages) || pages < 0))
      throw new RangeError('debug page budget must be a non-negative integer or null');
    this.debugPageBudget = pages;
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
        virtualSize: entry.virtualSize,
        pageGrid: entry.pageGrid,
        maxMip: entry.maxMip,
        residentPages: this.pageTables.get(entry.path)?.residentCount ?? 0,
        pendingPages: [...this.pendingPages.values()].filter(pending => pending.page.path === entry.path).length,
      })),
    };
  }

  /** Get stats for debugging. */
  getStats() {
    return {
      atlasSlotsUsed: this.cache.usedSlots,
      atlasSlotsTotal: this.cache.totalSlots,
      trackedPages: this.pageStates.size,
      pendingPages: this.pendingPages.size,
      lodBias: this.getLodBias(),
      budget: this.currentBudget,
      avgFrameTime: this.frameTimes.length > 0
        ? this.frameTimes.reduce((a, b) => a + b, 0) / this.frameTimes.length
        : 0,
    };
  }
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
    let tail_offset = i32(pageGrid.y * (2.0 - exp2(1.0 - maxMip)));
    let tail_entry = textureLoad(pageTable, vec2i(1, tail_offset), 0).r;
    if ((tail_entry & 1u) != 0u) {
      let delta = desired_level - i32(maxMip);
      var rect_origin = vec2f(0.0);
      var tail_size = 64.0;
      if (delta == 2) { rect_origin = vec2f(72.0, 0.0); tail_size = 32.0; }
      else if (delta == 3) { rect_origin = vec2f(112.0, 0.0); tail_size = 16.0; }
      else if (delta == 4) { rect_origin = vec2f(72.0, 40.0); tail_size = 8.0; }
      else if (delta == 5) { rect_origin = vec2f(88.0, 40.0); tail_size = 4.0; }
      else if (delta == 6) { rect_origin = vec2f(100.0, 40.0); tail_size = 2.0; }
      else if (delta >= 7) { rect_origin = vec2f(110.0, 40.0); tail_size = 1.0; }
      let tail_x = (tail_entry >> 1) & 0xFFu;
      let tail_y = (tail_entry >> 9) & 0xFFu;
      let slot_origin = vec2f(tail_x, tail_y) * (pageSize + pageBorder * 2.0);
      let tail_texel = slot_origin + rect_origin + pageBorder + addressed_uv * tail_size;
      let tail_uv = tail_texel / atlasSize;
      let tail_scale = vec2f(tail_size) / atlasSize;
      return textureSampleGrad(atlas, atlasSampler, tail_uv, dpdx(uv) * tail_scale, dpdy(uv) * tail_scale);
    }
  }

  var mip_level = min(desired_level, i32(maxMip));
  let max_level = i32(maxMip);

  // Walk from desired mip up, looking for resident page
  var is_resident = false;
  var entry = 0u;
  var curr_page_grid = vec2f(0.0);

  for (var m = mip_level; m <= max_level; m = m + 1) {
    let mip_scale = exp2(-f32(m));
    curr_page_grid = max(pageGrid * mip_scale, vec2f(1.0));
    let page_coords = vec2i(floor(addressed_uv * curr_page_grid));
    let mip_offset = i32(pageGrid.y * (2.0 - exp2(1.0 - f32(m))));
    entry = textureLoad(pageTable, vec2i(page_coords.x, page_coords.y + mip_offset), 0).r;
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
  let local_uv = fract(addressed_uv * curr_page_grid);
  let page_origin = vec2f(physX, physY) * (pageSize + pageBorder * 2.0);
  let sample_texel = page_origin + pageBorder + local_uv * pageSize;
  let atlas_uv = sample_texel / atlasSize;

  // Atlas-space gradients preserve anisotropy without allowing the GPU to
  // derive across an unrelated neighboring physical slot.
  let gradient_scale = curr_page_grid * pageSize / atlasSize;
  let atlas_dx = dpdx(uv) * gradient_scale;
  let atlas_dy = dpdy(uv) * gradient_scale;
  return textureSampleGrad(atlas, atlasSampler, atlas_uv, atlas_dx, atlas_dy);
}
`;

/**
 * The feedback shader. Renders to a low-res target, writing page IDs.
 *
 * Source: [SHLOM] feedback.frag, validated in prototype.
 */
export const VT_FEEDBACK_WGSL = /* wgsl */ `
fn vtFeedback(
  uv: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  maxMip: f32,
  bufferScreenRatio: f32,
  textureId: u32
) -> vec2u {
  let effective_size = virtualSize * bufferScreenRatio;
  let dx = dpdx(uv * effective_size);
  let dy = dpdy(uv * effective_size);
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_level = u32(clamp(0.5 * log2(max(texel_footprint, 1e-8)), 0.0, maxMip));

  let mip_scale = exp2(-f32(mip_level));
  let curr_page_grid = max(pageGrid * mip_scale, vec2f(1.0));
  let page_coords = floor(uv * curr_page_grid);

  // RG32Uint: word 0 carries valid + 6-bit mip + 11-bit X/Y;
  // word 1 carries the full virtual-texture identity.
  let packed = 0x80000000u |
               (mip_level & 0x3Fu) |
               ((u32(page_coords.x) & 0x7FFu) << 6) |
               ((u32(page_coords.y) & 0x7FFu) << 17);
  return vec2u(packed, textureId);
}
`;
