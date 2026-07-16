// ECS Resources — singleton objects with state, stored on the world.
//
// Unlike components (per-entity, SoA TypedArrays), resources are global — one
// instance per world. They hold engine-wide singletons: the asset store, the
// render adapter, the input state, the physics world, etc.
//
// bitECS doesn't have a built-in resource API, but `createWorld` accepts any
// object as context. We store resources in a hidden `Symbol.for` property on
// the world, lazily initialized on first access.
//
// Usage:
//   import { defineResource } from './resource.ts';
//
//   const AssetStoreRes = defineResource<AssetStore>('assetStore', () => new AssetStore(...));
//
//   // In a system or the render loop:
//   const store = AssetStoreRes.get(world);   // lazily creates on first call
//   const tex = await store.loadTexture('sky.png');
//
//   // Or set explicitly (e.g. after async spawn):
//   AssetStoreRes.set(world, store);

/** Symbol under which resources are stored on the bitECS world. */
const RESOURCES = Symbol.for('afterglow-resources');
const RESOURCES_SEALED = Symbol.for('afterglow-resources-sealed');

/** Internal storage on the world: a plain object keyed by resource name. */
type ResourceStore = Record<string, unknown>;

/** Ensure the world has a resource storage object. */
function ensureStore(world: object): ResourceStore {
  const w = world as Record<symbol, unknown>;
  if (!w[RESOURCES]) w[RESOURCES] = {} as ResourceStore;
  return w[RESOURCES] as ResourceStore;
}

/**
 * A typed ECS resource — a singleton with state, lazily created on first
 * access via a factory function.
 *
 * @typeParam T — the resource type (e.g. `AssetStore`, `InputState`).
 */
export class Resource<T> {
  constructor(
    /** Unique key for this resource on the world. */
    readonly name: string,
    /** Called once to create the resource if it doesn't exist. */
    private readonly factory: () => T,
  ) {}

  /** Get the resource from the world, creating it on first access. */
  get(world: object): T {
    const store = ensureStore(world);
    if (!(this.name in store)) {
      if ((world as Record<symbol, unknown>)[RESOURCES_SEALED] === true)
        throw new Error(`resource ${this.name} was not initialized before gameplay seal`);
      store[this.name] = this.factory();
    }
    return store[this.name] as T;
  }

  /** Set the resource explicitly (overwrites any existing instance). */
  set(world: object, value: T): void {
    ensureStore(world)[this.name] = value;
  }

  /** Does the world have this resource yet? */
  has(world: object): boolean {
    return this.name in ensureStore(world);
  }

  /** Remove the resource from the world (does not dispose it). */
  remove(world: object): void {
    delete ensureStore(world)[this.name];
  }
}

/**
 * Define a new ECS resource with a factory function.
 * The resource is created lazily on first `get()`.
 */
export function defineResource<T>(
  name: string,
  factory: () => T,
): Resource<T> {
  return new Resource(name, factory);
}

/**
 * Initialize resource storage on a world. Called automatically on first
 * `Resource.get()`, but can be called explicitly at startup.
 */
export function initResources(world: object): void {
  ensureStore(world);
}

/** Bootstrap manifest: eagerly creates every declared resource before sealing. */
export class ResourceManifest {
  private readonly resources: readonly Resource<unknown>[];

  constructor(...resources: Resource<unknown>[]) {
    const names = new Set<string>();
    for (const resource of resources) {
      if (names.has(resource.name)) throw new Error(`duplicate resource manifest entry ${resource.name}`);
      names.add(resource.name);
    }
    this.resources = resources;
  }

  initialize(world: object): void {
    initResources(world);
    for (const resource of this.resources) resource.get(world);
  }

  seal(world: object): void {
    const missing: string[] = [];
    for (const resource of this.resources) if (!resource.has(world)) missing.push(resource.name);
    if (missing.length !== 0)
      throw new Error(`resources missing before gameplay seal: ${missing.join(', ')}`);
    sealResources(world);
  }

  initializeAndSeal(world: object): void {
    this.initialize(world);
    this.seal(world);
  }
}

/** Forbid lazy resource construction after bootstrap/warm-up. */
export function sealResources(world: object): void {
  ensureStore(world);
  (world as Record<symbol, unknown>)[RESOURCES_SEALED] = true;
}

export function resourcesAreSealed(world: object): boolean {
  return (world as Record<symbol, unknown>)[RESOURCES_SEALED] === true;
}
