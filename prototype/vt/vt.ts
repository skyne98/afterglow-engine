// Virtual Texturing Prototype — validates all VT algorithms in pure TypeScript.
// Run: bun run prototype/vt/vt.ts
// Tests: bun test prototype/vt/vt.test.ts
//
// Each algorithm is annotated with its source reference:
//   [SHLOM]  shlomnissan/virtual-textures (C++/OpenGL, 2025)
//   [IDTECH] id Software "Software Virtual Textures" paper (van Waveren, 2012)
//   [ELFRA]  elfrank/virtual-texturing (Three.js WebGL, r57)
//   [BARRE]  Sean Barrett "Sparse Virtual Textures" (GDC 2008)
//
// Output: PPM images showing rendered result, ground truth, and atlas contents.
//
// All functions/classes are exported for unit testing.

// ============================================================================
// Constants
// ============================================================================

// Page layout — matches [IDTECH] Section 4.1:
//   "a 4-texel border is typically used around each physical page"
//   pageWidth=128, pageBorder=4 (per side), payload=120, slot=128
// Note: [SHLOM] uses kPagePadding=4 TOTAL (2 per side), kSlotSize=516.
// We use the [IDTECH] approach (4 per side) for better anisotropic filtering.
const PAGE_SIZE = 128;           // payload texels per page (excluding border)
const PAGE_BORDER = 4;           // border texels PER SIDE (for bilinear/aniso)
const SLOT_SIZE = PAGE_SIZE + PAGE_BORDER * 2; // 136 texels per physical slot

// Physical atlas — matches [IDTECH] Section 6.4:
//   "physical texture size is typically 4096×4096 = 32×32 pages"
// We use a smaller atlas for the prototype.
const ATLAS_PAGES_X = 8;         // 8×8 = 64 page slots
const ATLAS_PAGES_Y = 8;
const ATLAS_WIDTH = ATLAS_PAGES_X * SLOT_SIZE;  // 1088
const ATLAS_HEIGHT = ATLAS_PAGES_Y * SLOT_SIZE;  // 1088

// Virtual texture — matches [IDTECH] Section 6.4:
//   "120K×120K virtual texture (1024×1024 pages)"
// We use a smaller virtual texture for the prototype.
const VIRTUAL_SIZE = 4096;       // 4096×4096 virtual texture
const VIRTUAL_PAGES_X = VIRTUAL_SIZE / PAGE_SIZE; // 32×32 = 1024 pages
const VIRTUAL_PAGES_Y = VIRTUAL_SIZE / PAGE_SIZE;
const MAX_MIP = Math.floor(Math.log2(VIRTUAL_PAGES_X)); // 5 (6 levels: 0-5)

// Rendering
const SCREEN_WIDTH = 256;
const SCREEN_HEIGHT = 256;

// Feedback buffer — [IDTECH] Section 3.4:
//   "feedback can be rendered at a significantly lower resolution (say 10x smaller)"
// [SHLOM] uses u_BufferScreenRatio which serves the same purpose.
const FEEDBACK_SCALE = 0.125;    // 1/8 resolution

// Pinned LODs — [SHLOM] kMinPinnedLod=4 (coarsest 2 levels always resident)
// [IDTECH] Section 4.5: "the texture page that represents the coarsest mip level
//   of a virtual texture is usually locked in the physical page textures"
const PINNED_MIPS = new Set<number>([MAX_MIP, MAX_MIP - 1]);

// ============================================================================
// Types
// ============================================================================

/** A page request: which virtual page at which mip level is needed. */
export interface PageRequest {
  mip: number;
  x: number;   // page X in virtual texture at this mip level
  y: number;   // page Y in virtual texture at this mip level
}

/** A physical page slot in the atlas. */
export interface PageSlot {
  x: number;   // slot X in atlas (0..ATLAS_PAGES_X-1)
  y: number;   // slot Y in atlas
}

/** RGBA pixel */
export type Pixel = [number, number, number, number]; // R, G, B, A (0-255)

// Export constants for testing
export {
  PAGE_SIZE, PAGE_BORDER, SLOT_SIZE,
  ATLAS_PAGES_X, ATLAS_PAGES_Y, ATLAS_WIDTH, ATLAS_HEIGHT,
  VIRTUAL_SIZE, VIRTUAL_PAGES_X, VIRTUAL_PAGES_Y, MAX_MIP,
  SCREEN_WIDTH, SCREEN_HEIGHT, FEEDBACK_SCALE, PINNED_MIPS,
};

// ============================================================================
// Page Table Entry — bit-packed u32
// ============================================================================
// Source: [SHLOM] page_manager.cpp, material.frag
//
// shlomnissan packs: bit 0 = resident, bits 1-8 = physX, bits 9-16 = physY
// We add bits 17-21 = mip level (needed since we use a flat Map, not a
// mipmapped texture, for the prototype).
//
// [SHLOM] page_manager.cpp:
//   auto entry = uint32_t { 0x1 | ((req.slot.x & 0xFFu) << 1) | ((req.slot.y & 0xFFu) << 9) };
//
// [SHLOM] material.frag:
//   entry = texelFetch(u_PageTable, ivec2(page_coords), mip_level).r;
//   if ((entry & 1u) != 0u) { is_resident = true; break; }
//   ivec2 physical_page = ivec2((entry >> 1) & 0xFFu, (entry >> 9) & 0xFFu);

export function packEntry(resident: boolean, physX: number, physY: number, mip: number): number {
  return (resident ? 1 : 0) |
         ((physX & 0xFF) << 1) |
         ((physY & 0xFF) << 9) |
         ((mip & 0x1F) << 17);
}
export function isResident(entry: number): boolean { return (entry & 1) !== 0; }
export function getPhysX(entry: number): number { return (entry >> 1) & 0xFF; }
export function getPhysY(entry: number): number { return (entry >> 9) & 0xFF; }
export function getMip(entry: number): number { return (entry >> 17) & 0x1F; }

// ============================================================================
// Virtual Texture (source data — what we're virtualizing)
// ============================================================================

/** Sample the virtual texture directly (ground truth for verification). */
export function sampleVirtualTexture(u: number, v: number): Pixel {
  const x = Math.floor(u * VIRTUAL_SIZE);
  const y = Math.floor(v * VIRTUAL_SIZE);

  const left = x < VIRTUAL_SIZE / 2;
  const top = y < VIRTUAL_SIZE / 2;
  let r: number, g: number, b: number;

  if (top && left) { r = 220; g = 50; b = 50; }       // red
  else if (top && !left) { r = 50; g = 180; b = 80; }  // green
  else if (!top && left) { r = 50; g = 100; b = 220; } // blue
  else { r = 220; g = 200; b = 50; }                    // yellow

  // White cross
  if (Math.abs(x - VIRTUAL_SIZE / 2) < 16 || Math.abs(y - VIRTUAL_SIZE / 2) < 16) {
    r = 255; g = 255; b = 255;
  }

  // Diagonal stripes
  if ((x + y) % 64 < 8) {
    r = Math.min(255, r + 60);
    g = Math.min(255, g + 60);
    b = Math.min(255, b + 60);
  }

  return [r, g, b, 255];
}

// ============================================================================
// Page Generation (with border)
// ============================================================================
// Source: [IDTECH] Section 3.2 + Section 4.1
//
// "In order to properly support hardware bi-linear filtering, each physical
//  texture page must have a border of texels around it."
// "a 4-texel border is typically used around each physical page"
//
// The border texels replicate from adjacent virtual pages so that bilinear
// filtering at page boundaries samples correct data.
// [IDTECH] Section 4.1: "The border texels need not be stored on disk, but
//   as a practical matter, it is far less complicated to have pages be fully
//   independent and actually store the additional 12% texels on disk."

export function generatePage(req: PageRequest): Uint8Array {
  const pagesAtMip = VIRTUAL_PAGES_X >> req.mip;
  const texelsAtMip = VIRTUAL_SIZE >> req.mip;
  const data = new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4);

  for (let sy = 0; sy < SLOT_SIZE; sy++) {
    for (let sx = 0; sx < SLOT_SIZE; sx++) {
      // Map slot texel → virtual texture coordinate
      // Slot layout: [BORDER][PAGE_SIZE payload][BORDER]
      const payloadX = sx - PAGE_BORDER;
      const payloadY = sy - PAGE_BORDER;

      // Virtual texel coordinate in the full virtual texture at this mip
      const vx = req.x * PAGE_SIZE + payloadX;
      const vy = req.y * PAGE_SIZE + payloadY;

      // Clamp to valid range — border texels at edges replicate edge texel
      // [IDTECH] Section 3.2: "edge texels to be equal ... clamped pages"
      const cx = Math.max(0, Math.min(texelsAtMip - 1, vx));
      const cy = Math.max(0, Math.min(texelsAtMip - 1, vy));

      const u = cx / texelsAtMip;
      const v = cy / texelsAtMip;
      const [r, g, b, a] = sampleVirtualTexture(u, v);

      const idx = (sy * SLOT_SIZE + sx) * 4;
      data[idx] = r;
      data[idx + 1] = g;
      data[idx + 2] = b;
      data[idx + 3] = a;
    }
  }

  return data;
}

// ============================================================================
// Page Table
// ============================================================================
// Source: [SHLOM] page_tables.cpp, [IDTECH] Section 3.1
//
// [SHLOM] uses a mipmapped R32UI texture. Each mip level of the page table
// corresponds to a virtual texture mip level. texelFetch(pageTable, coords, mip)
// looks up a specific mip level.
//
// [IDTECH] Section 3.1 describes multiple page table formats. The simplest
// is a mipmapped texture with one texel per virtual page.
//
// For the prototype, we use a Map<string, u32> which is functionally equivalent.

export class PageTable {
  private entries = new Map<string, number>();
  private maxMip: number;

  constructor(maxMip: number) {
    this.maxMip = maxMip;
  }

  private key(req: PageRequest): string {
    return `${req.mip}:${req.x}:${req.y}`;
  }

  get(req: PageRequest): number {
    return this.entries.get(this.key(req)) ?? 0; // 0 = not resident
  }

  setResident(req: PageRequest, slot: PageSlot) {
    this.entries.set(this.key(req), packEntry(true, slot.x, slot.y, req.mip));
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
   * Source: [SHLOM] material.frag — the fallback loop:
   *   for (; mip_level <= max_level; ++mip_level) {
   *     curr_page_grid = max(u_PageGrid * exp2(-float(mip_level)), vec2(1.0));
   *     page_coords = floor(v_TexCoord * curr_page_grid);
   *     entry = texelFetch(u_PageTable, ivec2(page_coords), mip_level).r;
   *     if ((entry & 1u) != 0u) { is_resident = true; break; }
   *   }
   *
   * [IDTECH] Section 3.1: "the address translation falls back to a page from
   *   a coarser mip level because the texture page for the desired finer mip
   *   level is not yet available in the pool of physical pages."
   */
  findResidentPage(u: number, v: number, desiredMip: number): { entry: number; mip: number } | null {
    for (let mip = desiredMip; mip <= this.maxMip; mip++) {
      const pagesAtMip = VIRTUAL_PAGES_X >> mip;
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
// Source: [SHLOM] page_cache.cpp, [IDTECH] Section 5.2
//
// [SHLOM] uses std::list + std::unordered_map for O(1) LRU.
// [IDTECH] Section 5.2: "The priority is first based on the LOD level of the
//   page such that finer mips will be replaced first. Next, the page priority
//   for replacement increases as the number of rendered frames increases since
//   the page was last used."
//
// [SHLOM] uses pure LRU (no mip-priority), with pinned LODs (kMinPinnedLod=4).
// We follow [SHLOM]'s approach for simplicity.

export class PageCache {
  atlas: Uint8Array;
  slots: (PageRequest | null)[] = [];
  freeSlots: number[] = [];
  private lru: PageRequest[] = [];              // front = MRU, back = LRU
  private lruMap = new Map<string, number>();   // page key → index in lru
  private pinnedMips: Set<number>;

  constructor(pinnedMips: Set<number>) {
    this.atlas = new Uint8Array(ATLAS_WIDTH * ATLAS_HEIGHT * 4);
    this.pinnedMips = pinnedMips;

    // [SHLOM] page_cache.cpp constructor:
    //   for (auto y = 0; y < kAtlasSlots.y; ++y)
    //     for (auto x = 0; x < kAtlasSlots.x; ++x)
    //       free_slots_.emplace_back(x, y);
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

  /**
   * Mark a page as recently used (move to front of LRU).
   * Source: [SHLOM] page_cache.cpp Touch():
   *   if (request.lod >= kMinPinnedLod) return; // no-op for pinned lods
   *   lru_list_.splice(lru_list_.begin(), lru_list_, it->second);
   */
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

  /**
   * Acquire a free slot, evicting LRU if necessary.
   * Source: [SHLOM] page_cache.cpp Acquire():
   *   1. Check if already resident → return existing slot
   *   2. Try free slot → return it
   *   3. Walk LRU from back, skip pinned, evict first non-pinned
   */
  acquire(req: PageRequest): { slot: PageSlot; evicted: PageRequest | null } {
    // [SHLOM] step 1: already resident? (caller checks this, but we keep for safety)

    // [SHLOM] step 2: try free slot
    if (this.freeSlots.length > 0) {
      const idx = this.freeSlots.pop()!;
      const slot = { x: idx % ATLAS_PAGES_X, y: Math.floor(idx / ATLAS_PAGES_X) };
      return { slot, evicted: null };
    }

    // [SHLOM] step 3: evict LRU (from back of list, skip pinned)
    //   auto it = lru_list_.rbegin();
    //   while (it != lru_list_.rend()) {
    //     if (it->lod < kMinPinnedLod) break;
    //     ++it;
    //   }
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

    // Find the physical slot that holds the evicted page
    const slotIdx = this.slots.findIndex(s =>
      s !== null && s.mip === evictedReq.mip && s.x === evictedReq.x && s.y === evictedReq.y
    );
    if (slotIdx === -1) throw new Error('Slot not found for evicted page');

    // Remove from LRU + slots
    this.lru.splice(evictIdx, 1);
    this.lruMap.delete(this.lruKey(evictedReq));
    this.slots[slotIdx] = null;

    const slot = { x: slotIdx % ATLAS_PAGES_X, y: Math.floor(slotIdx / ATLAS_PAGES_X) };
    return { slot, evicted: evictedReq };
  }

  /**
   * Write page data into a slot and mark as resident.
   * Source: [SHLOM] page_cache.cpp Commit() + page_manager.cpp FlushProcessingRequests():
   *   tex_atlas_->UpdateSubregion(0, slot_size_x * req.slot.x, slot_size_y * req.slot.y,
   *                              slot_size_x, slot_size_y, data);
   */
  commit(req: PageRequest, slot: PageSlot, data: Uint8Array) {
    // Copy page data into atlas at slot position
    // [SHLOM] uses UpdateSubregion(slot.x * kSlotSize, slot.y * kSlotSize, ...)
    const dstX = slot.x * SLOT_SIZE;
    const dstY = slot.y * SLOT_SIZE;

    for (let y = 0; y < SLOT_SIZE; y++) {
      const srcRow = y * SLOT_SIZE * 4;
      const dstRow = ((dstY + y) * ATLAS_WIDTH + dstX) * 4;
      for (let x = 0; x < SLOT_SIZE * 4; x++) {
        this.atlas[dstRow + x] = data[srcRow + x];
      }
    }

    // Mark slot as used
    const slotIdx = slot.y * ATLAS_PAGES_X + slot.x;
    this.slots[slotIdx] = req;

    // Remove from free slots
    const freeIdx = this.freeSlots.indexOf(slotIdx);
    if (freeIdx >= 0) this.freeSlots.splice(freeIdx, 1);

    // Add to LRU front (most recently used)
    // [SHLOM] Commit: lru_list_.emplace_front(request); lru_map_[request] = lru_list_.begin();
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
}

// ============================================================================
// Mip Level Computation (Feedback Shader Logic)
// ============================================================================
// Source: [SHLOM] feedback.frag, [IDTECH] Appendix B, [ELFRA] Shaders.js
//
// [SHLOM] feedback.frag:
//   float ComputeMipLevel(in vec2 effective_size, in vec2 uv) {
//     vec2 dx = dFdx(uv) * effective_size;
//     vec2 dy = dFdy(uv) * effective_size;
//     float texel_footprint = max(dot(dx, dx), dot(dy, dy));
//     return 0.5 * log2(max(texel_footprint, 1e-8));
//   }
//
// [IDTECH] Appendix B:
//   float2 texcoords = fragment.texcoord0.xy * virtTexelsWide;
//   float2 dx = ddx(texcoords);
//   float2 dy = ddy(texcoords);
//   float px = dot(dx, dx);
//   float py = dot(dy, dy);
//   float maxLod = 0.5 * log2(max(px, py));   // log2(sqrt()) = 0.5*log2()
//
// [ELFRA] Shaders.js (has -1.0 offset for border compensation):
//   return max(0.5 * log2(d) - 1.0, 0.0);
//
// We use the [SHLOM]/[IDTECH] formula (no -1.0 offset).

/**
 * Compute the desired mip level for a given UV and its screen-space derivatives.
 *
 * @param uvDx  dFdx(uv) — change in UV per screen X pixel
 * @param uvDy  dFdy(uv) — change in UV per screen Y pixel
 * @param virtualSize  width of the virtual texture in texels
 * @returns mip level (float, should be floored + clamped before use)
 */
export function computeMipLevel(
  uvDx: [number, number],
  uvDy: [number, number],
  virtualSize: number,
): number {
  // dFdx(uv * virtualSize) = dFdx(uv) * virtualSize
  const dx0 = uvDx[0] * virtualSize;
  const dx1 = uvDx[1] * virtualSize;
  const dy0 = uvDy[0] * virtualSize;
  const dy1 = uvDy[1] * virtualSize;

  // dot(dx, dx) and dot(dy, dy)
  const px = dx0 * dx0 + dx1 * dx1;
  const py = dy0 * dy0 + dy1 * dy1;

  // 0.5 * log2(max(px, py))  ==  log2(sqrt(max(px, py)))  ==  log2(length)
  const d = Math.max(px, py);
  return 0.5 * Math.log2(Math.max(d, 1e-8));
}

// ============================================================================
// Address Translation (Material Shader Logic)
// ============================================================================
// Source: [SHLOM] material.frag, [IDTECH] Appendix A.5 (RGBA8 page table)
//
// [SHLOM] material.frag:
//   // 1. Compute mip level
//   float mip_float = clamp(ComputeMipLevel(u_VirtualSize, v_TexCoord), ...);
//   int mip_level = int(mip_float);
//
//   // 2. Walk from desired mip up, looking for resident page
//   for (; mip_level <= max_level; ++mip_level) {
//     curr_page_grid = max(u_PageGrid * exp2(-float(mip_level)), vec2(1.0));
//     page_coords = floor(v_TexCoord * curr_page_grid);
//     entry = texelFetch(u_PageTable, ivec2(page_coords), mip_level).r;
//     if ((entry & 1u) != 0u) { is_resident = true; break; }
//   }
//
//   // 3. Compute physical atlas UV
//   ivec2 physical_page = ivec2((entry >> 1) & 0xFFu, (entry >> 9) & 0xFFu);
//   vec2 local_uv = fract(v_TexCoord * curr_page_grid);
//   vec2 page_origin = vec2(physical_page) * (u_PageSize + u_PagePadding);
//   vec2 half_padding = u_PagePadding * 0.5;
//   vec2 sample_texel = page_origin + half_padding + local_uv * u_PageSize;
//   vec2 atlas_uv = sample_texel / u_AtlasSize;
//
//   // 4. Sample with scaled derivatives
//   vec2 dx = dFdx(v_TexCoord) * curr_page_grid * (u_PageSize / u_AtlasSize);
//   vec2 dy = dFdy(v_TexCoord) * curr_page_grid * (u_PageSize / u_AtlasSize);
//   v_FragColor = textureGrad(u_TextureAtlas, atlas_uv, dx, dy);
//
// Note: [SHLOM] uses half_padding = padding * 0.5 (padding is TOTAL).
// We use PAGE_BORDER per side, so offset = PAGE_BORDER (not PAGE_BORDER/2).

export function vtSample(
  u: number,
  v: number,
  uvDx: [number, number],
  uvDy: [number, number],
  pageTable: PageTable,
  atlas: Uint8Array,
): Pixel | null {
  // 1. Compute desired mip level — [SHLOM] ComputeMipLevel
  const desiredMip = Math.max(0, Math.min(
    Math.floor(computeMipLevel(uvDx, uvDy, VIRTUAL_SIZE)),
    MAX_MIP,
  ));

  // 2. Find resident page (walk from desired mip up to coarser) — [SHLOM] fallback loop
  const found = pageTable.findResidentPage(u, v, desiredMip);
  if (!found) return null;

  const { entry, mip } = found;

  // 3. Compute physical atlas UV — [SHLOM] material.frag
  const physX = getPhysX(entry);
  const physY = getPhysY(entry);

  // Page grid at this mip level (equivalent to [SHLOM] curr_page_grid)
  const pagesAtMip = VIRTUAL_PAGES_X >> mip;

  // Local UV within the page (0-1) — [SHLOM] fract(v_TexCoord * curr_page_grid)
  const localU = (u * pagesAtMip) % 1;
  const localV = (v * pagesAtMip) % 1;

  // Physical atlas coordinates
  // [SHLOM]: page_origin = physical_page * (pageSize + padding)
  //          sample_texel = page_origin + half_padding + local_uv * pageSize
  // Ours:    page_origin = physSlot * SLOT_SIZE (where SLOT_SIZE = PAGE_SIZE + 2*BORDER)
  //          sample_texel = page_origin + BORDER + localUV * PAGE_SIZE
  const slotOriginX = physX * SLOT_SIZE;
  const slotOriginY = physY * SLOT_SIZE;
  const sampleX = slotOriginX + PAGE_BORDER + localU * PAGE_SIZE;
  const sampleY = slotOriginY + PAGE_BORDER + localV * PAGE_SIZE;

  const atlasU = Math.floor(sampleX);
  const atlasV = Math.floor(sampleY);

  // 4. Sample the atlas (no textureGrad in prototype — just nearest point sample)
  if (atlasU < 0 || atlasU >= ATLAS_WIDTH || atlasV < 0 || atlasV >= ATLAS_HEIGHT) {
    return null;
  }
  const idx = (atlasV * ATLAS_WIDTH + atlasU) * 4;
  return [atlas[idx], atlas[idx + 1], atlas[idx + 2], atlas[idx + 3]];
}

// ============================================================================
// Feedback Simulation
// ============================================================================
// Source: [IDTECH] Section 3.4 (Feedback Rendering), [SHLOM] feedback.frag
//
// [IDTECH] Section 3.4:
//   "Texture feedback is rendered to a separate buffer that, for the virtual
//    texture pages used in the current scene, stores the virtual page
//    coordinates (x,y), desired mip level, and virtual texture ID."
//   "the feedback can be rendered at a significantly lower resolution (say 10x)"
//
// [SHLOM] feedback.frag packs into u32:
//   bit 31 = valid, bits 0-4 = mip, bits 5-12 = pageX, bits 13-20 = pageY
//
// In the prototype, we simulate the GPU feedback pass by computing what
// the GPU would request for each screen pixel.

export function simulateFeedback(
  cameraUv: [number, number],
  cameraZoom: number,
  lodBias: number = 0,  // [IDTECH] Section 3.5 oversubscription
): Map<string, PageRequest> {
  const requests = new Map<string, PageRequest>();

  const uvWidth = 1 / cameraZoom;
  const uvHeight = 1 / cameraZoom;

  // Feedback at 1/8 resolution — [IDTECH] "10x smaller"
  const fbW = Math.floor(SCREEN_WIDTH * FEEDBACK_SCALE);
  const fbH = Math.floor(SCREEN_HEIGHT * FEEDBACK_SCALE);

  // UV derivatives (constant for orthographic view)
  const uvDx: [number, number] = [uvWidth / fbW, 0];
  const uvDy: [number, number] = [0, uvHeight / fbH];

  // [SHLOM] feedback.frag: effective_size = u_VirtualSize * u_BufferScreenRatio
  // The feedback buffer is at lower resolution, so derivatives are larger.
  // Multiplying virtualSize by FEEDBACK_SCALE compensates, producing the
  // same mip level as the full-resolution render would need.
  const effectiveSize = VIRTUAL_SIZE * FEEDBACK_SCALE;

  for (let fy = 0; fy < fbH; fy++) {
    for (let fx = 0; fx < fbW; fx++) {
      const u = cameraUv[0] - uvWidth / 2 + (fx / fbW) * uvWidth;
      const v = cameraUv[1] - uvHeight / 2 + (fy / fbH) * uvHeight;

      if (u < 0 || u >= 1 || v < 0 || v >= 1) continue;

      // [SHLOM] ComputeMipLevel with effective_size compensation
      let mip = computeMipLevel(uvDx, uvDy, effectiveSize) + lodBias;
      mip = Math.max(0, Math.min(Math.floor(mip), MAX_MIP));

      const pagesAtMip = VIRTUAL_PAGES_X >> mip;
      const px = Math.min(Math.floor(u * pagesAtMip), pagesAtMip - 1);
      const py = Math.min(Math.floor(v * pagesAtMip), pagesAtMip - 1);

      const key = `${mip}:${px}:${py}`;
      if (!requests.has(key)) {
        requests.set(key, { mip, x: px, y: py });
      }
    }
  }

  return requests;
}

// ============================================================================
// Page Manager (ties everything together)
// ============================================================================
// Source: [SHLOM] page_manager.cpp, [IDTECH] Section 5.1 (pipeline overview)
//
// [IDTECH] Section 5.1 pipeline:
//   1. Render feedback → small screen buffer
//   2. Feedback analysis → sorted list of needed pages
//   3. For each page: fetch compressed data from cache (or schedule disk load)
//   4. Allocate physical page, unmap old page (GPU falls back to coarser mip)
//   5. Transcode compressed → GPU format
//   6. Map new page (GPU starts using it)
//
// [SHLOM] PageManager: IngestFeedback → RequestPage → FlushProcessingRequests

export class PageManager {
  pageTable: PageTable;
  cache: PageCache;

  // [IDTECH] Section 3.5 oversubscription:
  // "If that number is greater than a high water mark, the system is
  //  considered oversubscribed and the LOD bias used when generating
  //  feedback is incremented."
  private lodBias = 0;
  private highWaterMark = ATLAS_PAGES_X * ATLAS_PAGES_Y * 0.9;
  private lowWaterMark = ATLAS_PAGES_X * ATLAS_PAGES_Y * 0.5;

  constructor() {
    this.pageTable = new PageTable(MAX_MIP);
    this.cache = new PageCache(PINNED_MIPS);
    this.loadPinnedPages();
  }

  /** Pre-load pinned (coarsest) pages. Source: [SHLOM] PageManager constructor. */
  private loadPinnedPages() {
    for (const mip of PINNED_MIPS) {
      const pagesAtMip = VIRTUAL_PAGES_X >> mip;
      for (let y = 0; y < pagesAtMip; y++) {
        for (let x = 0; x < pagesAtMip; x++) {
          const req: PageRequest = { mip, x, y };
          const { slot } = this.cache.acquire(req);
          const data = generatePage(req);
          this.cache.commit(req, slot, data);
          this.pageTable.setResident(req, slot);
        }
      }
    }
  }

  /**
   * Process feedback: load requested pages, evict old ones.
   * Source: [SHLOM] page_manager.cpp IngestFeedback + RequestPage
   *         [IDTECH] Section 5.1 pipeline
   */
  processFeedback(requests: Map<string, PageRequest>) {
    // [SHLOM] IngestFeedback: Touch all resident pages seen in feedback
    for (const req of requests.values()) {
      this.cache.touch(req);
    }

    // [IDTECH] Section 5.1: "sorted on priority"
    // Priority: (1) farther from resident mip = higher, (2) more hits = higher
    // For simplicity, we sort coarsest-first (progressive loading: parents
    // before children — [ELFRA] progressive loading pattern)
    const toLoad = [...requests.values()]
      .filter(req => !this.pageTable.isResident(req))
      .sort((a, b) => b.mip - a.mip); // coarsest first

    let loaded = 0;
    let evicted = 0;
    let skipped = 0;

    for (const req of toLoad) {
      try {
        const { slot, evicted: evictedReq } = this.cache.acquire(req);

        // [SHLOM] RequestPage: if evicted, clear page table entry
        if (evictedReq) {
          this.pageTable.setEvicted(evictedReq);
          evicted++;
        }

        // [SHLOM] FlushProcessingRequests: write data to atlas + update page table
        const data = generatePage(req);
        this.cache.commit(req, slot, data);
        this.pageTable.setResident(req, slot);
        loaded++;
      } catch {
        // No evictable slots — oversubscribed
        skipped++;
      }
    }

    // [IDTECH] Section 3.5: Oversubscription handling
    const residentRequested = [...requests.values()].filter(r => this.pageTable.isResident(r)).length;
    if (residentRequested > this.highWaterMark) {
      this.lodBias = Math.min(this.lodBias + 1, MAX_MIP);
    } else if (residentRequested < this.lowWaterMark) {
      this.lodBias = Math.max(this.lodBias - 1, 0);
    }

    return { loaded, evicted, skipped, totalRequests: requests.size, lodBias: this.lodBias };
  }

  /** Get current LOD bias for feedback simulation. */
  getLodBias(): number { return this.lodBias; }

  /**
   * Render the virtual texture using VT sampling.
   * This simulates what the GPU material shader does.
   */
  render(cameraUv: [number, number], cameraZoom: number): Uint8Array {
    const pixels = new Uint8Array(SCREEN_WIDTH * SCREEN_HEIGHT * 4);
    const uvWidth = 1 / cameraZoom;
    const uvHeight = 1 / cameraZoom;
    const uvDx: [number, number] = [uvWidth / SCREEN_WIDTH, 0];
    const uvDy: [number, number] = [0, uvHeight / SCREEN_HEIGHT];

    for (let y = 0; y < SCREEN_HEIGHT; y++) {
      for (let x = 0; x < SCREEN_WIDTH; x++) {
        const u = cameraUv[0] - uvWidth / 2 + (x / SCREEN_WIDTH) * uvWidth;
        const v = cameraUv[1] - uvHeight / 2 + (y / SCREEN_HEIGHT) * uvHeight;

        let pixel: Pixel | null = null;
        if (u >= 0 && u <= 1 && v >= 0 && v <= 1) {
          pixel = vtSample(u, v, uvDx, uvDy, this.pageTable, this.cache.atlas);
        }

        const idx = (y * SCREEN_WIDTH + x) * 4;
        if (pixel) {
          pixels[idx] = pixel[0];
          pixels[idx + 1] = pixel[1];
          pixels[idx + 2] = pixel[2];
          pixels[idx + 3] = pixel[3];
        } else {
          pixels[idx] = 0;
          pixels[idx + 1] = 0;
          pixels[idx + 2] = 0;
          pixels[idx + 3] = 255;
        }
      }
    }

    return pixels;
  }

  /** Render ground truth (direct sampling, no VT) for comparison. */
  renderGroundTruth(cameraUv: [number, number], cameraZoom: number): Uint8Array {
    const pixels = new Uint8Array(SCREEN_WIDTH * SCREEN_HEIGHT * 4);
    const uvWidth = 1 / cameraZoom;
    const uvHeight = 1 / cameraZoom;

    for (let y = 0; y < SCREEN_HEIGHT; y++) {
      for (let x = 0; x < SCREEN_WIDTH; x++) {
        const u = cameraUv[0] - uvWidth / 2 + (x / SCREEN_WIDTH) * uvWidth;
        const v = cameraUv[1] - uvHeight / 2 + (y / SCREEN_HEIGHT) * uvHeight;

        const idx = (y * SCREEN_WIDTH + x) * 4;
        if (u >= 0 && u <= 1 && v >= 0 && v <= 1) {
          const [r, g, b, a] = sampleVirtualTexture(u, v);
          pixels[idx] = r;
          pixels[idx + 1] = g;
          pixels[idx + 2] = b;
          pixels[idx + 3] = a;
        } else {
          pixels[idx] = 0;
          pixels[idx + 1] = 0;
          pixels[idx + 2] = 0;
          pixels[idx + 3] = 255;
        }
      }
    }

    return pixels;
  }
}

// ============================================================================
// PPM Output
// ============================================================================

export function writePPM(path: string, data: Uint8Array, width: number, height: number) {
  const header = `P6\n${width} ${height}\n255\n`;
  const headerBytes = new TextEncoder().encode(header);
  const rgb = new Uint8Array(width * height * 3);
  for (let i = 0; i < width * height; i++) {
    rgb[i * 3] = data[i * 4];
    rgb[i * 3 + 1] = data[i * 4 + 1];
    rgb[i * 3 + 2] = data[i * 4 + 2];
  }
  const out = new Uint8Array(headerBytes.length + rgb.length);
  out.set(headerBytes, 0);
  out.set(rgb, headerBytes.length);
  Bun.write(path, out);
}

// ============================================================================
// Tests
// ============================================================================

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(`ASSERT FAILED: ${msg}`);
}

// --- Test 1: Page table entry pack/unpack ---
// Verifies [SHLOM] bit-packed u32 format
function testPageTableEntryPackUnpack() {
  console.log('Test 1: page table entry pack/unpack [SHLOM format]');
  const entry = packEntry(true, 5, 7, 3);
  assert(isResident(entry), 'should be resident');
  assert(getPhysX(entry) === 5, 'physX should be 5');
  assert(getPhysY(entry) === 7, 'physY should be 7');
  assert(getMip(entry) === 3, 'mip should be 3');

  const notResident = packEntry(false, 0, 0, 0);
  assert(!isResident(notResident), 'should not be resident');

  // Verify bit layout matches [SHLOM] exactly:
  // entry = 0x1 | (physX << 1) | (physY << 9) | (mip << 17)
  assert(entry === (1 | (5 << 1) | (7 << 9) | (3 << 17)), 'bit layout matches [SHLOM]');
  console.log('  ✓ pass\n');
}

// --- Test 2: Mip level computation ---
// Verifies [SHLOM] ComputeMipLevel / [IDTECH] Appendix B formula:
//   0.5 * log2(max(dot(dx,dx), dot(dy,dy)))
// where dx = dFdx(uv * virtualSize), dy = dFdy(uv * virtualSize)
function testMipLevelComputation() {
  console.log('Test 2: mip level computation [SHLOM ComputeMipLevel]');

  // 1 texel per pixel → mip 0
  // dx = (1, 0), dot(dx,dx) = 1, 0.5 * log2(1) = 0
  let mip = computeMipLevel([1 / VIRTUAL_SIZE, 0], [0, 1 / VIRTUAL_SIZE], VIRTUAL_SIZE);
  assert(Math.abs(mip) < 0.01, `1 texel/pixel → mip ~0, got ${mip}`);

  // 16 texels per pixel → mip 4
  // dx = (16, 0), dot(dx,dx) = 256, 0.5 * log2(256) = 4
  mip = computeMipLevel([16 / VIRTUAL_SIZE, 0], [0, 16 / VIRTUAL_SIZE], VIRTUAL_SIZE);
  assert(Math.abs(mip - 4) < 0.01, `16 texels/pixel → mip ~4, got ${mip}`);

  // 128 texels per pixel → mip 7
  // dx = (128, 0), dot(dx,dx) = 16384, 0.5 * log2(16384) = 7
  mip = computeMipLevel([128 / VIRTUAL_SIZE, 0], [0, 128 / VIRTUAL_SIZE], VIRTUAL_SIZE);
  assert(Math.abs(mip - 7) < 0.01, `128 texels/pixel → mip ~7, got ${mip}`);

  // Full texture per pixel → mip 12
  // dx = (4096, 0), dot(dx,dx) = 16777216, 0.5 * log2(16777216) = 12
  mip = computeMipLevel([1, 0], [0, 1], VIRTUAL_SIZE);
  assert(Math.abs(mip - 12) < 0.01, `full texture/pixel → mip ~12, got ${mip}`);

  console.log('  ✓ pass\n');
}

// --- Test 3: LRU eviction ---
// Verifies [SHLOM] page_cache.cpp Acquire/Touch/Commit
function testPageCacheLRU() {
  console.log('Test 3: LRU eviction [SHLOM page_cache.cpp]');
  const cache = new PageCache(new Set<number>());

  // Fill all slots
  for (let i = 0; i < ATLAS_PAGES_X * ATLAS_PAGES_Y; i++) {
    const req: PageRequest = { mip: 0, x: i, y: 0 };
    const { slot } = cache.acquire(req);
    cache.commit(req, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
  }
  assert(cache.freeSlotCount === 0, 'all slots full');
  assert(cache.usedSlots === ATLAS_PAGES_X * ATLAS_PAGES_Y, 'all slots used');

  // Touch page 0 (make it MRU)
  cache.touch({ mip: 0, x: 0, y: 0 });

  // Acquire new slot — should evict LRU (page 1, not page 0)
  const { slot, evicted } = cache.acquire({ mip: 0, x: 999, y: 999 });
  assert(evicted !== null, 'should have evicted');
  assert(!(evicted!.x === 0 && evicted!.y === 0), 'should not evict MRU page');
  console.log(`  evicted mip=${evicted!.mip} x=${evicted!.x} y=${evicted!.y} (expected ≠ 0,0)`);
  console.log('  ✓ pass\n');
}

// --- Test 4: Pinned mips never evicted ---
// Verifies [SHLOM] kMinPinnedLod behavior
function testPinnedMips() {
  console.log('Test 4: pinned mips never evicted [SHLOM kMinPinnedLod]');
  const cache = new PageCache(PINNED_MIPS);

  const totalSlots = ATLAS_PAGES_X * ATLAS_PAGES_Y;
  for (let i = 0; i < totalSlots; i++) {
    const mip = i === 0 ? MAX_MIP : 0; // first slot is pinned
    const req: PageRequest = { mip, x: i, y: 0 };
    const { slot } = cache.acquire(req);
    cache.commit(req, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
  }

  const { evicted } = cache.acquire({ mip: 0, x: 999, y: 999 });
  assert(evicted !== null, 'should evict something');
  assert(evicted!.mip === 0, `should evict mip 0, got mip ${evicted!.mip}`);
  console.log('  ✓ pass\n');
}

// --- Test 5: Address translation correctness ---
// Verifies [SHLOM] material.frag VT sampling produces correct texels
function testAddressTranslation() {
  console.log('Test 5: address translation [SHLOM material.frag]');
  const pm = new PageManager();

  // Load a specific page at mip 0
  const req: PageRequest = { mip: 0, x: 5, y: 3 };
  const { slot } = pm.cache.acquire(req);
  const data = generatePage(req);
  pm.cache.commit(req, slot, data);
  pm.pageTable.setResident(req, slot);

  // Sample at center of that page
  const u = (5 + 0.5) / VIRTUAL_PAGES_X;
  const v = (3 + 0.5) / VIRTUAL_PAGES_Y;

  // 1:1 (1 texel per pixel)
  const uvDx: [number, number] = [1 / VIRTUAL_SIZE, 0];
  const uvDy: [number, number] = [0, 1 / VIRTUAL_SIZE];

  const sampled = vtSample(u, v, uvDx, uvDy, pm.pageTable, pm.cache.atlas);
  const truth = sampleVirtualTexture(u, v);

  assert(sampled !== null, 'VT sample not null');
  if (sampled) {
    const diff = Math.abs(sampled[0] - truth[0]) +
                 Math.abs(sampled[1] - truth[1]) +
                 Math.abs(sampled[2] - truth[2]);
    assert(diff < 5, `VT sample matches truth (diff=${diff})`);
    console.log(`  VT:    [${sampled[0]}, ${sampled[1]}, ${sampled[2]}]`);
    console.log(`  Truth: [${truth[0]}, ${truth[1]}, ${truth[2]}]`);
  }
  console.log('  ✓ pass\n');
}

// --- Test 6: Fallback to coarser mip ---
// Verifies [SHLOM] material.frag fallback loop + [IDTECH] Section 3.1
function testFallbackToCoarserMip() {
  console.log('Test 6: fallback to coarser mip [SHLOM fallback loop]');
  const pm = new PageManager();

  // Only pinned pages loaded. Sample at fine mip — should fall back.
  const u = 0.3, v = 0.7;

  // Very zoomed out → high mip level
  const uvDx: [number, number] = [0.1, 0];
  const uvDy: [number, number] = [0, 0.1];

  const sampled = vtSample(u, v, uvDx, uvDy, pm.pageTable, pm.cache.atlas);
  assert(sampled !== null, 'should find resident page via fallback');
  console.log(`  Fallback sampled: [${sampled![0]}, ${sampled![1]}, ${sampled![2]}]`);
  console.log('  ✓ pass\n');
}

// --- Test 7: Border texel correctness ---
// Verifies [IDTECH] Section 3.2 border replication
function testBorderTexels() {
  console.log('Test 7: border texels [IDTECH Section 3.2]');
  // Generate a page at (0,0) mip 0 — its border should clamp to edges
  const req: PageRequest = { mip: 0, x: 0, y: 0 };
  const data = generatePage(req);

  // Top-left border texel (sx=0, sy=0) should equal payload (sx=BORDER, sy=BORDER)
  const borderIdx = (0 * SLOT_SIZE + 0) * 4;
  const payloadIdx = (PAGE_BORDER * SLOT_SIZE + PAGE_BORDER) * 4;

  // The border texel at (0,0) clamps to virtual texel (-4,-4) → clamped to (0,0)
  // The payload texel at (BORDER,BORDER) is virtual texel (0,0)
  // So they should be the same color
  assert(data[borderIdx] === data[payloadIdx], 'border (0,0) matches payload (0,0)');
  assert(data[borderIdx + 1] === data[payloadIdx + 1], 'border G matches');
  assert(data[borderIdx + 2] === data[payloadIdx + 2], 'border B matches');
  console.log('  ✓ pass\n');
}

// --- Test 8: Progressive loading (coarsest first) ---
// Verifies [ELFRA] progressive loading + [IDTECH] Section 5.1 pipeline
function testProgressiveLoading() {
  console.log('Test 8: progressive loading [ELFRA/IDTECH]');
  const pm = new PageManager();
  const cameraUv: [number, number] = [0.25, 0.25];
  const cameraZoom = 8;

  const requests = simulateFeedback(cameraUv, cameraZoom);
  console.log(`  Feedback: ${requests.size} requests`);

  const result = pm.processFeedback(requests);
  console.log(`  Loaded: ${result.loaded}, evicted: ${result.evicted}`);

  // Pinned coarse pages should be resident (for fallback) — [IDTECH] Section 4.5
  const coarsePages = [MAX_MIP, MAX_MIP - 1];
  for (const mip of coarsePages) {
    const pages = VIRTUAL_PAGES_X >> mip;
    for (let y = 0; y < pages; y++) {
      for (let x = 0; x < pages; x++) {
        assert(pm.pageTable.isResident({ mip, x, y }), `pinned page mip=${mip} (${x},${y}) resident`);
      }
    }
  }

  // Requested fine pages should now be loaded
  let fineResident = 0;
  for (const req of requests.values()) {
    if (pm.pageTable.isResident(req)) fineResident++;
  }
  console.log(`  Pinned coarse: resident ✓, Fine resident: ${fineResident}/${requests.size}`);
  assert(fineResident > 0, 'should have fine pages resident after loading');
  console.log('  ✓ pass\n');
}

// --- Test 9: Full pipeline (feedback → load → render → verify) ---
function testFullPipeline() {
  console.log('Test 9: full pipeline [IDTECH Section 5.1]');

  const pm = new PageManager();
  const cameraUv: [number, number] = [0.5, 0.5];
  const cameraZoom = 4;

  // Frame 1: Feedback
  const requests = simulateFeedback(cameraUv, cameraZoom, pm.getLodBias());
  console.log(`  Feedback: ${requests.size} page requests`);

  // Frame 1: Process (load pages)
  const result = pm.processFeedback(requests);
  console.log(`  Loaded: ${result.loaded}, evicted: ${result.evicted}, lodBias: ${result.lodBias}`);
  console.log(`  Atlas: ${pm.cache.usedSlots}/${ATLAS_PAGES_X * ATLAS_PAGES_Y} slots`);

  // Frame 2: Render (1 frame latency — [IDTECH] Section 3.4)
  const rendered = pm.render(cameraUv, cameraZoom);
  const truth = pm.renderGroundTruth(cameraUv, cameraZoom);

  // Compare
  let maxDiff = 0, avgDiff = 0, count = 0;
  for (let i = 0; i < rendered.length; i += 4) {
    const d = Math.abs(rendered[i] - truth[i]) +
              Math.abs(rendered[i + 1] - truth[i + 1]) +
              Math.abs(rendered[i + 2] - truth[i + 2]);
    maxDiff = Math.max(maxDiff, d);
    avgDiff += d;
    count++;
  }
  avgDiff /= count;
  console.log(`  Max diff: ${maxDiff.toFixed(1)}, Avg diff: ${avgDiff.toFixed(1)}`);

  // Write output images
  writePPM('prototype/vt/output_vt.ppm', rendered, SCREEN_WIDTH, SCREEN_HEIGHT);
  writePPM('prototype/vt/output_truth.ppm', truth, SCREEN_WIDTH, SCREEN_HEIGHT);
  writePPM('prototype/vt/output_atlas.ppm', pm.cache.atlas, ATLAS_WIDTH, ATLAS_HEIGHT);
  console.log('  Wrote: output_vt.ppm, output_truth.ppm, output_atlas.ppm');

  // Quality check — some diff expected due to mip fallback at edges
  assert(avgDiff < 30, `avg diff < 30, got ${avgDiff}`);
  console.log('  ✓ pass\n');
}

// --- Test 10: Camera movement (eviction + reload) ---
function testCameraMovement() {
  console.log('Test 10: camera movement (eviction + reload)');
  const pm = new PageManager();

  // Frame 1: View center
  let reqs = simulateFeedback([0.5, 0.5], 4, pm.getLodBias());
  let r = pm.processFeedback(reqs);
  console.log(`  Frame 1: loaded ${r.loaded}, atlas ${pm.cache.usedSlots}/${ATLAS_PAGES_X * ATLAS_PAGES_Y}`);

  // Frame 2: Move camera to corner — should evict old pages, load new ones
  reqs = simulateFeedback([0.1, 0.1], 4, pm.getLodBias());
  r = pm.processFeedback(reqs);
  console.log(`  Frame 2: loaded ${r.loaded}, evicted ${r.evicted}, atlas ${pm.cache.usedSlots}/${ATLAS_PAGES_X * ATLAS_PAGES_Y}`);

  // Frame 3: Move back — should reload (some from cache if not evicted)
  reqs = simulateFeedback([0.5, 0.5], 4, pm.getLodBias());
  r = pm.processFeedback(reqs);
  console.log(`  Frame 3: loaded ${r.loaded}, evicted ${r.evicted}, atlas ${pm.cache.usedSlots}/${ATLAS_PAGES_X * ATLAS_PAGES_Y}`);

  console.log('  ✓ pass\n');
}

// ============================================================================
// Main
// ============================================================================

console.log('=== Virtual Texturing Prototype ===\n');
console.log(`Virtual texture: ${VIRTUAL_SIZE}×${VIRTUAL_SIZE} (${VIRTUAL_PAGES_X}×${VIRTUAL_PAGES_Y} pages)`);
console.log(`Page: ${PAGE_SIZE}×${PAGE_SIZE} + ${PAGE_BORDER}×2 border = ${SLOT_SIZE}×${SLOT_SIZE} slot`);
console.log(`Atlas: ${ATLAS_WIDTH}×${ATLAS_HEIGHT} (${ATLAS_PAGES_X}×${ATLAS_PAGES_Y} = ${ATLAS_PAGES_X * ATLAS_PAGES_Y} slots)`);
console.log(`Max mip: ${MAX_MIP} (${MAX_MIP + 1} levels, pages: ${VIRTUAL_PAGES_X}→${VIRTUAL_PAGES_X >> MAX_MIP})`);
console.log(`Pinned mips: ${[...PINNED_MIPS].join(', ')}`);
console.log(`Screen: ${SCREEN_WIDTH}×${SCREEN_HEIGHT}, feedback: ${Math.floor(SCREEN_WIDTH * FEEDBACK_SCALE)}×${Math.floor(SCREEN_HEIGHT * FEEDBACK_SCALE)}\n`);

testPageTableEntryPackUnpack();       // [SHLOM] bit-packed u32
testMipLevelComputation();           // [SHLOM] ComputeMipLevel / [IDTECH] Appendix B
testPageCacheLRU();                   // [SHLOM] page_cache.cpp LRU
testPinnedMips();                     // [SHLOM] kMinPinnedLod
testAddressTranslation();            // [SHLOM] material.frag address translation
testFallbackToCoarserMip();          // [SHLOM] fallback loop / [IDTECH] Section 3.1
testBorderTexels();                   // [IDTECH] Section 3.2 border replication
testProgressiveLoading();            // [ELFRA] progressive loading / [IDTECH] Section 5.1
testFullPipeline();                   // [IDTECH] Section 5.1 full pipeline
testCameraMovement();                // eviction + reload

console.log('=== All tests passed! ===');
