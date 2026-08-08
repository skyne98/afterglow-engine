import { FixedByteLeasePool } from '../streaming/fixed-byte-lease-pool.ts';
import { FixedPageSlotMap } from './fixed-page-slot-map.ts';
import { PAGE_BORDER, PAGE_SIZE, SLOT_SIZE } from './virtual-texture-format.ts';
import type { PageDataProvider } from './virtual-texture.ts';
import type { PageRequest } from './virtual-texture-request.ts';

export type MemoryVirtualTextureFormat = 'rgba8unorm' | 'r8unorm' | 'r16float';
export type MemoryVirtualTextureMipFilter = 'linear-color' | 'normal' | 'scalar';
export type MemoryVirtualTextureAddressMode = 'clamp' | 'repeat' | 'mirror-repeat';

export interface MemoryPageSourceOptions {
  readonly width: number;
  readonly height: number;
  readonly format: MemoryVirtualTextureFormat;
  readonly mipFilter: MemoryVirtualTextureMipFilter;
  readonly addressMode: MemoryVirtualTextureAddressMode;
  /** Canonical/derived 128×128 pages retained in the fixed RAM pool. */
  readonly pageCapacity: number;
  /** Distinct page revisions waiting for bounded VRAM refresh. */
  readonly dirtyCapacity: number;
  /** Reusable bordered payloads retained by asynchronous page reads. */
  readonly outputCapacity: number;
  readonly defaultTexel?: Uint8Array;
}

export enum MemoryTextureWriteStatus {
  Written = 0,
  InvalidRegion = 1,
  PageCapacityExceeded = 2,
  DirtyCapacityExceeded = 3,
}

export interface MemoryTextureDirtyPage {
  mip: number;
  x: number;
  y: number;
  revision: number;
}

export type MemoryCanonicalPageVisitor = (
  x: number,
  y: number,
  storage: Uint8Array,
  byteOffset: number,
  byteLength: number,
) => void;

const PAGE_COORD_SCALE = 2048;
const MIP_COORD_SCALE = PAGE_COORD_SCALE * PAGE_COORD_SCALE;

function pageKey(mip: number, x: number, y: number): number {
  return mip * MIP_COORD_SCALE + y * PAGE_COORD_SCALE + x + 1;
}

function bytesPerTexel(format: MemoryVirtualTextureFormat): number {
  if (format === 'rgba8unorm') return 4;
  if (format === 'r16float') return 2;
  return 1;
}

const HALF_CONVERSION_BUFFER = new ArrayBuffer(4);
const HALF_CONVERSION_FLOAT = new Float32Array(HALF_CONVERSION_BUFFER);
const HALF_CONVERSION_BITS = new Uint32Array(HALF_CONVERSION_BUFFER);

function floatToHalf(value: number): number {
  HALF_CONVERSION_FLOAT[0] = value;
  const bits = HALF_CONVERSION_BITS[0] ?? 0;
  const sign = (bits >>> 16) & 0x8000;
  let exponent = ((bits >>> 23) & 0xff) - 127 + 15;
  let mantissa = bits & 0x7fffff;
  if (exponent <= 0) {
    if (exponent < -10) return sign;
    mantissa = (mantissa | 0x800000) >>> (1 - exponent);
    return sign | ((mantissa + 0x1000) >>> 13);
  }
  if (exponent >= 31) return sign | 0x7c00;
  return sign | (exponent << 10) | ((mantissa + 0x1000) >>> 13);
}

function srgbToLinear(value: number): number {
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function linearToSrgb(value: number): number {
  return value <= 0.0031308 ? value * 12.92 : 1.055 * value ** (1 / 2.4) - 0.055;
}

function halfToFloat(value: number): number {
  const sign = (value & 0x8000) !== 0 ? -1 : 1;
  const exponent = (value >>> 10) & 0x1f;
  const mantissa = value & 0x3ff;
  if (exponent === 0) return sign * 2 ** -14 * (mantissa / 1024);
  if (exponent === 31) return mantissa === 0 ? sign * Infinity : Number.NaN;
  return sign * 2 ** (exponent - 15) * (1 + mantissa / 1024);
}

/**
 * Fixed-capacity sparse canonical RAM texture. Mip zero stores only touched
 * pages; derived pages are generated recursively and retained in the same
 * bounded pool. Untouched pages resolve to one configured default texel.
 */
export class MemoryVirtualTextureSource {
  readonly provider: PageDataProvider;
  readonly bytesPerTexel: number;
  readonly maxMip: number;
  private readonly pageBytes: number;
  private readonly storage: Uint8Array;
  private readonly slotMip: Uint8Array;
  private readonly slotX: Uint16Array;
  private readonly slotY: Uint16Array;
  private readonly slotDirty: Uint8Array;
  private readonly slotRevision: Uint32Array;
  private readonly slotsByPage: FixedPageSlotMap;
  private readonly freeSlots: Uint32Array;
  private freeTop = 0;
  private readonly dirtyMip: Uint8Array;
  private readonly dirtyX: Uint16Array;
  private readonly dirtyY: Uint16Array;
  private readonly dirtyRevision: Uint32Array;
  private readonly dirtyKeys: FixedPageSlotMap;
  private dirtyHead = 0;
  private dirtyTail = 0;
  private dirtyCount = 0;
  private revision = 0;
  private readonly defaultTexel: Uint8Array;
  private readonly scratchTexel = new Float32Array(4);
  private readonly scratchSample = new Float32Array(4);
  private readonly refreshScratch: Uint8Array;
  private readonly outputPool: FixedByteLeasePool;
  private readonly dirtyPage: MemoryTextureDirtyPage = { mip: 0, x: 0, y: 0, revision: 0 };

  readonly options: Readonly<MemoryPageSourceOptions>;

  constructor(options: Readonly<MemoryPageSourceOptions>) {
    this.options = options.defaultTexel
      ? { ...options, defaultTexel: options.defaultTexel.slice() }
      : { ...options };
    if (!Number.isInteger(options.width) || options.width < 1 ||
        !Number.isInteger(options.height) || options.height < 1 ||
        !Number.isInteger(options.pageCapacity) || options.pageCapacity < 1 ||
        !Number.isInteger(options.dirtyCapacity) || options.dirtyCapacity < 1 ||
        !Number.isInteger(options.outputCapacity) || options.outputCapacity < 1)
      throw new RangeError('invalid memory virtual-texture capacities or dimensions');
    this.bytesPerTexel = bytesPerTexel(options.format);
    this.pageBytes = PAGE_SIZE * PAGE_SIZE * this.bytesPerTexel;
    const outputBytes = SLOT_SIZE * SLOT_SIZE * this.bytesPerTexel;
    this.refreshScratch = new Uint8Array(outputBytes);
    this.outputPool = new FixedByteLeasePool(options.outputCapacity, outputBytes);
    const pagesX = Math.ceil(options.width / PAGE_SIZE);
    const pagesY = Math.ceil(options.height / PAGE_SIZE);
    this.maxMip = Math.ceil(Math.log2(Math.max(pagesX, pagesY)));
    if (pagesX > PAGE_COORD_SCALE || pagesY > PAGE_COORD_SCALE || this.maxMip > 31)
      throw new RangeError('memory virtual texture exceeds page-coordinate limits');
    this.defaultTexel = options.defaultTexel?.slice() ?? new Uint8Array(this.bytesPerTexel);
    if (this.defaultTexel.byteLength !== this.bytesPerTexel)
      throw new RangeError('default texel byte length does not match memory texture format');
    this.storage = new Uint8Array(options.pageCapacity * this.pageBytes);
    this.slotMip = new Uint8Array(options.pageCapacity);
    this.slotX = new Uint16Array(options.pageCapacity);
    this.slotY = new Uint16Array(options.pageCapacity);
    this.slotDirty = new Uint8Array(options.pageCapacity);
    this.slotRevision = new Uint32Array(options.pageCapacity);
    this.slotsByPage = new FixedPageSlotMap(options.pageCapacity);
    this.freeSlots = new Uint32Array(options.pageCapacity);
    for (let slot = options.pageCapacity - 1; slot >= 0; slot--) this.freeSlots[this.freeTop++] = slot;
    this.dirtyMip = new Uint8Array(options.dirtyCapacity);
    this.dirtyX = new Uint16Array(options.dirtyCapacity);
    this.dirtyY = new Uint16Array(options.dirtyCapacity);
    this.dirtyRevision = new Uint32Array(options.dirtyCapacity);
    this.dirtyKeys = new FixedPageSlotMap(options.dirtyCapacity);
    this.provider = async (_path, request) => {
      const lease = this.outputPool.tryAcquire();
      if (!lease) throw new Error('memory virtual-texture output pool is full');
      try {
        this.readPageInto(request, lease.bytes);
        return lease;
      } catch (error) {
        lease.release();
        throw error;
      }
    };
  }

  private pagesX(mip: number): number {
    return Math.ceil(Math.max(1, Math.ceil(this.options.width / 2 ** mip)) / PAGE_SIZE);
  }

  private pagesY(mip: number): number {
    return Math.ceil(Math.max(1, Math.ceil(this.options.height / 2 ** mip)) / PAGE_SIZE);
  }

  private slot(mip: number, x: number, y: number): number | undefined {
    return this.slotsByPage.get(pageKey(mip, x, y));
  }

  private acquirePage(mip: number, x: number, y: number): number | undefined {
    const existing = this.slot(mip, x, y);
    if (existing !== undefined) return existing;
    if (this.freeTop === 0) return undefined;
    const slot = this.freeSlots[--this.freeTop] ?? 0;
    this.slotMip[slot] = mip;
    this.slotX[slot] = x;
    this.slotY[slot] = y;
    this.slotDirty[slot] = mip === 0 ? 0 : 1;
    this.slotRevision[slot] = this.revision;
    const start = slot * this.pageBytes;
    for (let texel = 0; texel < PAGE_SIZE * PAGE_SIZE; texel++)
      this.storage.set(this.defaultTexel, start + texel * this.bytesPerTexel);
    this.slotsByPage.set(pageKey(mip, x, y), slot);
    return slot;
  }

  private countMissingForRegion(x: number, y: number, width: number, height: number): number {
    let missing = 0;
    for (let mip = 0; mip <= this.maxMip; mip++) {
      const scale = 2 ** mip;
      const border = PAGE_BORDER * scale;
      const firstX = Math.max(0, Math.floor((x - border) / (PAGE_SIZE * scale)));
      const firstY = Math.max(0, Math.floor((y - border) / (PAGE_SIZE * scale)));
      const lastX = Math.min(this.pagesX(mip) - 1, Math.floor((x + width - 1 + border) / (PAGE_SIZE * scale)));
      const lastY = Math.min(this.pagesY(mip) - 1, Math.floor((y + height - 1 + border) / (PAGE_SIZE * scale)));
      for (let pageY = firstY; pageY <= lastY; pageY++)
        for (let pageX = firstX; pageX <= lastX; pageX++)
          if (this.slot(mip, pageX, pageY) === undefined) missing++;
    }
    return missing;
  }

  private countNewDirtyForRegion(x: number, y: number, width: number, height: number): number {
    let missing = 0;
    for (let mip = 0; mip <= this.maxMip; mip++) {
      const scale = 2 ** mip;
      const border = PAGE_BORDER * scale;
      const firstX = Math.max(0, Math.floor((x - border) / (PAGE_SIZE * scale)));
      const firstY = Math.max(0, Math.floor((y - border) / (PAGE_SIZE * scale)));
      const lastX = Math.min(this.pagesX(mip) - 1, Math.floor((x + width - 1 + border) / (PAGE_SIZE * scale)));
      const lastY = Math.min(this.pagesY(mip) - 1, Math.floor((y + height - 1 + border) / (PAGE_SIZE * scale)));
      for (let pageY = firstY; pageY <= lastY; pageY++)
        for (let pageX = firstX; pageX <= lastX; pageX++)
          if (this.dirtyKeys.get(pageKey(mip, pageX, pageY)) === undefined) missing++;
    }
    return missing;
  }

  private markRegion(x: number, y: number, width: number, height: number): void {
    for (let mip = 0; mip <= this.maxMip; mip++) {
      const scale = 2 ** mip;
      const border = PAGE_BORDER * scale;
      const firstX = Math.max(0, Math.floor((x - border) / (PAGE_SIZE * scale)));
      const firstY = Math.max(0, Math.floor((y - border) / (PAGE_SIZE * scale)));
      const lastX = Math.min(this.pagesX(mip) - 1, Math.floor((x + width - 1 + border) / (PAGE_SIZE * scale)));
      const lastY = Math.min(this.pagesY(mip) - 1, Math.floor((y + height - 1 + border) / (PAGE_SIZE * scale)));
      for (let pageY = firstY; pageY <= lastY; pageY++) {
        for (let pageX = firstX; pageX <= lastX; pageX++) {
          const slot = this.acquirePage(mip, pageX, pageY)!;
          if (mip > 0) this.slotDirty[slot] = 1;
          this.slotRevision[slot] = this.revision;
          const key = pageKey(mip, pageX, pageY);
          const existingDirty = this.dirtyKeys.get(key);
          if (existingDirty !== undefined) {
            this.dirtyRevision[existingDirty] = this.revision;
            continue;
          }
          const dirtySlot = this.dirtyTail;
          this.dirtyMip[dirtySlot] = mip;
          this.dirtyX[dirtySlot] = pageX;
          this.dirtyY[dirtySlot] = pageY;
          this.dirtyRevision[dirtySlot] = this.revision;
          this.dirtyKeys.set(key, dirtySlot);
          this.dirtyTail = (dirtySlot + 1) % this.dirtyMip.length;
          this.dirtyCount++;
        }
      }
    }
  }

  /** Write canonical mip-zero texels and enqueue every affected bordered mip page. */
  writeRegion(
    x: number,
    y: number,
    width: number,
    height: number,
    source: Uint8Array,
    bytesPerRow = width * this.bytesPerTexel,
  ): MemoryTextureWriteStatus {
    if (!Number.isInteger(x) || !Number.isInteger(y) || !Number.isInteger(width) ||
        !Number.isInteger(height) || width < 1 || height < 1 || x < 0 || y < 0 ||
        x + width > this.options.width || y + height > this.options.height ||
        bytesPerRow < width * this.bytesPerTexel ||
        source.byteLength < bytesPerRow * (height - 1) + width * this.bytesPerTexel)
      return MemoryTextureWriteStatus.InvalidRegion;
    const missingPages = this.countMissingForRegion(x, y, width, height);
    if (missingPages > this.freeTop) return MemoryTextureWriteStatus.PageCapacityExceeded;
    const newDirty = this.countNewDirtyForRegion(x, y, width, height);
    if (newDirty > this.dirtyMip.length - this.dirtyCount)
      return MemoryTextureWriteStatus.DirtyCapacityExceeded;
    this.revision = (this.revision + 1) >>> 0 || 1;
    this.markRegion(x, y, width, height);
    for (let row = 0; row < height; row++) {
      for (let column = 0; column < width; column++) {
        const texelX = x + column, texelY = y + row;
        const pageX = Math.floor(texelX / PAGE_SIZE), pageY = Math.floor(texelY / PAGE_SIZE);
        const slot = this.slot(0, pageX, pageY)!;
        const local = (texelY % PAGE_SIZE) * PAGE_SIZE + (texelX % PAGE_SIZE);
        const target = slot * this.pageBytes + local * this.bytesPerTexel;
        const input = row * bytesPerRow + column * this.bytesPerTexel;
        for (let byte = 0; byte < this.bytesPerTexel; byte++)
          this.storage[target + byte] = source[input + byte] ?? 0;
      }
    }
    return MemoryTextureWriteStatus.Written;
  }

  private address(value: number, size: number): number {
    if (this.options.addressMode === 'clamp') return Math.max(0, Math.min(size - 1, value));
    if (this.options.addressMode === 'repeat') return ((value % size) + size) % size;
    const period = size * 2;
    const wrapped = ((value % period) + period) % period;
    return wrapped < size ? wrapped : period - 1 - wrapped;
  }

  private decode(storageOffset: number, output: Float32Array): void {
    if (this.options.format === 'rgba8unorm') {
      for (let component = 0; component < 4; component++) output[component] = (this.storage[storageOffset + component] ?? 0) / 255;
    } else if (this.options.format === 'r8unorm') {
      output[0] = (this.storage[storageOffset] ?? 0) / 255;
    } else {
      output[0] = halfToFloat((this.storage[storageOffset] ?? 0) | ((this.storage[storageOffset + 1] ?? 0) << 8));
    }
  }

  private encode(storageOffset: number, input: Float32Array): void {
    if (this.options.format === 'rgba8unorm') {
      for (let component = 0; component < 4; component++)
        this.storage[storageOffset + component] = Math.max(0, Math.min(255, Math.round((input[component] ?? 0) * 255)));
    } else if (this.options.format === 'r8unorm') {
      this.storage[storageOffset] = Math.max(0, Math.min(255, Math.round((input[0] ?? 0) * 255)));
    } else {
      const half = floatToHalf(input[0] ?? 0);
      this.storage[storageOffset] = half & 0xff;
      this.storage[storageOffset + 1] = half >>> 8;
    }
  }

  private ensureDerived(mip: number, x: number, y: number): number | undefined {
    const slot = this.slot(mip, x, y);
    if (slot === undefined || mip === 0 || this.slotDirty[slot] === 0) return slot;
    const childMip = mip - 1;
    for (let childY = y * 2; childY <= y * 2 + 1; childY++)
      for (let childX = x * 2; childX <= x * 2 + 1; childX++)
        this.ensureDerived(childMip, childX, childY);
    const start = slot * this.pageBytes;
    for (let localY = 0; localY < PAGE_SIZE; localY++) {
      for (let localX = 0; localX < PAGE_SIZE; localX++) {
        const sums = this.scratchTexel;
        sums.fill(0);
        for (let oy = 0; oy < 2; oy++) {
          for (let ox = 0; ox < 2; ox++) {
            this.readTexel(childMip, (x * PAGE_SIZE + localX) * 2 + ox,
              (y * PAGE_SIZE + localY) * 2 + oy, this.scratchSample);
            for (let component = 0; component < this.bytesPerTexel; component++) {
              const sample = this.scratchSample[component] ?? 0;
              sums[component] = (sums[component] ?? 0) +
                (this.options.mipFilter === 'linear-color' &&
                  this.options.format === 'rgba8unorm' && component < 3
                  ? srgbToLinear(sample)
                  : sample);
            }
          }
        }
        for (let component = 0; component < 4; component++)
          sums[component] = (sums[component] ?? 0) / 4;
        if (this.options.mipFilter === 'linear-color' && this.options.format === 'rgba8unorm') {
          sums[0] = linearToSrgb(sums[0] ?? 0);
          sums[1] = linearToSrgb(sums[1] ?? 0);
          sums[2] = linearToSrgb(sums[2] ?? 0);
        }
        if (this.options.mipFilter === 'normal' && this.options.format === 'rgba8unorm') {
          let nx = (sums[0] ?? 0) * 2 - 1;
          let ny = (sums[1] ?? 0) * 2 - 1;
          let nz = (sums[2] ?? 0) * 2 - 1;
          const length = Math.hypot(nx, ny, nz) || 1;
          nx /= length; ny /= length; nz /= length;
          sums[0] = nx * 0.5 + 0.5; sums[1] = ny * 0.5 + 0.5; sums[2] = nz * 0.5 + 0.5;
        }
        this.encode(start + (localY * PAGE_SIZE + localX) * this.bytesPerTexel, sums);
      }
    }
    this.slotDirty[slot] = 0;
    return slot;
  }

  private readTexel(mip: number, x: number, y: number, output: Float32Array): void {
    const width = Math.max(1, Math.ceil(this.options.width / 2 ** mip));
    const height = Math.max(1, Math.ceil(this.options.height / 2 ** mip));
    const addressedX = this.address(x, width), addressedY = this.address(y, height);
    const pageX = Math.floor(addressedX / PAGE_SIZE), pageY = Math.floor(addressedY / PAGE_SIZE);
    const slot = this.ensureDerived(mip, pageX, pageY);
    output.fill(0);
    if (slot === undefined) {
      if (this.options.format === 'rgba8unorm')
        for (let component = 0; component < 4; component++) output[component] = (this.defaultTexel[component] ?? 0) / 255;
      else if (this.options.format === 'r8unorm') output[0] = (this.defaultTexel[0] ?? 0) / 255;
      else output[0] = halfToFloat((this.defaultTexel[0] ?? 0) | ((this.defaultTexel[1] ?? 0) << 8));
      return;
    }
    const local = (addressedY % PAGE_SIZE) * PAGE_SIZE + (addressedX % PAGE_SIZE);
    this.decode(slot * this.pageBytes + local * this.bytesPerTexel, output);
  }

  /** Fill one caller-owned bordered page without allocating a payload. */
  readPageInto(request: Readonly<PageRequest>, output: Uint8Array): void {
    if (request.tail) throw new Error('memory virtual textures use ordinary terminal pages, not packed tails');
    if (request.mip < 0 || request.mip > this.maxMip || request.x < 0 || request.y < 0 ||
        request.x >= this.pagesX(request.mip) || request.y >= this.pagesY(request.mip))
      throw new RangeError('memory virtual-texture page is out of range');
    const required = SLOT_SIZE * SLOT_SIZE * this.bytesPerTexel;
    if (output.byteLength !== required)
      throw new RangeError(`memory virtual-texture output has ${output.byteLength} bytes; expected ${required}`);
    this.fillPageInto(request, output);
  }

  // @hot-no-alloc-begin MemoryVirtualTextureSource.fillPageInto
  private fillPageInto(request: Readonly<PageRequest>, output: Uint8Array): void {
    this.ensureDerived(request.mip, request.x, request.y);
    for (let slotY = 0; slotY < SLOT_SIZE; slotY++) {
      for (let slotX = 0; slotX < SLOT_SIZE; slotX++) {
        this.readTexel(
          request.mip,
          request.x * PAGE_SIZE + slotX - PAGE_BORDER,
          request.y * PAGE_SIZE + slotY - PAGE_BORDER,
          this.scratchSample,
        );
        const target = (slotY * SLOT_SIZE + slotX) * this.bytesPerTexel;
        if (this.options.format === 'rgba8unorm') {
          for (let component = 0; component < 4; component++)
            output[target + component] = Math.round((this.scratchSample[component] ?? 0) * 255);
        } else if (this.options.format === 'r8unorm') {
          output[target] = Math.round((this.scratchSample[0] ?? 0) * 255);
        } else {
          const half = floatToHalf(this.scratchSample[0] ?? 0);
          output[target] = half & 0xff;
          output[target + 1] = half >>> 8;
        }
      }
    }
  }
  // @hot-no-alloc-end MemoryVirtualTextureSource.fillPageInto

  /** Allocating diagnostic/game-facing convenience wrapper. */
  readPage(request: Readonly<PageRequest>): Uint8Array {
    const output = new Uint8Array(SLOT_SIZE * SLOT_SIZE * this.bytesPerTexel);
    this.readPageInto(request, output);
    return output;
  }

  /** Generate and publish a bounded prefix of changed pages. */
  drainDirty(
    limit: number,
    publish: (page: Readonly<MemoryTextureDirtyPage>, bytes: Uint8Array) => boolean,
  ): number {
    let drained = 0;
    while (drained < limit && this.dirtyCount !== 0) {
      const slot = this.dirtyHead;
      const mip = this.dirtyMip[slot] ?? 0;
      const x = this.dirtyX[slot] ?? 0;
      const y = this.dirtyY[slot] ?? 0;
      const revision = this.dirtyRevision[slot] ?? 0;
      this.dirtyPage.mip = mip;
      this.dirtyPage.x = x;
      this.dirtyPage.y = y;
      this.dirtyPage.revision = revision;
      this.fillPageInto(this.dirtyPage, this.refreshScratch);
      if (!publish(this.dirtyPage, this.refreshScratch)) break;
      this.dirtyKeys.delete(pageKey(mip, x, y));
      this.dirtyHead = (slot + 1) % this.dirtyMip.length;
      this.dirtyCount--;
      drained++;
    }
    return drained;
  }

  get canonicalPageCount(): number {
    let count = 0;
    for (let slot = 0; slot < this.options.pageCapacity; slot++)
      if (this.slotsByPage.get(pageKey(0, this.slotX[slot] ?? 0, this.slotY[slot] ?? 0)) === slot)
        count++;
    return count;
  }

  /** Persistence-boundary enumeration; storage is borrowed only for the callback. */
  visitCanonicalPages(visitor: MemoryCanonicalPageVisitor): void {
    for (let slot = 0; slot < this.options.pageCapacity; slot++) {
      if (this.slotsByPage.get(pageKey(0, this.slotX[slot] ?? 0, this.slotY[slot] ?? 0)) !== slot)
        continue;
      visitor(
        this.slotX[slot] ?? 0,
        this.slotY[slot] ?? 0,
        this.storage,
        slot * this.pageBytes,
        this.pageBytes,
      );
    }
  }

  /** Restore the externally persisted logical revision after atomic decode. */
  restoreContentRevision(revision: number): void {
    if (!Number.isInteger(revision) || revision < 0 || revision > 0xffff_ffff)
      throw new RangeError('invalid memory texture revision');
    this.revision = revision >>> 0;
    for (let slot = 0; slot < this.options.pageCapacity; slot++)
      this.slotRevision[slot] = this.revision;
  }

  get pageCount(): number { return this.options.pageCapacity - this.freeTop; }
  get pendingDirtyPages(): number { return this.dirtyCount; }
  get contentRevision(): number { return this.revision; }
  get activeOutputPages(): number { return this.outputPool.active; }
  get outputPageHighWater(): number { return this.outputPool.highWater; }
  get outputPageOverflows(): number { return this.outputPool.overflows; }
}
