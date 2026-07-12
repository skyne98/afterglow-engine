// AssetStore — bridges the AssetLoaderClient (raw bytes over RPC) with Three.js.
//
// The store loads raw bytes via the async poll model, parses them into Three.js
// objects (textures, geometries, GLTF models), and caches them by path.
//
// `load()` returns an `AssetHandle<T>` immediately — no Promise, no callback.
// The handle starts with the fallback. Internally, the store loads the asset
// (chunked for large files, async via the poll model). When the asset is ready,
// the store swaps `handle.asset` and increments `handle.generation`.
//
// Consumer code checks the generation number each frame:
//
// ```ts
// const handle = store.load('sky.png', parseTexture, fallbackTex);
// let lastGen = -1;
//
// // Each frame:
// store.poll();
// if (handle.generation !== lastGen) {
//   material.map = handle.asset;
//   lastGen = handle.generation;
// }
// ```
//
// No callbacks. No effects. No closures. One integer comparison per frame.
//
// The store is an ECS Resource — one instance per world, accessed via:
//   const store = AssetStoreRes.get(world);

import * as THREE from 'three';

import { Resource, defineResource } from './resource.js';
import { AssetHandle, type AssetState } from './asset-handle.js';
import { fallbackTexture, fallbackGroup } from './fallback.js';

// --- asset loader interface (what AssetLoaderClient implements) -----------

export interface AssetLoader {
  load(path: string): Promise<Uint8Array>;
  size(path: string): Promise<number>;
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
  poll(): void;
}

/** A parser that turns raw bytes into a typed asset. */
export type AssetParser<T> = (bytes: Uint8Array) => Promise<T> | T;

/** Optional progress callback: `(loadedBytes, totalBytes)`. */
export type ProgressFn = (loaded: number, total: number) => void;

// --- constants -----------------------------------------------------------

/** Maximum bytes a single `load()` call can return (the response ring size). */
const MAX_SINGLE_LOAD = 1 << 20; // 1 MiB
/** Chunk size for streaming large assets. */
const CHUNK_SIZE = 512 * 1024; // 512 KiB

// --- parsers for common Three.js asset types ------------------------------

export async function parseTexture(bytes: Uint8Array): Promise<THREE.Texture> {
  const blob = new Blob([bytes]);
  const bitmap = await createImageBitmap(blob);
  const texture = new THREE.Texture(bitmap);
  texture.needsUpdate = true;
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

export async function parseDataTexture(bytes: Uint8Array): Promise<THREE.Texture> {
  return parseTexture(bytes);
}

export async function parseGLTF(
  bytes: Uint8Array,
  loader?: { parse(data: ArrayBuffer, path: string, onLoad: (result: { scene: THREE.Group }) => void, onError: (e: unknown) => void): void },
): Promise<THREE.Group> {
  const buffer = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buffer).set(bytes);

  if (loader) {
    return new Promise((resolve, reject) => {
      loader.parse(buffer, '', (result) => resolve(result.scene), reject);
    });
  }

  const GLTFLoader = (THREE as unknown as Record<string, unknown>).GLTFLoader as
    | (new () => { parse(data: ArrayBuffer, path: string, onLoad: (r: { scene: THREE.Group }) => void, onError: (e: unknown) => void): void })
    | undefined;

  if (!GLTFLoader) {
    throw new Error('GLTFLoader not available — pass a loader instance or include three examples');
  }

  const gltfLoader = new GLTFLoader();
  return new Promise((resolve, reject) => {
    gltfLoader.parse(buffer, '', (result) => resolve(result.scene), reject);
  });
}

export function parseJSON<T = unknown>(bytes: Uint8Array): T {
  return JSON.parse(new TextDecoder().decode(bytes)) as T;
}

// --- internal: a pending load -------------------------------------------

/** @internal — tracks an in-flight async load. */
interface PendingLoad<T> {
  handle: AssetHandle<T>;
  promise: Promise<Uint8Array>;
  parser: AssetParser<T>;
}

// --- AssetStore ----------------------------------------------------------

/**
 * Bridges the async asset worker with Three.js. Loads raw bytes via the
 * `AssetLoader` interface, parses them into Three.js objects, and caches
 * by path. Returns versioned handles — consumers check `.generation`.
 */
export class AssetStore {
  /** Parsed assets keyed by path (for sync `get()`). */
  private readonly cache = new Map<string, AssetHandle<unknown>>();
  /** In-flight loads keyed by path (prevents duplicate loads). */
  private readonly pending = new Map<string, PendingLoad<unknown>>();

  constructor(private readonly loader: AssetLoader) {}

  /** The underlying asset loader (for direct byte access if needed). */
  get assetLoader(): AssetLoader {
    return this.loader;
  }

  /** Drive the async worker — call each frame to resolve pending loads. */
  poll(): void {
    this.loader.poll();

    // Check completed loads — swap handles.
    for (const [path, pending] of this.pending) {
    // Promise.resolve() lets us check without awaiting.
      Promise.resolve(pending.promise).then(
        (bytes) => {
          // Bytes received — parse and swap.
          Promise.resolve(pending.parser(bytes)).then(
            (asset) => {
              pending.handle.asset = asset;
              pending.handle.generation++;
              pending.handle.state = 'ready';
              pending.handle.lod = 0;
              this.cache.set(path, pending.handle);
              this.pending.delete(path);
            },
            (err) => {
              pending.handle.state = 'error';
              console.error(`[afterglow] asset parse failed: ${path}`, err);
              this.pending.delete(path);
            },
          );
        },
        (err) => {
          pending.handle.state = 'error';
          console.error(`[afterglow] asset load failed: ${path}`, err);
          this.pending.delete(path);
        },
      );
    }
  }

  /**
   * Load an asset. Returns a handle immediately — the handle starts with
   * the fallback (or `undefined`). When the asset is ready, the store
   * swaps `handle.asset` and increments `handle.generation` (during `poll()`).
   *
   * If the asset is already cached, the returned handle is already at the
   * latest generation.
   *
   * If a load for the same path is already in-flight, returns the existing
   * handle (no duplicate load).
   */
  load<T>(path: string, parser: AssetParser<T>, fallback?: T): AssetHandle<T> {
    // Return cached handle if already loaded.
    const cached = this.cache.get(path);
    if (cached) return cached as AssetHandle<T>;

    // Return in-flight handle if already loading.
    const inflight = this.pending.get(path);
    if (inflight) return inflight.handle as AssetHandle<T>;

    // Create a new handle + start loading.
    const handle = new AssetHandle<T>(path, fallback);

    // Start the async load — chunked if large.
    const promise = this.startLoad(path);
    this.pending.set(path, { handle, promise, parser } as PendingLoad<unknown>);

    return handle;
  }

  /**
   * Get a cached handle by path. Returns `undefined` if not loaded or loading.
   * For the handle-based API, prefer `load()` which returns a handle immediately.
   */
  getHandle<T>(path: string): AssetHandle<T> | undefined {
    return this.cache.get(path) as AssetHandle<T> | undefined;
  }

  /** Is this asset cached and ready? */
  has(path: string): boolean {
    return this.cache.has(path);
  }

  /** Is this asset currently loading? */
  isLoading(path: string): boolean {
    return this.pending.has(path);
  }

  // --- Convenience methods for common Three.js types ---

  loadTexture(path: string): AssetHandle<THREE.Texture> {
    return this.load(path, parseTexture, fallbackTexture());
  }

  loadGLTF(path: string, loader?: Parameters<typeof parseGLTF>[1]): AssetHandle<THREE.Group> {
    return this.load(path, (bytes) => parseGLTF(bytes, loader), fallbackGroup());
  }

  loadJSON<T = unknown>(path: string): AssetHandle<T> {
    return this.load(path, parseJSON<T>, null as T);
  }

  /** Internal: start loading bytes (chunked if > 1 MiB). */
  private async startLoad(path: string): Promise<Uint8Array> {
    const total = await this.loader.size(path);
    if (total > MAX_SINGLE_LOAD) {
      return this.loadChunked(path, total);
    }
    return this.loader.load(path);
  }

  /** Internal: chunked load for assets larger than the 1 MiB response ring. */
  private async loadChunked(path: string, total: number): Promise<Uint8Array> {
    const chunks: Uint8Array[] = [];
    let offset = 0;
    while (offset < total) {
      const len = Math.min(CHUNK_SIZE, total - offset);
      const chunk = await this.loader.read(path, offset, len);
      chunks.push(chunk);
      offset += chunk.byteLength;
    }
    const bytes = new Uint8Array(total);
    let pos = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, pos);
      pos += chunk.byteLength;
    }
    return bytes;
  }

  /** Remove an asset from the cache (does not dispose GPU resources). */
  evict(path: string): void {
    this.cache.delete(path);
  }

  /** Number of cached assets. */
  get size(): number {
    return this.cache.size;
  }

  /** All cached asset paths. */
  get cachedPaths(): string[] {
    return [...this.cache.keys()];
  }

  /** Dispose all cached assets and clear the cache. */
  dispose(): void {
    for (const handle of this.cache.values()) {
      const asset = handle.asset as { dispose?: () => void } | undefined;
      asset?.dispose?.();
    }
    this.cache.clear();
    this.pending.clear();
  }
}

// --- ECS Resource registration -------------------------------------------

export const AssetStoreRes = defineResource<AssetStore>('assetStore', () => {
  throw new Error(
    'AssetStore resource not initialized. Call `AssetStoreRes.set(world, new AssetStore(client))` after spawning the AssetLoaderClient.',
  );
});
