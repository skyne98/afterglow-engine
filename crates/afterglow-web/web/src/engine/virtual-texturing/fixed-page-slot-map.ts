/** Fixed open-addressed numeric page-key map. Never resizes after bootstrap. */
export class FixedPageSlotMap {
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
