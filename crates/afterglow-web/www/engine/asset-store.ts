// AssetStore — loads assets from disk, optimizes meshes, and delegates all
// texture loading to the VirtualTextureStore (universal VT).
//
// Full pipeline:
//   .glb → load bytes → parse GLTF → optimize meshes (vertex cache + overdraw
//   + vertex fetch) → generate LODs (UV-aware simplify) → return ModelAsset
//
// Textures are ALL virtual — loaded page-by-page into the shared atlas via
// VirtualTextureStore. No per-texture DataTextures, no format detection,
// no progressive mip streaming code. The page table + atlas handle everything.
//
// Meshes are optimized by default. If no meshopt worker is provided, meshes
// load without optimization.

import * as THREE from 'three';

import { Resource, defineResource } from './resource.js';
import { AssetHandle } from './asset-handle.js';
import { fallbackGroup } from './fallback.js';
import type { VirtualTextureStore } from './virtual-texture.js';

// --- interfaces (match the generated client APIs) -----------------------

export interface AssetLoader {
  load(path: string): Promise<Uint8Array>;
  size(path: string): Promise<number>;
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
  poll(): void;
}

export type AssetParser<T> = (bytes: Uint8Array) => Promise<T> | T;

// --- constants -----------------------------------------------------------

const MAX_SINGLE_LOAD = 1 << 20;
const CHUNK_SIZE = 512 * 1024;
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

// --- AssetStore ----------------------------------------------------------

/**
 * Loads assets from disk, optimizes meshes, and delegates all texture
 * loading to the VirtualTextureStore (universal VT).
 *
 * Meshes are optimized by default (vertex cache + overdraw + simplify).
 * If no meshopt worker is provided, meshes load without optimization.
 */
export class AssetStore {
  private readonly cache = new Map<string, AssetHandle<unknown>>();
  private readonly pending = new Map<string, PendingLoad<unknown>>();

  /** The meshopt worker (structural type — matches MeshoptClient). */
  private readonly meshopt?: {
    optimizeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Uint32Array>;
    optimizeOverdraw(indices: Uint32Array, positions: Float32Array, stride: number, threshold: number): Promise<Uint32Array>;
    simplifyWithUvs(indices: Uint32Array, positions: Float32Array, posStride: number, uvs: Float32Array, uvStride: number, uvWeight: number, targetIndexCount: number, targetError: number): Promise<Uint32Array>;
    analyzeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Float32Array>;
    encodeIndexBuffer(indices: Uint32Array, vertexCount: number): Promise<Uint8Array>;
    poll(): void;
  };

  private loader: AssetLoader;
  private vtStore: VirtualTextureStore | null = null;

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
  ) {
    this.loader = loader;
    this.meshopt = meshopt;
  }

  get assetLoader(): AssetLoader { return this.loader; }

  /** Set the VirtualTextureStore — enables universal VT for all textures. */
  setVirtualTextureStore(vt: VirtualTextureStore) { this.vtStore = vt; }

  /** Get the VirtualTextureStore (if set). */
  get virtualTextureStore(): VirtualTextureStore | null { return this.vtStore; }

  /** Drive all workers + process pending loads. Call each frame. */
  poll(): void {
    this.loader.poll();
    this.meshopt?.poll();
    this.processPendingLoads();
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
    let meshes: { indices: Uint32Array; positions: Float32Array; uvs: Float32Array }[] = [];
    let textures = new Map<string, THREE.Texture>();

    try {
      const scene = await parseGLTF(bytes);
      scene.traverse((obj: any) => {
        if (obj.isMesh && obj.geometry?.index) {
          const geo = obj.geometry;
          meshes.push({
            indices: new Uint32Array(geo.index.array),
            positions: new Float32Array(geo.attributes.position.array),
            uvs: geo.attributes.uv ? new Float32Array(geo.attributes.uv.array) : new Float32Array(0),
          });
        }
        if (obj.isMesh && obj.material?.map) textures.set('diffuse', obj.material.map);
      });
    } catch {
      meshes = this.parseMinimalGLB(bytes);
    }

    const stats: MeshStats[] = [];
    const meshLods: MeshLOD[][] = [];
    for (const mesh of meshes) {
      const { lods, stat } = await this.optimizeMesh(mesh.indices, mesh.positions, mesh.uvs);
      meshLods.push(lods);
      if (stat) stats.push(stat);
    }
    return { meshes: meshLods, textures, stats };
  }

  /** @internal — parse a minimal GLB (one mesh, POSITION + TEXCOORD_0 + indices). */
  private parseMinimalGLB(bytes: Uint8Array): { indices: Uint32Array; positions: Float32Array; uvs: Float32Array }[] {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (view.getUint32(0, true) !== 0x46546C67) throw new Error('not a GLB');
    const totalLen = view.getUint32(8, true);
    let off = 12;
    let json: any = null;
    let bin: Uint8Array | null = null;
    while (off < totalLen) {
      const len = view.getUint32(off, true); off += 4;
      const type = view.getUint32(off, true); off += 4;
      if (type === 0x4E4F534A) json = JSON.parse(new TextDecoder().decode(bytes.subarray(off, off + len)));
      else if (type === 0x004E4942) bin = bytes.subarray(off, off + len);
      off += len;
    }
    if (!json || !bin) throw new Error('GLB missing JSON or BIN');

    const accs = json.accessors || [];
    const bvs = json.bufferViews || [];
    const prims = json.meshes?.[0]?.primitives || [];
    const meshes: { indices: Uint32Array; positions: Float32Array; uvs: Float32Array }[] = [];
    for (const prim of prims) {
      const read = (aIdx: number, T: any) => {
        if (aIdx === undefined) return new T(0);
        const a = accs[aIdx];
        const bv = bvs[a.bufferView];
        const o = (bv.byteOffset || 0) + (a.byteOffset || 0);
        const comps = a.type === 'VEC3' ? 3 : a.type === 'VEC2' ? 2 : 1;
        return new T(bin.buffer, bin.byteOffset + o, a.count * comps).slice();
      };
      meshes.push({
        indices: read(prim.indices, Uint32Array),
        positions: read(prim.attributes.POSITION, Float32Array),
        uvs: prim.attributes.TEXCOORD_0 !== undefined ? read(prim.attributes.TEXCOORD_0, Float32Array) : new Float32Array(0),
      });
    }
    return meshes;
  }

  /** @internal — optimize a mesh and generate LOD levels. */
  private async optimizeMesh(
    indices: Uint32Array,
    positions: Float32Array,
    uvs: Float32Array,
  ): Promise<{ lods: MeshLOD[]; stat?: MeshStats }> {
    const vertexCount = positions.length / 3;
    const originalTriangles = indices.length / 3;
    const stride = 12;
    const uvStride = 8;

    if (!this.meshopt) {
      return {
        lods: [{ indices, positions, uvs, triangleCount: originalTriangles }],
      };
    }

    const origStats = await this.meshopt.analyzeVertexCache(indices, vertexCount);
    const originalAcmr = origStats[0];

    let optimized = await this.meshopt.optimizeVertexCache(indices, vertexCount);
    optimized = await this.meshopt.optimizeOverdraw(optimized, positions, stride, 1.05);

    const optStats = await this.meshopt.analyzeVertexCache(optimized, vertexCount);
    const optimizedAcmr = optStats[0];

    const compressed = await this.meshopt.encodeIndexBuffer(optimized, vertexCount);

    const lods: MeshLOD[] = [];
    for (const ratio of DEFAULT_LOD_RATIOS) {
      if (ratio >= 1.0) {
        lods.push({
          indices: optimized, positions, uvs,
          triangleCount: optimized.length / 3,
          stats: ratio === 1.0 ? {
            originalTriangles, originalAcmr, optimizedAcmr,
            compressedIndexBytes: compressed.length,
            uncompressedIndexBytes: optimized.length * 4,
          } : undefined,
        });
      } else {
        const targetTris = Math.max(4, Math.floor(originalTriangles * ratio));
        const targetIndexCount = targetTris * 3;
        const simplified = await this.meshopt.simplifyWithUvs(
          optimized, positions, stride, uvs, uvStride, 0.5, targetIndexCount, DEFAULT_TARGET_ERROR,
        );
        lods.push({ indices: simplified, positions, uvs, triangleCount: simplified.length / 3 });
      }
    }

    return {
      lods,
      stat: { originalTriangles, originalAcmr, optimizedAcmr,
        compressedIndexBytes: compressed.length, uncompressedIndexBytes: optimized.length * 4 },
    };
  }

  // --- Texture loading — ALL through VirtualTextureStore ---

  loadTexture(path: string): AssetHandle<THREE.Texture> {
    if (this.vtStore) {
      return this.vtStore.loadTexture(path);
    }
    // Fallback: no VT store — use basic texture loading
    return this.load(path, parseTexture, undefined);
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

  evict(path: string): void { this.cache.delete(path); }
  get size(): number { return this.cache.size; }
  get cachedPaths(): string[] { return [...this.cache.keys()]; }

  dispose(): void {
    for (const h of this.cache.values()) (h.asset as { dispose?: () => void })?.dispose?.();
    this.cache.clear();
    this.pending.clear();
  }
}

export const AssetStoreRes = defineResource<AssetStore>('assetStore', () => {
  throw new Error('AssetStore not initialized. Call AssetStoreRes.set(world, new AssetStore(loader, meshopt)).');
});
