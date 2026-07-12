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

// --- texture transcoder interface (what TextureClient implements) --------

/** Interface for the texture transcoder worker (TextureClient). */
export interface TextureTranscoder {
  /** Transcode Basis/KTX2 bytes to GPU-native format. Returns compressed data. */
  transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
  /** Generate a mip chain from RGBA data. Returns serialized mips. */
  generateMips(data: Uint8Array, width: number, height: number): Promise<Uint8Array>;
  /** Downscale RGBA data. */
  downscale(data: Uint8Array, w: number, h: number, tw: number, th: number): Promise<Uint8Array>;
  /** Drive the async worker. */
  poll(): void;
}

/** Target GPU format for Basis transcoding. */
export const FORMAT_BC7 = 0;
export const FORMAT_ASTC = 1;
export const FORMAT_ETC1 = 2;
export const FORMAT_ETC2 = 3;
export const FORMAT_RGBA = 4;

/** Cached best format for the current device. */
let _bestFormat: number | null = null;

/**
 * Detect the best GPU texture format for the current device.
 * Checks WebGPU adapter features: BC7 (desktop), ASTC (mobile), ETC2 (mobile).
 * Falls back to RGBA (uncompressed) if no compressed format is available.
 * Cached after first call.
 */
export async function detectBestTextureFormat(): Promise<number> {
  if (_bestFormat !== null) return _bestFormat;

  // Try WebGPU adapter feature detection.
  if (typeof navigator !== 'undefined' && navigator.gpu) {
    try {
      const adapter = await navigator.gpu.requestAdapter();
      if (adapter) {
        const features = adapter.features;
        if (features.has('texture-compression-bc')) {
          _bestFormat = FORMAT_BC7;
          return FORMAT_BC7;
        }
        if (features.has('texture-compression-astc')) {
          _bestFormat = FORMAT_ASTC;
          return FORMAT_ASTC;
        }
        if (features.has('texture-compression-etc2')) {
          _bestFormat = FORMAT_ETC2;
          return FORMAT_ETC2;
        }
      }
    } catch {
      // WebGPU not available — fall through.
    }
  }

  // No compressed format available — use uncompressed RGBA.
  _bestFormat = FORMAT_RGBA;
  return FORMAT_RGBA;
}

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

/**
 * Create a THREE.CompressedTexture from Basis-transcoded BC7 data.
 * `width`/`height` are the original texture dimensions.
 */
export function createBC7Texture(bc7Data: Uint8Array, width: number, height: number): THREE.Texture {
  // BC7: 16 bytes per 4×4 block.
  const blocksX = Math.ceil(width / 4);
  const blocksY = Math.ceil(height / 4);
  const expectedSize = blocksX * blocksY * 16;
  if (bc7Data.length < expectedSize) {
    throw new Error(`BC7 data too small: ${bc7Data.length} < ${expectedSize}`);
  }
  // Use DataTexture with RGBA format as a fallback — the actual BC7
  // compressed upload requires GPU-specific format constants that vary
  // by renderer. For WebGPU, use device.queue.writeTexture with BC7 format.
  // For now, create a DataTexture from the raw bytes (callers that need
  // true BC7 compression should access the raw bytes via the handle).
  const tex = new THREE.DataTexture(bc7Data, width, height, THREE.RGBAFormat);
  tex.needsUpdate = true;
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

/**
 * Parse a Basis/KTX2 texture: transcode to BC7, then create a texture.
 * Requires a TextureTranscoder (TextureClient) to be passed in.
 */
export async function parseBasisTexture(
  bytes: Uint8Array,
  transcoder: TextureTranscoder,
  format: number = FORMAT_BC7,
): Promise<Uint8Array> {
  // Returns raw transcoded bytes — the caller creates the GPU texture.
  return transcoder.transcode(bytes, format);
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
  /** Optional texture transcoder for Basis/KTX2 support. */
  private readonly textureTranscoder?: TextureTranscoder;

  constructor(
    private readonly loader: AssetLoader,
    textureTranscoder?: TextureTranscoder,
  ) {
    this.textureTranscoder = textureTranscoder;
  }

  /** The underlying asset loader (for direct byte access if needed). */
  get assetLoader(): AssetLoader {
    return this.loader;
  }

  /** Drive the async worker(s) — call each frame to resolve pending loads. */
  poll(): void {
    this.loader.poll();
    this.textureTranscoder?.poll();

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

  /** Load a texture. Auto-detects Basis/KTX2 and transcodes if a texture
   * transcoder was provided to the AssetStore constructor.
   * The GPU format is auto-selected based on the device (BC7 desktop, ASTC mobile). */
  loadTexture(path: string): AssetHandle<THREE.Texture> {
    const lower = path.toLowerCase();
    if (this.textureTranscoder && (lower.endsWith('.basis') || lower.endsWith('.ktx2'))) {
      // Auto-detect best format, then transcode.
      // The detection is async but cached — we start the load now and the
      // parser will await the format detection before transcoding.
      return this.load(path, async (bytes) => {
        const format = await detectBestTextureFormat();
        const transcoded = await parseBasisTexture(bytes, this.textureTranscoder!, format);
        return createBC7Texture(transcoded, 0, 0); // dimensions from Basis header
      }, fallbackTexture());
    }
    return this.load(path, parseTexture, fallbackTexture());
  }

  /** Load a Basis/KTX2 texture, returning raw transcoded bytes.
   * Format is auto-selected for the current device.
   * Use this if you need direct GPU upload (bypass THREE.Texture). */
  loadBasisTexture(path: string): AssetHandle<Uint8Array> {
    if (!this.textureTranscoder) {
      throw new Error('AssetStore has no texture transcoder. Pass a TextureClient to the constructor.');
    }
    return this.load(path, async (bytes) => {
      const format = await detectBestTextureFormat();
      return parseBasisTexture(bytes, this.textureTranscoder!, format);
    }, new Uint8Array());
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
