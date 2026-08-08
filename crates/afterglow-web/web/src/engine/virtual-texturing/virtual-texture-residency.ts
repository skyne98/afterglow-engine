import { FixedPageSlotMap } from './fixed-page-slot-map.ts';
import {
  ATLAS_WIDTH,
  BLOCK_SIZE,
  SLOT_BLOCKS_X,
  SLOT_BLOCKS_Y,
  SLOT_SIZE,
  bytesPerBlock,
  isCompressedTextureFormat,
  isPageTableEntryResident as isResident,
  uncompressedBytesPerTexel,
  packPageTableEntry as packEntry,
} from './virtual-texture-format.ts';
import type { CachedPage, PageRequest } from './virtual-texture-request.ts';
import {
  type PackedPageTableLayout,
  packedPageTableIndex,
} from './virtual-texture-layout.ts';

export interface PageSlot {
  x: number;
  y: number;
}


// ============================================================================
// Page Table
// ============================================================================

export class PageTable {
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

export class PageCache {
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
    if (!isCompressedTextureFormat(format)) {
      const bytesPerTexel = uncompressedBytesPerTexel(format);
      this.atlasBytesPerRow = this.width * bytesPerTexel;
      this.slotBytesPerRow = SLOT_SIZE * bytesPerTexel;
      this.slotDataSize = SLOT_SIZE * SLOT_SIZE * bytesPerTexel;
      this.atlas = new Uint8Array(this.width * this.height * bytesPerTexel);
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

  private writeSlot(slot: PageSlot, data: Uint8Array): void {
    if (data.byteLength !== this.slotDataSize)
      throw new RangeError(`VT page has ${data.byteLength} bytes; expected ${this.slotDataSize}`);
    const compressed = isCompressedTextureFormat(this.format);
    const rows = compressed ? SLOT_BLOCKS_Y : SLOT_SIZE;
    const dstXBytes = compressed
      ? slot.x * SLOT_BLOCKS_X * (this.slotBytesPerRow / SLOT_BLOCKS_X)
      : slot.x * this.slotBytesPerRow;
    const dstY = compressed ? slot.y * SLOT_BLOCKS_Y : slot.y * SLOT_SIZE;
    for (let row = 0; row < rows; row++) {
      const srcOffset = row * this.slotBytesPerRow;
      const dstOffset = (dstY + row) * this.atlasBytesPerRow + dstXBytes;
      this.atlas.set(data.subarray(srcOffset, srcOffset + this.slotBytesPerRow), dstOffset);
    }
  }

  /** Write page data into a slot and mark as resident. */
  commit(req: CachedPage, slot: PageSlot, data: Uint8Array) {
    this.writeSlot(slot, data);
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


  /** Replace one resident page in place, retaining its physical identity. */
  replaceByKey(cacheKey: number, data: Uint8Array): PageSlot | null {
    const index = this.slotByKey.get(cacheKey);
    if (index === undefined || this.slotActive[index] === 0 || this.reserved[index] !== 0) return null;
    const slot = this.slotCoords[index];
    this.writeSlot(slot, data);
    this.referenced[index] = 1;
    return slot;
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
  get reservedSlots(): number {
    let count = 0;
    for (let i = 0; i < this.reserved.length; i++) if (this.reserved[i] !== 0) count++;
    return count;
  }
  get totalSlots(): number { return this.pagesX * this.pagesY; }
}
