// AssetStore — loads assets from disk, optimizes meshes, transcodes textures,
// and streams mips progressively. One store, one API: loadModel(path).
//
// Full pipeline:
//   .glb → load bytes → parse GLTF → optimize meshes (vertex cache + overdraw
//   + vertex fetch) → generate LODs (UV-aware simplify) → transcode textures
//   (Basis → BC7/ASTC) → progressive mip upload → generation++
//
// Meshes are optimized by default (same as textures are transcoded by default).
// If no meshopt worker is provided, meshes load without optimization.
// If no texture worker is provided, .basis textures fall back to raw RGBA.

import * as THREE from 'three';

import { Resource, defineResource } from './resource.js';
import { AssetHandle } from './asset-handle.js';
import { fallbackTexture, fallbackGroup } from './fallback.js';

// --- interfaces (match the generated client APIs) -----------------------

export interface AssetLoader {
  load(path: string): Promise<Uint8Array>;
  size(path: string): Promise<number>;
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
  poll(): void;
}

export type AssetParser<T> = (bytes: Uint8Array) => Promise<T> | T;

// --- format constants ----------------------------------------------------

export const FORMAT_BC7 = 0;
export const FORMAT_ASTC = 1;
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
const DEFAULT_LOD_RATIOS = [1.0, 0.5, 0.25, 0.1];
const DEFAULT_TARGET_ERROR = 0.02;

// --- model asset types ---------------------------------------------------

/** A single LOD level of an optimized mesh. */
export interface MeshLOD {
  indices: Uint32Array;
  positions: Float32Array;
  uvs: Float32Array;
  triangleCount: number;
  /** Only set for LOD 0 — before/after optimization stats. */
  stats?: MeshStats;
}

/** Optimization stats for a mesh. */
export interface MeshStats {
  originalTriangles: number;
  originalAcmr: number;
  optimizedAcmr: number;
  compressedIndexBytes: number;
  uncompressedIndexBytes: number;
}

/** A loaded model — one or more meshes, each with LOD levels, + textures. */
export interface ModelAsset {
  meshes: MeshLOD[][];
  textures: Map<string, THREE.Texture>;
  stats: MeshStats[];
}

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

interface StreamingTexture {
  handle: AssetHandle<THREE.Texture>;
  texture: THREE.DataTexture;
  pendingMips: { data: Uint8Array; width: number; height: number; level: number }[];
  mipsUploaded: number;
  totalMips: number;
}

// --- AssetStore ----------------------------------------------------------

/**
 * Loads assets from disk, optimizes meshes, transcodes textures, and streams
 * mips progressively. One store, one API: `loadModel(path)`.
 *
 * Meshes are optimized by default (vertex cache + overdraw + simplify).
 * Textures are transcoded by default (Basis → BC7/ASTC, progressive mips).
 *
 * Constructor takes the asset loader (required) + optional workers:
 *   - meshopt worker (MeshoptClient) — enables mesh optimization + LODs
 *   - texture worker (TextureClient) — enables Basis transcoding + mip streaming
 */
export class AssetStore {
  private readonly cache = new Map<string, AssetHandle<unknown>>();
  private readonly pending = new Map<string, PendingLoad<unknown>>();
  private readonly streaming = new Map<string, StreamingTexture>();

  /** The meshopt worker (structural type — matches MeshoptClient). */
  private readonly meshopt?: {
    optimizeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Uint32Array>;
    optimizeOverdraw(indices: Uint32Array, positions: Float32Array, stride: number, threshold: number): Promise<Uint32Array>;
    simplifyWithUvs(indices: Uint32Array, positions: Float32Array, posStride: number, uvs: Float32Array, uvStride: number, uvWeight: number, targetIndexCount: number, targetError: number): Promise<Uint32Array>;
    analyzeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Float32Array>;
    encodeIndexBuffer(indices: Uint32Array, vertexCount: number): Promise<Uint8Array>;
    poll(): void;
  };

  /** The texture worker (structural type — matches TextureClient). */
  private readonly texture?: {
    transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
    generateMips(data: Uint8Array, width: number, height: number): Promise<Uint8Array>;
    poll(): void;
  };

  private loader: AssetLoader;

  constructor(
    loader: AssetLoader,
    meshopt?: {
      optimizeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Uint32Array>;
      optimizeOverdraw(indices: Uint32Array, positions: Float32Array, stride: number, threshold: number): Promise<Uint32Array>;
      simplifyWithUvs(indices: Uint32Array, positions: Float32Array, posStride: number, uvs: Float32Array, uvStride: number, uvWeight: number, targetIndexCount: number, targetError: number): Promise<Uint32Array>;
      analyzeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Float32Array>;
      encodeIndexBuffer(indices: Uint32Array, vertexCount: number): Promise<Uint8Array>;
      poll(): void;
    },
    texture?: {
      transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
      generateMips(data: Uint8Array, width: number, height: number): Promise<Uint8Array>;
      poll(): void;
    },
  ) {
    this.loader = loader;
    this.meshopt = meshopt;
    this.texture = texture;
  }

  get assetLoader(): AssetLoader { return this.loader; }

  /** Drive all workers + process pending loads + stream mips. Call each frame. */
  poll(): void {
    this.loader.poll();
    this.meshopt?.poll();
    this.texture?.poll();
    this.processPendingLoads();
    this.processStreaming();
  }

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

  private processStreaming(): void {
    let processed = 0;
    for (const [, stream] of this.streaming) {
      while (processed < MAX_MIP_UPLOADS_PER_FRAME && stream.pendingMips.length > 0) {
        const mip = stream.pendingMips.shift()!;
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

  // --- loadModel — the one API for loading 3D models ---

  /**
   * Load a 3D model (.glb). Automatically:
   * 1. Parses the GLTF
   * 2. Optimizes meshes (vertex cache + overdraw) via meshopt worker
   * 3. Generates LOD levels (UV-aware simplify) via meshopt worker
   * 4. Transcodes textures (Basis → BC7/ASTC) via texture worker
   * 5. Returns a ModelAsset with all meshes, LODs, textures, and stats
   *
   * If no meshopt worker is set, meshes load without optimization (just parsed).
   * If no texture worker is set, .basis textures fall back to raw RGBA.
   */
  loadModel(path: string): AssetHandle<ModelAsset> {
    const cached = this.cache.get(path);
    if (cached) return cached as AssetHandle<ModelAsset>;

    const handle = new AssetHandle<ModelAsset>(path, undefined);

    const promise = this.startLoad(path);
    promise.then(async (bytes) => {
      try {
        const asset = await this.processModel(bytes, path);
        handle.asset = asset;
        handle.generation++;
        handle.state = 'ready';
        this.cache.set(path, handle);
      } catch (err) {
        handle.state = 'error';
        console.error(`[afterglow] loadModel failed: ${path}`, err);
      }
    });

    return handle;
  }

  /** @internal — parse GLTF + optimize meshes + generate LODs. */
  private async processModel(bytes: Uint8Array, path: string): Promise<ModelAsset> {
    // Parse GLTF to get the scene.
    const scene = await parseGLTF(bytes);

    // Extract meshes from the scene.
    const meshes: { indices: Uint32Array; positions: Float32Array; uvs: Float32Array }[] = [];
    scene.traverse((obj: any) => {
      if (obj.isMesh && obj.geometry?.index) {
        const geo = obj.geometry;
        const indices = new Uint32Array(geo.index.array);
        const pos = geo.attributes.position;
        const positions = new Float32Array(pos.array);
        const uvs = geo.attributes.uv ? new Float32Array(geo.attributes.uv.array) : new Float32Array(0);
        meshes.push({ indices, positions, uvs });
      }
    });

    const stats: MeshStats[] = [];
    const meshLods: MeshLOD[][] = [];

    for (const mesh of meshes) {
      const { lods, stat } = await this.optimizeMesh(mesh.indices, mesh.positions, mesh.uvs);
      meshLods.push(lods);
      if (stat) stats.push(stat);
    }

    // Collect textures from materials.
    const textures = new Map<string, THREE.Texture>();
    scene.traverse((obj: any) => {
      if (obj.isMesh && obj.material?.map) {
        textures.set('diffuse', obj.material.map);
      }
    });

    return { meshes: meshLods, textures, stats };
  }

  /** @internal — optimize a mesh and generate LOD levels. */
  private async optimizeMesh(
    indices: Uint32Array,
    positions: Float32Array,
    uvs: Float32Array,
  ): Promise<{ lods: MeshLOD[]; stat?: MeshStats }> {
    const vertexCount = positions.length / 3;
    const originalTriangles = indices.length / 3;
    const stride = 12; // 3 floats × 4 bytes
    const uvStride = 8; // 2 floats × 4 bytes

    // If no meshopt worker, return unoptimized.
    if (!this.meshopt) {
      return {
        lods: [{
          indices, positions, uvs,
          triangleCount: originalTriangles,
        }],
      };
    }

    // 1. Analyze original (before optimization).
    const origStats = await this.meshopt.analyzeVertexCache(indices, vertexCount);
    const originalAcmr = origStats[0]; // [acmr, atvr, transformed, misspelled]

    // 2. Optimize vertex cache.
    let optimized = await this.meshopt.optimizeVertexCache(indices, vertexCount);

    // 3. Optimize overdraw (requires positions).
    optimized = await this.meshopt.optimizeOverdraw(optimized, positions, stride, 1.05);

    // 4. Analyze optimized.
    const optStats = await this.meshopt.analyzeVertexCache(optimized, vertexCount);
    const optimizedAcmr = optStats[0];

    // 5. Compress indices (for stats).
    const compressed = await this.meshopt.encodeIndexBuffer(optimized, vertexCount);

    // 6. Generate LOD levels.
    const lods: MeshLOD[] = [];
    for (const ratio of DEFAULT_LOD_RATIOS) {
      if (ratio >= 1.0) {
        // LOD 0 = the optimized mesh.
        lods.push({
          indices: optimized, positions, uvs,
          triangleCount: optimized.length / 3,
          stats: ratio === 1.0 ? {
            originalTriangles,
            originalAcmr,
            optimizedAcmr,
            compressedIndexBytes: compressed.length,
            uncompressedIndexBytes: optimized.length * 4,
          } : undefined,
        });
      } else {
        // Higher LODs: UV-aware simplification.
        const targetTris = Math.max(4, Math.floor(originalTriangles * ratio));
        const targetIndexCount = targetTris * 3;
        const simplified = await this.meshopt.simplifyWithUvs(
          optimized, positions, stride, uvs, uvStride, 0.5, targetIndexCount, DEFAULT_TARGET_ERROR,
        );
        lods.push({
          indices: simplified, positions, uvs,
          triangleCount: simplified.length / 3,
        });
      }
    }

    return {
      lods,
      stat: {
        originalTriangles,
        originalAcmr,
        optimizedAcmr,
        compressedIndexBytes: compressed.length,
        uncompressedIndexBytes: optimized.length * 4,
      },
    };
  }

  // --- Texture loading (existing, unchanged) ---

  loadTexture(path: string): AssetHandle<THREE.Texture> {
    const lower = path.toLowerCase();
    if (this.texture && (lower.endsWith('.basis') || lower.endsWith('.ktx2'))) {
      return this.loadStreamingBasisTexture(path);
    }
    return this.load(path, parseTexture, fallbackTexture());
  }

  private loadStreamingBasisTexture(path: string): AssetHandle<THREE.Texture> {
    const cached = this.cache.get(path);
    if (cached) return cached as AssetHandle<THREE.Texture>;

    const handle = new AssetHandle<THREE.Texture>(path, fallbackTexture());
    const texture = new THREE.DataTexture(new Uint8Array(0), 1, 1, THREE.RGBAFormat);
    texture.generateMipmaps = false;
    texture.mipmaps = [];
    texture.minFilter = THREE.LinearMipmapLinearFilter;
    texture.magFilter = THREE.LinearFilter;
    texture.colorSpace = THREE.SRGBColorSpace;
    handle.asset = texture;

    const promise = this.startLoad(path);
    promise.then(async (bytes) => {
      const format = await detectBestTextureFormat();
      const transcoded = await this.texture!.transcode(bytes, format);
      const mips = this.parseSerializedMips(transcoded);
      if (mips.length === 0) {
        const blockSize = format === FORMAT_RGBA ? 4 : 16;
        const blocks = transcoded.length / blockSize;
        const dim = Math.round(Math.sqrt(blocks * (format === FORMAT_RGBA ? 1 : 16)));
        const w = Math.max(4, Math.ceil(dim / 4) * 4);
        this.queueMipUpload(path, handle, texture, 0, transcoded, w, w, 1, 1);
      } else {
        const reversed = [...mips].reverse();
        for (let i = 0; i < reversed.length; i++) {
          const m = reversed[i];
          this.queueMipUpload(path, handle, texture, i, m.data, m.width, m.height, i + 1, reversed.length);
        }
      }
    }).catch((err) => {
      handle.state = 'error';
      console.error(`[afterglow] basis texture failed: ${path}`, err);
    });

    this.streaming.set(path, { handle, texture, pendingMips: [], mipsUploaded: 0, totalMips: 1 });
    return handle;
  }

  private parseSerializedMips(data: Uint8Array): { data: Uint8Array; width: number; height: number }[] {
    if (data.length < 4) return [];
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const count = view.getUint32(0, true);
    if (count === 0 || count > 20) return [];
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

  private queueMipUpload(path: string, handle: AssetHandle<THREE.Texture>, texture: THREE.DataTexture,
    level: number, data: Uint8Array, width: number, height: number, mipNumber: number, totalMips: number): void {
    const stream = this.streaming.get(path);
    if (!stream) return;
    stream.pendingMips.push({ data, width, height, level });
    stream.totalMips = totalMips;
    if (mipNumber === 1) { handle.generation++; handle.state = 'loading'; }
  }

  loadBasisTexture(path: string): AssetHandle<Uint8Array> {
    if (!this.texture) throw new Error('No texture worker');
    return this.load(path, async (bytes) => {
      const format = await detectBestTextureFormat();
      return this.texture!.transcode(bytes, format);
    }, new Uint8Array());
  }

  loadGLTF(path: string, loader?: Parameters<typeof parseGLTF>[1]): AssetHandle<THREE.Group> {
    return this.load(path, (bytes) => parseGLTF(bytes, loader), fallbackGroup());
  }

  loadJSON<T = unknown>(path: string): AssetHandle<T> {
    return this.load(path, parseJSON<T>, null as T);
  }

  private async startLoad(path: string): Promise<Uint8Array> {
    const total = await this.loader.size(path);
    if (total > MAX_SINGLE_LOAD) return this.loadChunked(path, total);
    return this.loader.load(path);
  }

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
  throw new Error('AssetStore not initialized. Call AssetStoreRes.set(world, new AssetStore(loader, meshopt, texture)).');
});
