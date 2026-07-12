// AssetStore — loads assets from disk, transcodes, creates GPU textures,
// and streams mips progressively. One store, no bridge needed.
//
// The store handles the full pipeline:
//   disk → bytes (asset worker) → transcode (texture worker) → GPU texture
//   → progressive mip upload (texture.mipmaps + needsUpdate) → generation++
//
// Consumer code checks generation each frame:
//
// ```ts
// const handle = store.loadTexture('sky.basis');
// // handle.asset = GMod checkerboard fallback (immediately)
// let lastGen = -1;
//
// // Each frame:
// store.poll();
// if (handle.generation !== lastGen) {
//   material.map = handle.asset;
//   lastGen = handle.generation;
// }
// ```

import * as THREE from 'three';

import { Resource, defineResource } from './resource.js';
import { AssetHandle } from './asset-handle.js';
import { fallbackTexture, fallbackGroup } from './fallback.js';

// --- interfaces ----------------------------------------------------------

export interface AssetLoader {
  load(path: string): Promise<Uint8Array>;
  size(path: string): Promise<number>;
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
  poll(): void;
}

export interface TextureTranscoder {
  transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
  generateMips(data: Uint8Array, width: number, height: number): Promise<Uint8Array>;
  downscale(data: Uint8Array, w: number, h: number, tw: number, th: number): Promise<Uint8Array>;
  poll(): void;
}

export type AssetParser<T> = (bytes: Uint8Array) => Promise<T> | T;

// --- format constants ----------------------------------------------------

export const FORMAT_BC7 = 0;
export const FORMAT_ASTC = 1;
export const FORMAT_ETC1 = 2;
export const FORMAT_ETC2 = 3;
export const FORMAT_RGBA = 4;

let _bestFormat: number | null = null;

export async function detectBestTextureFormat(): Promise<number> {
  if (_bestFormat !== null) return _bestFormat;
  if (typeof navigator !== 'undefined' && navigator.gpu) {
    try {
      const adapter = await navigator.gpu.requestAdapter();
      if (adapter) {
        const f = adapter.features;
        if (f.has('texture-compression-bc')) { _bestFormat = FORMAT_BC7; return FORMAT_BC7; }
        if (f.has('texture-compression-astc')) { _bestFormat = FORMAT_ASTC; return FORMAT_ASTC; }
        if (f.has('texture-compression-etc2')) { _bestFormat = FORMAT_ETC2; return FORMAT_ETC2; }
      }
    } catch { /* fall through */ }
  }
  _bestFormat = FORMAT_RGBA;
  return FORMAT_RGBA;
}

// --- constants -----------------------------------------------------------

const MAX_SINGLE_LOAD = 1 << 20;
const CHUNK_SIZE = 512 * 1024;
const MAX_MIP_UPLOADS_PER_FRAME = 2;

// --- parsers -------------------------------------------------------------

export async function parseTexture(bytes: Uint8Array): Promise<THREE.Texture> {
  const bitmap = await createImageBitmap(new Blob([bytes]));
  const tex = new THREE.Texture(bitmap);
  tex.needsUpdate = true;
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

export async function parseGLTF(
  bytes: Uint8Array,
  loader?: { parse(data: ArrayBuffer, path: string, onLoad: (r: { scene: THREE.Group }) => void, onError: (e: unknown) => void): void },
): Promise<THREE.Group> {
  const buf = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buf).set(bytes);
  if (loader) return new Promise((res, rej) => loader.parse(buf, '', r => res(r.scene), rej));
  const GLTFLoader = (THREE as unknown as Record<string, unknown>).GLTFLoader as
    | (new () => { parse(d: ArrayBuffer, p: string, l: (r: { scene: THREE.Group }) => void, e: (e: unknown) => void): void }) | undefined;
  if (!GLTFLoader) throw new Error('GLTFLoader not available');
  const gl = new GLTFLoader();
  return new Promise((res, rej) => gl.parse(buf, '', r => res(r.scene), rej));
}

export function parseJSON<T = unknown>(bytes: Uint8Array): T {
  return JSON.parse(new TextDecoder().decode(bytes)) as T;
}

// --- internal types ------------------------------------------------------

interface PendingLoad<T> {
  handle: AssetHandle<T>;
  promise: Promise<Uint8Array>;
  parser: AssetParser<T>;
}

/** @internal — tracks a texture that needs progressive mip streaming. */
interface StreamingTexture {
  handle: AssetHandle<THREE.Texture>;
  texture: THREE.DataTexture;
  /** Mip levels waiting to be uploaded (lowest first). */
  pendingMips: { data: Uint8Array; width: number; height: number; level: number }[];
  mipsUploaded: number;
  totalMips: number;
  lastGen: number;
}

// --- AssetStore ----------------------------------------------------------

export class AssetStore {
  private readonly cache = new Map<string, AssetHandle<unknown>>();
  private readonly pending = new Map<string, PendingLoad<unknown>>();
  private readonly textureTranscoder?: TextureTranscoder;
  /** Textures that need progressive mip streaming. */
  private readonly streaming = new Map<string, StreamingTexture>();

  constructor(loader: AssetLoader, textureTranscoder?: TextureTranscoder) {
    this.loader = loader;
    this.textureTranscoder = textureTranscoder;
  }

  private loader: AssetLoader;

  get assetLoader(): AssetLoader { return this.loader; }

  /** Drive all workers + process pending loads + stream mips. Call each frame. */
  poll(): void {
    this.loader.poll();
    this.textureTranscoder?.poll();
    this.processPendingLoads();
    this.processStreaming();
  }

  /** @internal — check completed loads, parse, swap handles. */
  private processPendingLoads(): void {
    for (const [path, pending] of this.pending) {
      Promise.resolve(pending.promise).then(
        (bytes) => {
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
              console.error(`[afterglow] parse failed: ${path}`, err);
              this.pending.delete(path);
            },
          );
        },
        (err) => {
          pending.handle.state = 'error';
          console.error(`[afterglow] load failed: ${path}`, err);
          this.pending.delete(path);
        },
      );
    }
  }

  /** @internal — process pending mip uploads (time-sliced). */
  private processStreaming(): void {
    let processed = 0;
    for (const [, stream] of this.streaming) {
      while (processed < MAX_MIP_UPLOADS_PER_FRAME && stream.pendingMips.length > 0) {
        const mip = stream.pendingMips.shift()!;
        // Upload this mip level to the texture.
        stream.texture.mipmaps![mip.level] = { data: mip.data, width: mip.width, height: mip.height };
        if (mip.level === 0) {
          stream.texture.image = { data: mip.data, width: mip.width, height: mip.height };
        }
        stream.texture.needsUpdate = true;
        stream.mipsUploaded++;
        stream.handle.generation++;
        processed++;
      }
      if (stream.mipsUploaded >= stream.totalMips) {
        stream.handle.state = 'ready';
        this.streaming.delete(stream.handle.path);
      }
    }
  }

  /** Load an asset with a custom parser. Returns handle immediately. */
  load<T>(path: string, parser: AssetParser<T>, fallback?: T): AssetHandle<T> {
    const cached = this.cache.get(path);
    if (cached) return cached as AssetHandle<T>;
    const inflight = this.pending.get(path);
    if (inflight) return inflight.handle as AssetHandle<T>;
    const handle = new AssetHandle<T>(path, fallback);
    const promise = this.startLoad(path);
    this.pending.set(path, { handle, promise, parser } as PendingLoad<unknown>);
    return handle;
  }

  getHandle<T>(path: string): AssetHandle<T> | undefined {
    return this.cache.get(path) as AssetHandle<T> | undefined;
  }

  has(path: string): boolean { return this.cache.has(path); }
  isLoading(path: string): boolean { return this.pending.has(path); }

  // --- Texture loading ---

  /**
   * Load a texture. Auto-detects Basis/KTX2 and transcodes to the device's
   * best GPU format (BC7 desktop, ASTC mobile). Mips are streamed
   * progressively — lowest first, then higher, one per frame.
   *
   * - `.basis`/`.ktx2`: transcode → progressive mip streaming
   * - `.png`/`.jpg`/etc: decode → upload, GPU generates mips
   */
  loadTexture(path: string): AssetHandle<THREE.Texture> {
    const lower = path.toLowerCase();
    if (this.textureTranscoder && (lower.endsWith('.basis') || lower.endsWith('.ktx2'))) {
      return this.loadStreamingBasisTexture(path);
    }
    return this.load(path, parseTexture, fallbackTexture());
  }

  /** @internal — load a Basis texture with progressive mip streaming. */
  private loadStreamingBasisTexture(path: string): AssetHandle<THREE.Texture> {
    const cached = this.cache.get(path);
    if (cached) return cached as AssetHandle<THREE.Texture>;

    const handle = new AssetHandle<THREE.Texture>(path, fallbackTexture());

    // Create the GPU texture immediately (empty mipmaps array).
    const texture = new THREE.DataTexture(new Uint8Array(0), 1, 1, THREE.RGBAFormat);
    texture.generateMipmaps = false;
    texture.mipmaps = [];
    texture.minFilter = THREE.LinearMipmapLinearFilter;
    texture.magFilter = THREE.LinearFilter;
    texture.colorSpace = THREE.SRGBColorSpace;
    handle.asset = texture;

    // Start loading + transcoding.
    const promise = this.startLoad(path);
    promise.then(async (bytes) => {
      const format = await detectBestTextureFormat();
      const transcoded = await this.textureTranscoder!.transcode(bytes, format);

      // Parse the serialized mip data from the transcoder.
      // Format: [count(u32)][w0(u32)][h0(u32)][len0(u32)][data0...][w1...]...
      const mips = this.parseSerializedMips(transcoded);
      if (mips.length === 0) {
        // No mip data — upload as single level.
        // Estimate dimensions from data size.
        const blockSize = format === FORMAT_RGBA ? 4 : 16;
        const blocks = transcoded.length / blockSize;
        const dim = Math.round(Math.sqrt(blocks * (format === FORMAT_RGBA ? 1 : 16)));
        const w = Math.max(4, Math.ceil(dim / 4) * 4);
        const h = w;
        this.queueMipUpload(path, handle, texture, 0, transcoded, w, h, 1, 1);
      } else {
        // Queue mips for streaming (lowest first — they're in the serialized
        // data from highest to lowest, so reverse).
        const reversed = [...mips].reverse();
        const totalMips = reversed.length;
        for (let i = 0; i < reversed.length; i++) {
          const m = reversed[i];
          this.queueMipUpload(path, handle, texture, i, m.data, m.width, m.height, i + 1, totalMips);
        }
      }
    }).catch((err) => {
      handle.state = 'error';
      console.error(`[afterglow] basis texture failed: ${path}`, err);
    });

    // Track for streaming.
    this.streaming.set(path, {
      handle, texture, pendingMips: [], mipsUploaded: 0, totalMips: 1, lastGen: 0,
    });

    return handle;
  }

  /** @internal — parse serialized mip data from the transcoder. */
  private parseSerializedMips(data: Uint8Array): { data: Uint8Array; width: number; height: number }[] {
    if (data.length < 4) return [];
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const count = view.getUint32(0, true);
    if (count === 0 || count > 20) return []; // sanity check
    let offset = 4;
    const mips: { data: Uint8Array; width: number; height: number }[] = [];
    for (let i = 0; i < count; i++) {
      if (offset + 12 > data.length) break;
      const w = view.getUint32(offset, true); offset += 4;
      const h = view.getUint32(offset, true); offset += 4;
      const len = view.getUint32(offset, true); offset += 4;
      if (offset + len > data.length) break;
      mips.push({ data: data.slice(offset, offset + len), width: w, height: h });
      offset += len;
    }
    return mips;
  }

  /** @internal — queue a mip level for progressive upload. */
  private queueMipUpload(
    path: string,
    handle: AssetHandle<THREE.Texture>,
    texture: THREE.DataTexture,
    level: number,
    data: Uint8Array,
    width: number,
    height: number,
    mipNumber: number,
    totalMips: number,
  ): void {
    const stream = this.streaming.get(path);
    if (!stream) return;
    stream.pendingMips.push({ data, width, height, level });
    stream.totalMips = totalMips;
    if (mipNumber === 1) {
      // First mip uploaded — update the handle immediately.
      handle.generation++;
      handle.state = 'loading'; // partial — more mips coming
    }
  }

  /** Load a Basis texture, returning raw transcoded bytes. */
  loadBasisTexture(path: string): AssetHandle<Uint8Array> {
    if (!this.textureTranscoder) throw new Error('No texture transcoder');
    return this.load(path, async (bytes) => {
      const format = await detectBestTextureFormat();
      return this.textureTranscoder!.transcode(bytes, format);
    }, new Uint8Array());
  }

  loadGLTF(path: string, loader?: Parameters<typeof parseGLTF>[1]): AssetHandle<THREE.Group> {
    return this.load(path, (bytes) => parseGLTF(bytes, loader), fallbackGroup());
  }

  loadJSON<T = unknown>(path: string): AssetHandle<T> {
    return this.load(path, parseJSON<T>, null as T);
  }

  /** @internal — start loading bytes (chunked if large). */
  private async startLoad(path: string): Promise<Uint8Array> {
    const total = await this.loader.size(path);
    if (total > MAX_SINGLE_LOAD) return this.loadChunked(path, total);
    return this.loader.load(path);
  }

  /** @internal — chunked load for large assets. */
  private async loadChunked(path: string, total: number): Promise<Uint8Array> {
    const chunks: Uint8Array[] = [];
    let offset = 0;
    while (offset < total) {
      const len = Math.min(CHUNK_SIZE, total - offset);
      chunks.push(await this.loader.read(path, offset, len));
      offset += chunks[chunks.length - 1].byteLength;
    }
    const bytes = new Uint8Array(total);
    let pos = 0;
    for (const c of chunks) { bytes.set(c, pos); pos += c.byteLength; }
    return bytes;
  }

  evict(path: string): void { this.cache.delete(path); this.streaming.delete(path); }
  get size(): number { return this.cache.size; }
  get cachedPaths(): string[] { return [...this.cache.keys()]; }

  dispose(): void {
    for (const h of this.cache.values()) (h.asset as { dispose?: () => void })?.dispose?.();
    this.cache.clear();
    this.pending.clear();
    this.streaming.clear();
  }
}

export const AssetStoreRes = defineResource<AssetStore>('assetStore', () => {
  throw new Error('AssetStore not initialized. Call AssetStoreRes.set(world, new AssetStore(loader, textureClient)).');
});
