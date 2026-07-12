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
//   import { defineResource } from './resource.js';
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
    if (!(this.name in store)) store[this.name] = this.factory();
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
