const SLOT_BITS = 20;
const SLOT_SCALE = 2 ** SLOT_BITS;
const SLOT_MASK = SLOT_SCALE - 1;

/** Numeric generational handle shared by streamed texture and model systems. */
export type StreamResourceHandle = number & { readonly __streamResourceHandle: unique symbol };

/** Fixed-capacity ownership table. Zero is always an invalid handle. */
export class FixedResourceRegistry<T> {
  private readonly values: (T | null)[];
  private readonly generations: Uint32Array;
  private readonly free: Uint32Array;
  private freeTop = 0;
  private count = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 1 || capacity > SLOT_MASK)
      throw new RangeError(`stream resource capacity must be from 1 through ${SLOT_MASK}`);
    this.values = new Array<T | null>(capacity).fill(null);
    this.generations = new Uint32Array(capacity);
    this.free = new Uint32Array(capacity);
    for (let slot = capacity - 1; slot >= 0; slot--) this.free[this.freeTop++] = slot;
  }

  /** @alloc-effect none */
  acquire(value: T): StreamResourceHandle | 0 {
    if (this.freeTop === 0) return 0;
    const slot = this.free[--this.freeTop] ?? 0;
    let generation = ((this.generations[slot] ?? 0) + 1) >>> 0;
    if (generation === 0) generation = 1;
    this.generations[slot] = generation;
    this.values[slot] = value;
    this.count++;
    return (generation * SLOT_SCALE + slot + 1) as StreamResourceHandle;
  }

  /** @alloc-effect none */
  get(handle: StreamResourceHandle): T | null {
    const numeric = handle as number;
    if (!Number.isSafeInteger(numeric) || numeric <= 0) return null;
    const encodedSlot = numeric % SLOT_SCALE;
    if (encodedSlot === 0) return null;
    const slot = encodedSlot - 1;
    const generation = Math.floor(numeric / SLOT_SCALE) >>> 0;
    if (slot >= this.capacity || this.generations[slot] !== generation) return null;
    return this.values[slot] ?? null;
  }

  /** @alloc-effect none */
  release(handle: StreamResourceHandle): T | null {
    const numeric = handle as number;
    if (!Number.isSafeInteger(numeric) || numeric <= 0) return null;
    const encodedSlot = numeric % SLOT_SCALE;
    if (encodedSlot === 0) return null;
    const slot = encodedSlot - 1;
    const generation = Math.floor(numeric / SLOT_SCALE) >>> 0;
    if (slot >= this.capacity || this.generations[slot] !== generation) return null;
    const value = this.values[slot] ?? null;
    if (value === null) return null;
    this.values[slot] = null;
    let nextGeneration = (generation + 1) >>> 0;
    if (nextGeneration === 0) nextGeneration = 1;
    this.generations[slot] = nextGeneration;
    this.free[this.freeTop++] = slot;
    this.count--;
    return value;
  }

  /** Allocation-free bounded structural iteration support. */
  valueAt(slot: number): T | null {
    return Number.isInteger(slot) && slot >= 0 && slot < this.capacity
      ? this.values[slot] ?? null
      : null;
  }

  get size(): number { return this.count; }
  get freeCount(): number { return this.freeTop; }
}
