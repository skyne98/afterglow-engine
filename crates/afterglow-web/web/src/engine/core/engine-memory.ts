import { Resource, defineResource } from './resource.ts';

export const INVALID_MEMORY_OFFSET = -1;
export const INVALID_POOL_INDEX = 0xffffffff;

export enum RingPushStatus {
  Accepted = 0,
  CapacityExceeded = 1,
}

export enum StructuralCommandKind {
  Spawn = 1,
  Despawn = 2,
  Reparent = 3,
  SetRenderRef = 4,
}

export interface StructuralCommandSink {
  applyStructuralCommand(kind: number, entity: number, argument0: number, argument1: number): void;
}

export enum EnginePhase {
  Bootstrap,
  Warmup,
  GameplaySealed,
  Shutdown,
}

export interface EngineMemoryConfig {
  frameScratchBytes: number;
  renderScratchBytes: number;
  structuralCommands: number;
  workerCompletions: number;
  assetRequests: number;
  vtRequests: number;
  /** Fixed 40-byte trace records reserved for the page telemetry producer. */
  telemetryRecords: number;
  /** Fixed Float64 metric cells reserved for page telemetry descriptors. */
  telemetryMetricCells: number;
}

export interface EngineMemoryMetrics {
  frameArenaOverflows: number;
  renderArenaOverflows: number;
  poolOverflows: number;
  sealedAllocationViolations: number;
}

/** Fixed linear scratch. allocate() returns an offset and never grows. */
export class LinearArena {
  readonly buffer: ArrayBuffer;
  used = 0;
  highWater = 0;
  overflows = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 0) throw new RangeError('arena capacity must be a non-negative integer');
    this.buffer = new ArrayBuffer(capacity);
  }

  // @hot-no-alloc-begin LinearArena.allocate
  allocate(size: number, alignment = 1): number {
    const aligned = Math.ceil(this.used / alignment) * alignment;
    if (size < 0 || aligned + size > this.capacity) {
      this.overflows++;
      return INVALID_MEMORY_OFFSET;
    }
    this.used = aligned + size;
    if (this.used > this.highWater) this.highWater = this.used;
    return aligned;
  }
  // @hot-no-alloc-end LinearArena.allocate

  // @hot-no-alloc-begin LinearArena.reset
  reset(): void { this.used = 0; }
  // @hot-no-alloc-end LinearArena.reset
}

/** O(1) fixed-capacity index pool backed by one preallocated free stack. */
export class FixedIndexPool {
  private readonly free: Uint32Array;
  private top: number;
  used = 0;
  highWater = 0;
  overflows = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 0) throw new RangeError('pool capacity must be a non-negative integer');
    this.free = new Uint32Array(capacity);
    for (let index = 0; index < capacity; index++) this.free[index] = capacity - index - 1;
    this.top = capacity;
  }

  // @hot-no-alloc-begin FixedIndexPool.acquire
  acquire(): number {
    if (this.top === 0) {
      this.overflows++;
      return INVALID_POOL_INDEX;
    }
    const index = this.free[--this.top];
    this.used++;
    if (this.used > this.highWater) this.highWater = this.used;
    return index;
  }
  // @hot-no-alloc-end FixedIndexPool.acquire

  // @hot-no-alloc-begin FixedIndexPool.release
  release(index: number): boolean {
    if (!Number.isInteger(index) || index < 0 || index >= this.capacity || this.top >= this.capacity) return false;
    this.free[this.top++] = index;
    this.used--;
    return true;
  }
  // @hot-no-alloc-end FixedIndexPool.release
}

/** Fixed structural command ring with typed overflow and bounded draining. */
export class FixedStructuralCommandRing {
  private readonly kinds: Uint8Array;
  private readonly entities: Uint32Array;
  private readonly argument0: Float64Array;
  private readonly argument1: Float64Array;
  private head = 0;
  private tail = 0;
  count = 0;
  highWater = 0;
  overflows = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError('structural command capacity must be positive');
    this.kinds = new Uint8Array(capacity);
    this.entities = new Uint32Array(capacity);
    this.argument0 = new Float64Array(capacity);
    this.argument1 = new Float64Array(capacity);
  }

  // @hot-no-alloc-begin FixedStructuralCommandRing.tryPush
  tryPush(kind: StructuralCommandKind, entity: number, argument0 = 0, argument1 = 0): RingPushStatus {
    if (this.count === this.capacity) {
      this.overflows++;
      return RingPushStatus.CapacityExceeded;
    }
    const slot = this.tail;
    this.kinds[slot] = kind;
    this.entities[slot] = entity;
    this.argument0[slot] = argument0;
    this.argument1[slot] = argument1;
    this.tail = (slot + 1) % this.capacity;
    this.count++;
    if (this.count > this.highWater) this.highWater = this.count;
    return RingPushStatus.Accepted;
  }
  // @hot-no-alloc-end FixedStructuralCommandRing.tryPush

  // @hot-no-alloc-begin FixedStructuralCommandRing.drain
  drain(maxOperations: number, sink: StructuralCommandSink): number {
    let drained = 0;
    while (drained < maxOperations && this.count !== 0) {
      const slot = this.head;
      sink.applyStructuralCommand(
        this.kinds[slot], this.entities[slot], this.argument0[slot], this.argument1[slot],
      );
      this.head = (slot + 1) % this.capacity;
      this.count--;
      drained++;
    }
    return drained;
  }
  // @hot-no-alloc-end FixedStructuralCommandRing.drain
}

/**
 * The single page-side owner of engine scratch and fixed-capacity record pools.
 * Constructors allocate only during bootstrap. Gameplay methods only mutate
 * preallocated storage and numeric counters.
 */
export class EngineMemory {
  phase = EnginePhase.Bootstrap;
  readonly frame: LinearArena;
  readonly render: LinearArena;
  readonly structuralCommands: FixedStructuralCommandRing;
  readonly workerCompletions: FixedIndexPool;
  readonly assetRequests: FixedIndexPool;
  readonly vtRequests: FixedIndexPool;
  readonly telemetryTrace: ArrayBuffer;
  readonly telemetryMetrics: Float64Array;
  readonly metrics: EngineMemoryMetrics = {
    frameArenaOverflows: 0,
    renderArenaOverflows: 0,
    poolOverflows: 0,
    sealedAllocationViolations: 0,
  };

  constructor(readonly config: Readonly<EngineMemoryConfig>) {
    this.frame = new LinearArena(config.frameScratchBytes);
    this.render = new LinearArena(config.renderScratchBytes);
    this.structuralCommands = new FixedStructuralCommandRing(config.structuralCommands);
    this.workerCompletions = new FixedIndexPool(config.workerCompletions);
    this.assetRequests = new FixedIndexPool(config.assetRequests);
    this.vtRequests = new FixedIndexPool(config.vtRequests);
    if (!Number.isInteger(config.telemetryRecords) || config.telemetryRecords <= 0)
      throw new RangeError('telemetryRecords must be a positive integer');
    if (!Number.isInteger(config.telemetryMetricCells) || config.telemetryMetricCells <= 0)
      throw new RangeError('telemetryMetricCells must be a positive integer');
    this.telemetryTrace = new ArrayBuffer(config.telemetryRecords * 40);
    this.telemetryMetrics = new Float64Array(config.telemetryMetricCells);
  }

  warmup(): void {
    if (this.phase !== EnginePhase.Bootstrap) throw new Error('EngineMemory can enter warmup only from bootstrap');
    this.phase = EnginePhase.Warmup;
  }

  sealGameplay(): void {
    if (this.phase !== EnginePhase.Warmup)
      throw new Error('EngineMemory can seal only after warmup');
    this.phase = EnginePhase.GameplaySealed;
  }

  // @hot-no-alloc-begin EngineMemory.beginFrame
  beginFrame(): void {
    this.frame.reset();
    this.render.reset();
  }
  // @hot-no-alloc-end EngineMemory.beginFrame

  refreshMetrics(): EngineMemoryMetrics {
    this.metrics.frameArenaOverflows = this.frame.overflows;
    this.metrics.renderArenaOverflows = this.render.overflows;
    this.metrics.poolOverflows = this.structuralCommands.overflows + this.workerCompletions.overflows
      + this.assetRequests.overflows + this.vtRequests.overflows;
    return this.metrics;
  }
}

export function defineEngineMemoryResource(config: EngineMemoryConfig): Resource<EngineMemory> {
  return defineResource('engineMemory', () => new EngineMemory(config));
}
