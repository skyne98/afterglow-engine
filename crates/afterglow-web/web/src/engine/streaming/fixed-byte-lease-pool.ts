export interface FixedByteLease {
  readonly bytes: Uint8Array;
  readonly slot: number;
  release(): boolean;
}

class ByteLease implements FixedByteLease {
  inUse = false;

  constructor(
    readonly bytes: Uint8Array,
    readonly slot: number,
    private readonly owner: FixedByteLeasePool,
  ) {}

  release(): boolean {
    return this.owner.release(this);
  }
}

/** Fixed reusable byte buffers with O(1) acquire/release and stable telemetry. */
export class FixedByteLeasePool {
  private readonly leases: ByteLease[];
  private readonly free: Uint32Array;
  private freeTop: number;
  private activeCount = 0;
  private highWaterCount = 0;
  private overflowCount = 0;

  constructor(readonly capacity: number, readonly bytesPerLease: number) {
    if (!Number.isInteger(capacity) || capacity < 1 ||
        !Number.isInteger(bytesPerLease) || bytesPerLease < 1)
      throw new RangeError('invalid fixed byte-lease pool capacity');
    this.leases = new Array(capacity);
    this.free = new Uint32Array(capacity);
    this.freeTop = capacity;
    for (let slot = 0; slot < capacity; slot++) {
      this.leases[slot] = new ByteLease(new Uint8Array(bytesPerLease), slot, this);
      this.free[slot] = capacity - slot - 1;
    }
  }

  tryAcquire(): FixedByteLease | null {
    if (this.freeTop === 0) {
      this.overflowCount++;
      return null;
    }
    const slot = this.free[--this.freeTop] ?? 0;
    const lease = this.leases[slot]!;
    lease.inUse = true;
    this.activeCount++;
    if (this.activeCount > this.highWaterCount) this.highWaterCount = this.activeCount;
    return lease;
  }

  release(lease: ByteLease): boolean {
    if (!lease.inUse || this.leases[lease.slot] !== lease) return false;
    lease.inUse = false;
    this.free[this.freeTop++] = lease.slot;
    this.activeCount--;
    return true;
  }

  get active(): number { return this.activeCount; }
  get highWater(): number { return this.highWaterCount; }
  get overflows(): number { return this.overflowCount; }
}
