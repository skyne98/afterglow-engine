// crates/afterglow-web/www/engine/resource.ts
var RESOURCES = Symbol.for("afterglow-resources");
var RESOURCES_SEALED = Symbol.for("afterglow-resources-sealed");
function ensureStore(world) {
  const w = world;
  if (!w[RESOURCES])
    w[RESOURCES] = {};
  return w[RESOURCES];
}

class Resource {
  name;
  factory;
  constructor(name, factory) {
    this.name = name;
    this.factory = factory;
  }
  get(world) {
    const store = ensureStore(world);
    if (!(this.name in store)) {
      if (world[RESOURCES_SEALED] === true)
        throw new Error(`resource ${this.name} was not initialized before gameplay seal`);
      store[this.name] = this.factory();
    }
    return store[this.name];
  }
  set(world, value) {
    ensureStore(world)[this.name] = value;
  }
  has(world) {
    return this.name in ensureStore(world);
  }
  remove(world) {
    delete ensureStore(world)[this.name];
  }
}
function defineResource(name, factory) {
  return new Resource(name, factory);
}

// crates/afterglow-web/www/engine/engine-memory.ts
var INVALID_MEMORY_OFFSET = -1;
var INVALID_POOL_INDEX = 4294967295;
var RingPushStatus;
((RingPushStatus2) => {
  RingPushStatus2[RingPushStatus2["Accepted"] = 0] = "Accepted";
  RingPushStatus2[RingPushStatus2["CapacityExceeded"] = 1] = "CapacityExceeded";
})(RingPushStatus ||= {});
var StructuralCommandKind;
((StructuralCommandKind2) => {
  StructuralCommandKind2[StructuralCommandKind2["Spawn"] = 1] = "Spawn";
  StructuralCommandKind2[StructuralCommandKind2["Despawn"] = 2] = "Despawn";
  StructuralCommandKind2[StructuralCommandKind2["Reparent"] = 3] = "Reparent";
  StructuralCommandKind2[StructuralCommandKind2["SetRenderRef"] = 4] = "SetRenderRef";
})(StructuralCommandKind ||= {});
var EnginePhase;
((EnginePhase2) => {
  EnginePhase2[EnginePhase2["Bootstrap"] = 0] = "Bootstrap";
  EnginePhase2[EnginePhase2["Warmup"] = 1] = "Warmup";
  EnginePhase2[EnginePhase2["GameplaySealed"] = 2] = "GameplaySealed";
  EnginePhase2[EnginePhase2["LoadingScreen"] = 3] = "LoadingScreen";
  EnginePhase2[EnginePhase2["Shutdown"] = 4] = "Shutdown";
})(EnginePhase ||= {});

class LinearArena {
  capacity;
  buffer;
  used = 0;
  highWater = 0;
  overflows = 0;
  constructor(capacity) {
    this.capacity = capacity;
    if (!Number.isInteger(capacity) || capacity < 0)
      throw new RangeError("arena capacity must be a non-negative integer");
    this.buffer = new ArrayBuffer(capacity);
  }
  allocate(size, alignment = 1) {
    const aligned = Math.ceil(this.used / alignment) * alignment;
    if (size < 0 || aligned + size > this.capacity) {
      this.overflows++;
      return INVALID_MEMORY_OFFSET;
    }
    this.used = aligned + size;
    if (this.used > this.highWater)
      this.highWater = this.used;
    return aligned;
  }
  reset() {
    this.used = 0;
  }
}

class FixedIndexPool {
  capacity;
  free;
  top;
  used = 0;
  highWater = 0;
  overflows = 0;
  constructor(capacity) {
    this.capacity = capacity;
    if (!Number.isInteger(capacity) || capacity < 0)
      throw new RangeError("pool capacity must be a non-negative integer");
    this.free = new Uint32Array(capacity);
    for (let index = 0;index < capacity; index++)
      this.free[index] = capacity - index - 1;
    this.top = capacity;
  }
  acquire() {
    if (this.top === 0) {
      this.overflows++;
      return INVALID_POOL_INDEX;
    }
    const index = this.free[--this.top];
    this.used++;
    if (this.used > this.highWater)
      this.highWater = this.used;
    return index;
  }
  release(index) {
    if (!Number.isInteger(index) || index < 0 || index >= this.capacity || this.top >= this.capacity)
      return false;
    this.free[this.top++] = index;
    this.used--;
    return true;
  }
}

class FixedStructuralCommandRing {
  capacity;
  kinds;
  entities;
  argument0;
  argument1;
  head = 0;
  tail = 0;
  count = 0;
  highWater = 0;
  overflows = 0;
  constructor(capacity) {
    this.capacity = capacity;
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError("structural command capacity must be positive");
    this.kinds = new Uint8Array(capacity);
    this.entities = new Uint32Array(capacity);
    this.argument0 = new Float64Array(capacity);
    this.argument1 = new Float64Array(capacity);
  }
  tryPush(kind, entity, argument0 = 0, argument1 = 0) {
    if (this.count === this.capacity) {
      this.overflows++;
      return 1 /* CapacityExceeded */;
    }
    const slot = this.tail;
    this.kinds[slot] = kind;
    this.entities[slot] = entity;
    this.argument0[slot] = argument0;
    this.argument1[slot] = argument1;
    this.tail = (slot + 1) % this.capacity;
    this.count++;
    if (this.count > this.highWater)
      this.highWater = this.count;
    return 0 /* Accepted */;
  }
  drain(maxOperations, sink) {
    let drained = 0;
    while (drained < maxOperations && this.count !== 0) {
      const slot = this.head;
      sink.applyStructuralCommand(this.kinds[slot], this.entities[slot], this.argument0[slot], this.argument1[slot]);
      this.head = (slot + 1) % this.capacity;
      this.count--;
      drained++;
    }
    return drained;
  }
}

class EngineMemory {
  config;
  phase = 0 /* Bootstrap */;
  frame;
  render;
  structuralCommands;
  workerCompletions;
  assetRequests;
  vtRequests;
  metrics = {
    frameArenaOverflows: 0,
    renderArenaOverflows: 0,
    poolOverflows: 0,
    sealedAllocationViolations: 0
  };
  constructor(config) {
    this.config = config;
    this.frame = new LinearArena(config.frameScratchBytes);
    this.render = new LinearArena(config.renderScratchBytes);
    this.structuralCommands = new FixedStructuralCommandRing(config.structuralCommands);
    this.workerCompletions = new FixedIndexPool(config.workerCompletions);
    this.assetRequests = new FixedIndexPool(config.assetRequests);
    this.vtRequests = new FixedIndexPool(config.vtRequests);
  }
  warmup() {
    if (this.phase !== 0 /* Bootstrap */)
      throw new Error("EngineMemory can enter warmup only from bootstrap");
    this.phase = 1 /* Warmup */;
  }
  sealGameplay() {
    if (this.phase !== 1 /* Warmup */ && this.phase !== 3 /* LoadingScreen */)
      throw new Error("EngineMemory can seal only after warmup/loading");
    this.phase = 2 /* GameplaySealed */;
  }
  beginFrame() {
    this.frame.reset();
    this.render.reset();
  }
  refreshMetrics() {
    this.metrics.frameArenaOverflows = this.frame.overflows;
    this.metrics.renderArenaOverflows = this.render.overflows;
    this.metrics.poolOverflows = this.structuralCommands.overflows + this.workerCompletions.overflows + this.assetRequests.overflows + this.vtRequests.overflows;
    return this.metrics;
  }
}
function defineEngineMemoryResource(config) {
  return defineResource("engineMemory", () => new EngineMemory(config));
}
export {
  defineEngineMemoryResource,
  StructuralCommandKind,
  RingPushStatus,
  LinearArena,
  INVALID_POOL_INDEX,
  INVALID_MEMORY_OFFSET,
  FixedStructuralCommandRing,
  FixedIndexPool,
  EnginePhase,
  EngineMemory
};
