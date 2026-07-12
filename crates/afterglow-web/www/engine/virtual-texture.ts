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
const PINNED_MIPS = new Set([MAX_MIP, MAX_MIP - 1]); // coarsest 2 levels always resident

// ============================================================================
// Types
// ============================================================================

/** A page request: which virtual page at which mip level is needed. */
export interface PageRequest {
  mip: number;
  x: number;
  y: number;
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

/** A virtual texture descriptor — created by loadTexture(). */
export interface VirtualTextureEntry {
  /** Path in the .big file (or loader key). */
  path: string;
  /** Virtual texture size in texels (e.g., 4096 for a 4096×4096 texture). */
  virtualSize: number;
  /** Number of pages per side at mip 0 (virtualSize / PAGE_SIZE). */
  pageGrid: number;
  /** Max mip level for this texture. */
  maxMip: number;
  /** Page table data (Uint32Array, one u32 per page at mip 0). */
  pageTable: Uint32Array;
  /** Page table texture (GPU-readable, NearestFilter, mipmapped). */
  pageTableTexture: THREE.DataTexture;
}

/** Per-page tracking state (for the smart strategy). */
interface PageState {
  request: PageRequest;
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
  /** Atlas pixel data (RGBA8). */
  atlas: Uint8Array;
  /** Which virtual page is in each slot. */
  private slots: (PageRequest | null)[] = [];
  /** Free slot indices. */
  private freeSlots: number[] = [];
  /** LRU list: front = MRU, back = LRU. */
  private lru: PageRequest[] = [];
  /** Map: page key → index in LRU. */
  private lruMap = new Map<string, number>();
  /** Pinned mip levels (never evicted). */
  private pinnedMips: Set<number>;

  constructor(pinnedMips: Set<number>) {
    this.atlas = new Uint8Array(ATLAS_WIDTH * ATLAS_HEIGHT * 4);
    this.pinnedMips = pinnedMips;

    for (let y = 0; y < ATLAS_PAGES_Y; y++) {
      for (let x = 0; x < ATLAS_PAGES_X; x++) {
        this.slots.push(null);
        this.freeSlots.push(y * ATLAS_PAGES_X + x);
      }
    }
  }

  private lruKey(req: PageRequest): string {
    return `${req.mip}:${req.x}:${req.y}`;
  }

  /** Mark a page as recently used (move to front of LRU). */
  touch(req: PageRequest) {
    if (this.pinnedMips.has(req.mip)) return;
    const key = this.lruKey(req);
    const idx = this.lruMap.get(key);
    if (idx !== undefined) {
      this.lru.splice(idx, 1);
      this.lru.unshift(req);
      this.rebuildLruMap();
    }
  }

  /** Acquire a free slot, evicting LRU if necessary. */
  acquire(req: PageRequest): { slot: PageSlot; evicted: PageRequest | null } {
    // Try free slot
    if (this.freeSlots.length > 0) {
      const idx = this.freeSlots.pop()!;
      const slot = { x: idx % ATLAS_PAGES_X, y: Math.floor(idx / ATLAS_PAGES_X) };
      return { slot, evicted: null };
    }

    // Evict LRU (from back, skip pinned)
    let evictIdx = -1;
    for (let i = this.lru.length - 1; i >= 0; i--) {
      if (!this.pinnedMips.has(this.lru[i].mip)) {
        evictIdx = i;
        break;
      }
    }

    if (evictIdx === -1) {
      throw new Error('No evictable slots available (all pages pinned)');
    }

    const evictedReq = this.lru[evictIdx];
    const slotIdx = this.slots.findIndex(s =>
      s !== null && s.mip === evictedReq.mip && s.x === evictedReq.x && s.y === evictedReq.y
    );
    if (slotIdx === -1) throw new Error('Slot not found for evicted page');

    this.lru.splice(evictIdx, 1);
    this.lruMap.delete(this.lruKey(evictedReq));
    this.slots[slotIdx] = null;

    const slot = { x: slotIdx % ATLAS_PAGES_X, y: Math.floor(slotIdx / ATLAS_PAGES_X) };
    return { slot, evicted: evictedReq };
  }

  /** Write page data into a slot and mark as resident. */
  commit(req: PageRequest, slot: PageSlot, data: Uint8Array) {
    const dstX = slot.x * SLOT_SIZE;
    const dstY = slot.y * SLOT_SIZE;

    // Copy page data into atlas at slot position
    for (let y = 0; y < SLOT_SIZE; y++) {
      const srcRow = y * SLOT_SIZE * 4;
      const dstRow = ((dstY + y) * ATLAS_WIDTH + dstX) * 4;
      for (let x = 0; x < SLOT_SIZE * 4; x++) {
        this.atlas[dstRow + x] = data[srcRow + x];
      }
    }

    const slotIdx = slot.y * ATLAS_PAGES_X + slot.x;
    this.slots[slotIdx] = req;

    const freeIdx = this.freeSlots.indexOf(slotIdx);
    if (freeIdx >= 0) this.freeSlots.splice(freeIdx, 1);

    if (!this.pinnedMips.has(req.mip)) {
      this.lru.unshift(req);
      this.rebuildLruMap();
    }
  }

  private rebuildLruMap() {
    this.lruMap.clear();
    for (let i = 0; i < this.lru.length; i++) {
      this.lruMap.set(this.lruKey(this.lru[i]), i);
    }
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
 *   const handle = vt.loadTexture('terrain', { virtualSize: 4096, pageData: ... });
 *   // handle.asset = shared atlas texture
 *   // handle.generation increments when pages are loaded/evicted
 */
export class VirtualTextureStore {
  /** The shared physical atlas texture (all VT textures sample from this). */
  readonly atlasTexture: THREE.DataTexture;

  /** Page tables per virtual texture path. */
  private entries = new Map<string, VirtualTextureEntry>();

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
  ) {
    this.loader = loader;
    this.pageDataProvider = pageDataProvider;
    this.cache = new PageCache(PINNED_MIPS);

    // Create the shared atlas texture
    this.atlasTexture = new THREE.DataTexture(
      this.cache.atlas,
      ATLAS_WIDTH,
      ATLAS_HEIGHT,
      THREE.RGBAFormat,
    );
    this.atlasTexture.minFilter = THREE.LinearFilter;
    this.atlasTexture.magFilter = THREE.LinearFilter;
    this.atlasTexture.generateMipmaps = false;
    this.atlasTexture.needsUpdate = true;
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
   * this virtual texture.
   */
  loadTexture(
    path: string,
    options: { virtualSize: number; pageData?: Map<string, Uint8Array> },
  ): AssetHandle<THREE.Texture> {
    const virtualSize = options.virtualSize;
    const pageGrid = virtualSize / PAGE_SIZE;
    const maxMip = Math.floor(Math.log2(pageGrid));

    // Create page table
    const pageTable = new PageTable(maxMip);
    this.pageTables.set(path, pageTable);

    // Create page table texture (for GPU shader)
    const pageTableData = new Uint32Array(pageGrid * pageGrid);
    const pageTableTexture = new THREE.DataTexture(
      new Uint8Array(pageTableData.buffer),
      pageGrid,
      pageGrid,
      THREE.RGBAIntegerFormat,
      THREE.UnsignedIntType,
    );
    pageTableTexture.minFilter = THREE.NearestFilter;
    pageTableTexture.magFilter = THREE.NearestFilter;
    pageTableTexture.generateMipmaps = false;

    const entry: VirtualTextureEntry = {
      path,
      virtualSize,
      pageGrid,
      maxMip,
      pageTable: pageTableData,
      pageTableTexture,
    };
    this.entries.set(path, entry);

    // Pre-load pinned pages (coarsest mip levels)
    this.loadPinnedPages(path, entry);

    // Create handle pointing to the shared atlas
    const handle = new AssetHandle<THREE.Texture>(path, this.atlasTexture);
    handle.generation++;
    handle.state = 'loading';
    return handle;
  }

  /** Pre-load pinned (coarsest) pages. */
  private loadPinnedPages(path: string, entry: VirtualTextureEntry) {
    const pageTable = this.pageTables.get(path)!;
    for (const mip of PINNED_MIPS) {
      if (mip > entry.maxMip) continue;
      const pagesAtMip = entry.pageGrid >> mip;
      for (let y = 0; y < pagesAtMip; y++) {
        for (let x = 0; x < pagesAtMip; x++) {
          const req: PageRequest = { mip, x, y };
          this.loadPageSync(path, req, pageTable);
        }
      }
    }
  }

  /** Load a single page synchronously (for pinned pages). */
  private loadPageSync(path: string, req: PageRequest, pageTable: PageTable) {
    if (!this.pageDataProvider) return;
    if (pageTable.isResident(req)) return;

    const { slot, evicted } = this.cache.acquire(req);
    if (evicted) pageTable.setEvicted(evicted);

    // Synchronous load (pinned pages should be available immediately)
    // In practice, this would use pre-loaded data
    const data = new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4); // placeholder
    this.cache.commit(req, slot, data);
    pageTable.setResident(req, slot);
  }

  /**
   * Process feedback from the GPU. This is the main per-frame VT update.
   *
   * @param feedback Map of page requests from the feedback pass
   * @param cameraPos Current camera position (for prediction)
   * @param cameraZoom Current camera zoom (for prediction)
   */
  processFeedback(
    feedback: Map<string, PageRequest>,
    cameraPos?: [number, number],
    cameraZoom?: number,
  ) {
    this.frame++;

    if (cameraPos && cameraZoom) {
      this.recordCamera(cameraPos, cameraZoom);
    }

    // Touch all resident pages seen in feedback
    for (const req of feedback.values()) {
      this.cache.touch(req);
    }

    // Update adaptive quality
    const atlasUsage = this.cache.usedSlots / this.cache.totalSlots;
    this.updateAdaptiveQuality(atlasUsage);

    // Update page states from feedback
    const activeKeys = new Set<string>();
    for (const [key, req] of feedback) {
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

    // Decay consecutive frames for pages not in feedback
    for (const [key, state] of this.pageStates) {
      if (!activeKeys.has(key)) {
        state.consecutiveFrames = 0;
      }
    }

    // Compute priority for all non-resident pages
    const toLoad: Array<{ req: PageRequest; priority: number; path: string }> = [];

    for (const [key, state] of this.pageStates) {
      // Find which page table this belongs to (simplified: check all)
      for (const [path, pt] of this.pageTables) {
        if (pt.isResident(state.request)) {
          state.residentMip = state.request.mip;
          continue;
        }
        state.residentMip = -1;
        const priority = this.computePriority(state);
        toLoad.push({ req: state.request, priority, path });
      }
    }

    // Sort by priority (highest first)
    toLoad.sort((a, b) => b.priority - a.priority);

    // Load pages up to budget
    let loaded = 0;
    let evicted = 0;
    const budget = this.currentBudget;

    for (const { req, path } of toLoad) {
      if (loaded >= budget) break;
      const pt = this.pageTables.get(path);
      if (!pt) continue;

      try {
        const { slot, evicted: evictedReq } = this.cache.acquire(req);
        if (evictedReq) {
          pt.setEvicted(evictedReq);
          evicted++;
        }

        // Load page data (async — will be available next frame)
        if (this.pageDataProvider) {
          this.pageDataProvider(path, req).then(data => {
            this.cache.commit(req, slot, data);
            pt.setResident(req, slot);
            this.atlasTexture.needsUpdate = true;
            // Update page table texture
            this.updatePageTableTexture(path, req, slot);
          });
        }
        loaded++;
      } catch {
        // No evictable slots
      }
    }

    // Evict pages not seen for graceFrames
    const toEvict: PageRequest[] = [];
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

  /** Update the page table texture on the GPU. */
  private updatePageTableTexture(path: string, req: PageRequest, slot: PageSlot) {
    const entry = this.entries.get(path);
    if (!entry) return;
    const pagesAtMip = entry.pageGrid >> req.mip;
    const idx = req.y * pagesAtMip + req.x;
    entry.pageTable[idx] = packEntry(true, slot.x, slot.y);
    entry.pageTableTexture.needsUpdate = true;
  }

  /** Get a virtual texture entry by path. */
  getEntry(path: string): VirtualTextureEntry | undefined {
    return this.entries.get(path);
  }

  /** Poll the store (call every frame before processFeedback). */
  poll() {
    this.loader.poll();
  }

  /** Get stats for debugging. */
  getStats() {
    return {
      atlasSlotsUsed: this.cache.usedSlots,
      atlasSlotsTotal: this.cache.totalSlots,
      trackedPages: this.pageStates.size,
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
  maxMip: f32
) -> vec4f {
  // Compute desired mip level from screen-space derivatives
  let dx = dpdx(uv * virtualSize);
  let dy = dpdy(uv * virtualSize);
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_float = clamp(0.5 * log2(max(texel_footprint, 1e-8)), 0.0, maxMip);

  var mip_level = i32(mip_float);
  let max_level = i32(maxMip);

  // Walk from desired mip up, looking for resident page
  var is_resident = false;
  var entry = 0u;
  var curr_page_grid = vec2f(0.0);

  for (var m = mip_level; m <= max_level; m = m + 1) {
    let mip_scale = exp2(-f32(m));
    curr_page_grid = max(pageGrid * mip_scale, vec2f(1.0));
    let page_coords = vec2i(floor(uv * curr_page_grid));
    entry = textureLoad(pageTable, page_coords, m).r;
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
  let local_uv = fract(uv * curr_page_grid);
  let page_origin = vec2f(physX, physY) * (pageSize + pageBorder * 2.0);
  let sample_texel = page_origin + pageBorder + local_uv * pageSize;
  let atlas_uv = sample_texel / atlasSize;

  return textureSample(atlas, atlasSampler, atlas_uv);
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
  bufferScreenRatio: f32
) -> u32 {
  let effective_size = virtualSize * bufferScreenRatio;
  let dx = dpdx(uv * effective_size);
  let dy = dpdy(uv * effective_size);
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_level = u32(clamp(0.5 * log2(max(texel_footprint, 1e-8)), 0.0, maxMip));

  let mip_scale = exp2(-f32(mip_level));
  let curr_page_grid = max(pageGrid * mip_scale, vec2f(1.0));
  let page_coords = floor(uv * curr_page_grid);

  // Pack: bit 31 = valid, bits 0-4 = mip, bits 5-12 = pageX, bits 13-20 = pageY
  return 0x80000000u |
         (mip_level & 0x1Fu) |
         ((u32(page_coords.x) & 0xFFu) << 5) |
         ((u32(page_coords.y) & 0xFFu) << 13);
}
`;
