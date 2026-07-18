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

import { Resource, defineResource } from '../core/resource.ts';
import { AssetHandle } from './asset-handle.ts';
import { fallbackGroup } from '../renderer/fallback.ts';
import type { VirtualTextureStore } from '../virtual-texturing/virtual-texture.ts';

// --- interfaces (match the generated client APIs) -----------------------

export interface AssetLoader {
  load(path: string): Promise<Uint8Array>;
  size(path: string): Promise<number>;
  read(path: string, offset: number, len: number): Promise<Uint8Array>;
  poll(): void;
}

export type AssetParser<T> = (bytes: Uint8Array) => Promise<T> | T;

export interface MeshOptimizer {
  optimizeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Uint32Array>;
  optimizeOverdraw(indices: Uint32Array, positions: Float32Array, stride: number, threshold: number): Promise<Uint32Array>;
  simplifyWithUvs(indices: Uint32Array, positions: Float32Array, posStride: number, uvs: Float32Array, uvStride: number, uvWeight: number, targetIndexCount: number, targetError: number): Promise<Uint32Array>;
  analyzeVertexCache(indices: Uint32Array, vertexCount: number): Promise<Float32Array>;
  encodeIndexBuffer(indices: Uint32Array, vertexCount: number): Promise<Uint8Array>;
  poll(): void;
}

// --- constants -----------------------------------------------------------

const MAX_SINGLE_LOAD = 1 << 20;
const CHUNK_SIZE = 512 * 1024;
const DEFAULT_LOD_RATIOS = [1.0, 0.5, 0.25, 0.1];
const DEFAULT_TARGET_ERROR = 0.02;
const DEFAULT_ASSET_CAPACITY = 1024;

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

export interface SceneMeshOptimizationStats extends MeshStats {
  name: string;
  vertexCount: number;
  skinned: boolean;
  preservedAttributes: string[];
}

export interface GltfTextureSamplingLayout {
  image: number;
  texCoord: number;
  offset: readonly [number, number];
  rotation: number;
  scale: readonly [number, number];
  wrapS: number;
  wrapT: number;
  minFilter: number;
  magFilter: number;
}

export interface GltfMaterialTextureLayout {
  index: number;
  name: string;
  baseColorImage: number | null;
  metallicRoughnessImage: number | null;
  normalImage: number | null;
  emissiveImage: number | null;
  baseColorSampling: GltfTextureSamplingLayout | null;
  metallicRoughnessSampling: GltfTextureSamplingLayout | null;
  normalSampling: GltfTextureSamplingLayout | null;
  emissiveSampling: GltfTextureSamplingLayout | null;
}

export interface OptimizedGltfAsset extends ParsedGLTF {
  meshOptimization: SceneMeshOptimizationStats[];
  materialTextures: GltfMaterialTextureLayout[];
}

// --- parsers -------------------------------------------------------------

export async function parseTexture(bytes: Uint8Array): Promise<THREE.Texture> {
  const bitmap = await createImageBitmap(new Blob([bytes]));
  const tex = new THREE.Texture(bitmap);
  tex.needsUpdate = true;
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

export interface ParsedGLTF {
  scene: THREE.Group;
  animations: THREE.AnimationClip[];
  /** Stable glTF material indices recovered from GLTFParser associations. */
  materialIndices: ReadonlyMap<THREE.Material, number>;
  dispose(): void;
}

interface LoaderGLTFResult {
  scene: THREE.Group;
  animations: THREE.AnimationClip[];
  parser?: { associations?: Map<object, { materials?: number }> };
}

export async function parseGLTFAsset(
  bytes: Uint8Array,
  loader?: { parse(data: ArrayBuffer, path: string, onLoad: (r: LoaderGLTFResult) => void, onError: (e: unknown) => void): void },
): Promise<ParsedGLTF> {
  const buf = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buf).set(bytes);
  if (!loader) throw new Error('parseGLTFAsset requires an injected Three.js GLTFLoader');
  return new Promise((resolve, reject) => loader.parse(buf, '', result => {
    try {
      const materialIndices = new Map<THREE.Material, number>();
      let materialCount = 0;
      result.scene.traverse((object) => {
        if (!(object instanceof THREE.Mesh)) return;
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        for (const material of materials) {
          materialCount++;
          const index = result.parser?.associations?.get(material)?.materials;
          if (index !== undefined) materialIndices.set(material, index);
        }
      });
      if (materialCount > 0 && materialIndices.size === 0)
        throw new Error('GLTFLoader parser associations did not expose stable material indices');
      resolve({
        scene: result.scene,
        animations: result.animations,
        materialIndices,
        dispose(): void {
          result.scene.traverse((object) => {
            if (!(object instanceof THREE.Mesh)) return;
            object.geometry.dispose();
            const materials = Array.isArray(object.material) ? object.material : [object.material];
            for (const material of materials) material.dispose();
          });
        },
      });
    } catch (error) {
      reject(error);
    }
  }, reject));
}

export function parseGlbMaterialTextures(bytes: Uint8Array): GltfMaterialTextureLayout[] {
  if (bytes.byteLength < 20 || new DataView(bytes.buffer, bytes.byteOffset, 4).getUint32(0, true) !== 0x46546c67)
    throw new Error('material metadata requires a GLB payload');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const jsonLength = view.getUint32(12, true);
  const jsonType = view.getUint32(16, true);
  if (jsonType !== 0x4e4f534a || 20 + jsonLength > bytes.byteLength) throw new Error('GLB has no valid JSON chunk');
  const document = JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + jsonLength)).replace(/[\\0 ]+$/, ''));
  type TextureInfoDocument = {
    index?: number;
    texCoord?: number;
    extensions?: { KHR_texture_transform?: {
      texCoord?: number; offset?: number[]; rotation?: number; scale?: number[];
    } };
  };
  const virtualMetadata = document.extensions?.AFTERGLOW_virtual_textures;
  const textures = virtualMetadata?.textures ?? document.textures ?? [];
  const samplers = virtualMetadata?.samplers ?? document.samplers ?? [];
  const materials = virtualMetadata?.materials ?? document.materials ?? [];
  const sampling = (info: TextureInfoDocument | undefined): GltfTextureSamplingLayout | null => {
    if (info?.index === undefined) return null;
    const texture = textures[info.index];
    const source = texture?.source;
    if (!Number.isSafeInteger(source) || source < 0) return null;
    const sampler = samplers[texture.sampler] ?? {};
    const transform = info.extensions?.KHR_texture_transform ?? {};
    const offset = transform.offset ?? [0, 0];
    const scale = transform.scale ?? [1, 1];
    return {
      image: source,
      texCoord: transform.texCoord ?? info.texCoord ?? 0,
      offset: [offset[0] ?? 0, offset[1] ?? 0],
      rotation: transform.rotation ?? 0,
      scale: [scale[0] ?? 1, scale[1] ?? 1],
      wrapS: sampler.wrapS ?? 10497,
      wrapT: sampler.wrapT ?? 10497,
      minFilter: sampler.minFilter ?? 9987,
      magFilter: sampler.magFilter ?? 9729,
    };
  };
  return materials.map((material: any, index: number) => {
    const baseColorSampling = sampling(material.pbrMetallicRoughness?.baseColorTexture);
    const metallicRoughnessSampling = sampling(material.pbrMetallicRoughness?.metallicRoughnessTexture);
    const normalSampling = sampling(material.normalTexture);
    const emissiveSampling = sampling(material.emissiveTexture);
    return {
      index,
      name: material.name ?? `material-${index}`,
      baseColorImage: baseColorSampling?.image ?? null,
      metallicRoughnessImage: metallicRoughnessSampling?.image ?? null,
      normalImage: normalSampling?.image ?? null,
      emissiveImage: emissiveSampling?.image ?? null,
      baseColorSampling,
      metallicRoughnessSampling,
      normalSampling,
      emissiveSampling,
    };
  });
}

export async function parseGLTF(
  bytes: Uint8Array,
  loader?: Parameters<typeof parseGLTFAsset>[1],
): Promise<THREE.Group> {
  return (await parseGLTFAsset(bytes, loader)).scene;
}

export function parseJSON<T = unknown>(bytes: Uint8Array): T {
  return JSON.parse(new TextDecoder().decode(bytes)) as T;
}

// --- fixed asset state ---------------------------------------------------

export type AssetId = number;

export enum AssetRequestState {
  Free = 0,
  Idle = 1,
  Reading = 2,
  Parsing = 3,
  ReadyToPublish = 4,
  Ready = 5,
  Error = 6,
}

export enum AssetAdmission {
  Started = 0,
  Existing = 1,
  CapacityExceeded = 2,
}

export interface AssetLoadResult<T> {
  status: AssetAdmission;
  id: AssetId;
  handle: AssetHandle<T> | null;
}

interface DisposableAsset { dispose?(): void; }

// --- AssetStore ----------------------------------------------------------

/**
 * Loads assets from disk, optimizes meshes, and delegates all texture
 * loading to the VirtualTextureStore (universal VT).
 *
 * Meshes are optimized by default (vertex cache + overdraw + simplify).
 * If no meshopt worker is provided, meshes load without optimization.
 */
export class AssetStore {
  /** String lookup is confined to registration/game-facing wrappers. */
  private readonly idsByPath = new Map<string, AssetId>();
  private readonly paths: (string | null)[];
  private readonly handles: (AssetHandle<unknown> | null)[];
  private readonly states: Uint8Array;
  private readonly requestTokens: Uint32Array;
  private readonly completionIds: Int32Array;
  private readonly completionTokens: Uint32Array;
  private readonly completionKinds: Uint8Array;
  private readonly completionValues: unknown[];
  private completionHead = 0;
  private completionTail = 0;
  private completionCount = 0;
  private completionHighWater = 0;
  private completionOverflows = 0;
  private assetCount = 0;
  private readyCount = 0;

  /** Optional policy-free mesh processor owned by the caller. */
  private readonly meshopt: MeshOptimizer | undefined;

  private loader: AssetLoader;
  private vtStore: VirtualTextureStore | null = null;

  constructor(
    loader: AssetLoader,
    meshopt?: MeshOptimizer,
    capacity = DEFAULT_ASSET_CAPACITY,
    private readonly maxCompletionsPerPoll = 32,
  ) {
    if (!Number.isInteger(capacity) || capacity <= 0) throw new RangeError('asset capacity must be positive');
    if (!Number.isInteger(maxCompletionsPerPoll) || maxCompletionsPerPoll <= 0)
      throw new RangeError('asset completion limit must be positive');
    this.loader = loader;
    this.meshopt = meshopt;
    this.paths = new Array(capacity).fill(null);
    this.handles = new Array(capacity).fill(null);
    this.states = new Uint8Array(capacity);
    this.requestTokens = new Uint32Array(capacity);
    this.completionIds = new Int32Array(capacity);
    this.completionTokens = new Uint32Array(capacity);
    this.completionKinds = new Uint8Array(capacity);
    this.completionValues = new Array(capacity).fill(null);
  }

  get assetLoader(): AssetLoader { return this.loader; }

  /** Set the VirtualTextureStore — enables universal VT for all textures. */
  setVirtualTextureStore(vt: VirtualTextureStore) { this.vtStore = vt; }

  /** Get the VirtualTextureStore (if set). */
  get virtualTextureStore(): VirtualTextureStore | null { return this.vtStore; }

  /** Drive workers and publish a bounded number of numeric completions. */
  poll(): void {
    this.loader.poll();
    this.meshopt?.poll();
    this.drainCompletions(this.maxCompletionsPerPoll);
  }

  private enqueueCompletion(id: AssetId, token: number, kind: number, value: unknown): void {
    if (this.requestTokens[id] !== token || this.handles[id] === null) {
      if (kind === 1) (value as { dispose?: () => void })?.dispose?.();
      return;
    }
    if (this.completionCount === this.completionIds.length) {
      this.completionOverflows++;
      if (kind === 1) (value as { dispose?: () => void })?.dispose?.();
      this.handles[id]!.state = 'error';
      this.states[id] = AssetRequestState.Error;
      return;
    }
    const slot = this.completionTail;
    this.completionIds[slot] = id;
    this.completionTokens[slot] = token;
    this.completionKinds[slot] = kind;
    this.completionValues[slot] = value;
    this.completionTail = (slot + 1) % this.completionIds.length;
    this.completionCount++;
    if (this.completionCount > this.completionHighWater) this.completionHighWater = this.completionCount;
    this.states[id] = AssetRequestState.ReadyToPublish;
  }

  // @hot-no-alloc-begin AssetStore.drainCompletions
  drainCompletions(limit: number): number {
    let drained = 0;
    while (drained < limit && this.completionCount !== 0) {
      const slot = this.completionHead;
      const id = this.completionIds[slot];
      const token = this.completionTokens[slot];
      const kind = this.completionKinds[slot];
      const value = this.completionValues[slot];
      this.completionValues[slot] = null;
      this.completionHead = (slot + 1) % this.completionIds.length;
      this.completionCount--;
      drained++;
      const handle = this.handles[id];
      if (this.requestTokens[id] !== token || handle === null) {
        if (kind === 1) (value as DisposableAsset)?.dispose?.();
        continue;
      }
      if (kind === 1) {
        handle.asset = value;
        handle.generation++;
        handle.state = 'ready';
        handle.lod = 0;
        this.states[id] = AssetRequestState.Ready;
        this.readyCount++;
      } else {
        handle.state = 'error';
        this.states[id] = AssetRequestState.Error;
        this.reportCompletionError(kind, id, value); // @alloc-allowed reason=DiagnosticFailure
      }
    }
    return drained;
  }
  // @hot-no-alloc-end AssetStore.drainCompletions

  /** Allocation-permitted diagnostic boundary for exceptional failures. */
  private reportCompletionError(kind: number, id: AssetId, value: unknown): void {
    const path = this.paths[id]!;
    console.error(kind === 2 ? `[afterglow] parse failed: ${path}` : `[afterglow] load failed: ${path}`, value);
  }

  /** Register paths during manifest/bootstrap; gameplay can then use AssetId. */
  registerAsset(path: string): AssetId {
    const existing = this.idsByPath.get(path);
    if (existing !== undefined) return existing;
    if (this.assetCount === this.states.length) return -1;
    const id = this.assetCount++;
    this.idsByPath.set(path, id);
    this.paths[id] = path;
    this.states[id] = AssetRequestState.Idle;
    return id;
  }

  private observeLoad<T>(id: AssetId, handle: AssetHandle<T>, parser: AssetParser<T>): void {
    const path = this.paths[id]!;
    const token = ++this.requestTokens[id];
    this.states[id] = AssetRequestState.Reading;
    this.startLoad(path).then(
      async (bytes) => {
        if (this.requestTokens[id] !== token || this.handles[id] !== handle) return;
        this.states[id] = AssetRequestState.Parsing;
        try {
          const asset = await parser(bytes);
          if (this.requestTokens[id] !== token || this.handles[id] !== handle) {
            (asset as { dispose?: () => void })?.dispose?.();
            return;
          }
          this.enqueueCompletion(id, token, 1, asset);
        } catch (err) {
          this.enqueueCompletion(id, token, 2, err);
        }
      },
      (err) => {
        this.enqueueCompletion(id, token, 3, err);
      },
    );
  }

  /** Fixed-table admission. Returns CapacityExceeded instead of growing. */
  tryLoad<T>(path: string, parser: AssetParser<T>, fallback?: T): AssetLoadResult<T> {
    const id = this.registerAsset(path);
    if (id < 0) return { status: AssetAdmission.CapacityExceeded, id, handle: null };
    return this.tryLoadAsset(id, parser, fallback);
  }

  /** Numeric hot-owner API; `id` must have been registered during bootstrap. */
  tryLoadAsset<T>(id: AssetId, parser: AssetParser<T>, fallback?: T): AssetLoadResult<T> {
    if (!Number.isInteger(id) || id < 0 || id >= this.assetCount)
      return { status: AssetAdmission.CapacityExceeded, id: -1, handle: null };
    const existing = this.handles[id] as AssetHandle<T> | null;
    const state = this.states[id];
    if (existing && state !== AssetRequestState.Idle && state !== AssetRequestState.Error)
      return { status: AssetAdmission.Existing, id, handle: existing };
    if (state === AssetRequestState.Ready)
      return { status: AssetAdmission.Existing, id, handle: existing };
    const path = this.paths[id]!;
    const handle = new AssetHandle<T>(path, fallback);
    this.handles[id] = handle as AssetHandle<unknown>;
    this.observeLoad(id, handle, parser);
    return { status: AssetAdmission.Started, id, handle };
  }

  /** Game-facing convenience wrapper. Prefer registration + `tryLoadAsset`. */
  load<T>(path: string, parser: AssetParser<T>, fallback?: T): AssetHandle<T> {
    const result = this.tryLoad(path, parser, fallback);
    if (!result.handle) throw new RangeError(`asset capacity exceeded while registering ${path}`);
    return result.handle;
  }

  getHandleById<T>(id: AssetId): AssetHandle<T> | undefined {
    if (id < 0 || id >= this.assetCount || this.states[id] !== AssetRequestState.Ready) return undefined;
    return this.handles[id] as AssetHandle<T> | undefined;
  }

  getHandle<T>(path: string): AssetHandle<T> | undefined {
    const id = this.idsByPath.get(path);
    return id === undefined ? undefined : this.getHandleById<T>(id);
  }

  has(path: string): boolean {
    const id = this.idsByPath.get(path);
    return id !== undefined && this.states[id] === AssetRequestState.Ready;
  }
  isLoading(path: string): boolean {
    const id = this.idsByPath.get(path);
    return id !== undefined && (this.states[id] === AssetRequestState.Reading ||
      this.states[id] === AssetRequestState.Parsing || this.states[id] === AssetRequestState.ReadyToPublish);
  }

  // --- model loading and runtime mesh-worker processing ---

  /**
   * Optimize indexed triangle order in a parsed scene without touching vertex
   * identity. Every vertex attribute, morph target, skin joint/weight, skeleton,
   * bind matrix, animation track, and material group therefore remains valid.
   *
   * LOD simplification is deliberately not performed here: the current worker
   * simplifier does not include skin weights in its error metric.
   */
  async optimizeGltfScene(scene: THREE.Group): Promise<SceneMeshOptimizationStats[]> {
    if (!this.meshopt) throw new Error('optimizeGltfScene requires a meshopt worker');
    const meshes: THREE.Mesh[] = [];
    scene.traverse(object => { if ((object as THREE.Mesh).isMesh) meshes.push(object as THREE.Mesh); });
    const stats: SceneMeshOptimizationStats[] = [];
    for (const mesh of meshes) {
      const geometry = mesh.geometry;
      const position = geometry.getAttribute('position');
      if (!position || position.itemSize < 3) continue;
      let source: Uint32Array;
      if (geometry.index) {
        source = new Uint32Array(geometry.index.count);
        for (let index = 0; index < source.length; index++) source[index] = geometry.index.getX(index);
      } else {
        source = new Uint32Array(position.count);
        for (let index = 0; index < source.length; index++) source[index] = index;
      }
      if (source.length % 3 !== 0) throw new Error(`mesh ${mesh.name} index count is not a triangle list`);
      const positions = new Float32Array(position.count * 3);
      for (let index = 0; index < position.count; index++) {
        const target = index * 3;
        positions[target] = position.getX(index);
        positions[target + 1] = position.getY(index);
        positions[target + 2] = position.getZ(index);
      }
      const original = await this.meshopt.analyzeVertexCache(source, position.count);
      const optimized = source.slice();
      const groups = geometry.groups.length === 0
        ? [{ start: 0, count: source.length }]
        : geometry.groups;
      for (const group of groups) {
        if (group.start % 3 !== 0 || group.count % 3 !== 0 || group.start + group.count > source.length)
          throw new Error(`mesh ${mesh.name} has a non-triangular material group`);
        const groupIndices = source.slice(group.start, group.start + group.count);
        const cacheOptimized = await this.meshopt.optimizeVertexCache(groupIndices, position.count);
        const overdrawOptimized = await this.meshopt.optimizeOverdraw(cacheOptimized, positions, 12, 1.05);
        optimized.set(overdrawOptimized, group.start);
      }
      const optimizedAnalysis = await this.meshopt.analyzeVertexCache(optimized, position.count);
      const compressed = await this.meshopt.encodeIndexBuffer(optimized, position.count);
      geometry.setIndex(new THREE.BufferAttribute(optimized, 1));
      stats.push({
        name: mesh.name,
        vertexCount: position.count,
        skinned: (mesh as THREE.SkinnedMesh).isSkinnedMesh === true,
        preservedAttributes: Object.keys(geometry.attributes),
        originalTriangles: source.length / 3,
        originalAcmr: original[0],
        optimizedAcmr: optimizedAnalysis[0],
        compressedIndexBytes: compressed.byteLength,
        uncompressedIndexBytes: optimized.byteLength,
      });
    }
    return stats;
  }

  /** Load a packed GLB, preserve its complete scene/rig, and optimize it through the runtime mesh worker. */
  loadOptimizedGLTF(
    path: string,
    loader: Parameters<typeof parseGLTFAsset>[1],
  ): AssetHandle<OptimizedGltfAsset> {
    const fallback = {
      scene: fallbackGroup(), animations: [], materialIndices: new Map(),
      meshOptimization: [], materialTextures: [], dispose() {},
    } as OptimizedGltfAsset;
    return this.load(path, async bytes => {
      const materialTextures = parseGlbMaterialTextures(bytes);
      const parsed = await parseGLTFAsset(bytes, loader);
      const meshOptimization = await this.optimizeGltfScene(parsed.scene);
      return { ...parsed, meshOptimization, materialTextures };
    }, fallback);
  }

  loadModel(path: string): AssetHandle<ModelAsset> {
    return this.load(path, (bytes) => this.processModel(bytes, path));
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

  evict(path: string): void {
    const id = this.idsByPath.get(path);
    if (id === undefined) return;
    if (this.states[id] === AssetRequestState.Ready) this.readyCount--;
    this.requestTokens[id]++;
    this.handles[id] = null;
    this.states[id] = AssetRequestState.Idle;
  }
  get size(): number { return this.readyCount; }
  /** Explicit diagnostic snapshot; allocates by design. */
  get cachedPaths(): string[] {
    const result: string[] = [];
    for (let id = 0; id < this.assetCount; id++)
      if (this.states[id] === AssetRequestState.Ready) result.push(this.paths[id]!);
    return result;
  }
  get capacity(): number { return this.states.length; }
  get registeredAssetCount(): number { return this.assetCount; }
  get stateTable(): Uint8Array { return this.states; }
  get pendingCompletionCount(): number { return this.completionCount; }
  get completionQueueHighWater(): number { return this.completionHighWater; }
  get completionQueueOverflows(): number { return this.completionOverflows; }

  dispose(): void {
    while (this.completionCount !== 0) {
      const slot = this.completionHead;
      if (this.completionKinds[slot] === 1)
        (this.completionValues[slot] as { dispose?: () => void })?.dispose?.();
      this.completionValues[slot] = null;
      this.completionHead = (slot + 1) % this.completionIds.length;
      this.completionCount--;
    }
    this.completionHead = 0;
    this.completionTail = 0;
    for (let id = 0; id < this.assetCount; id++) {
      const handle = this.handles[id];
      (handle?.asset as { dispose?: () => void })?.dispose?.();
      this.requestTokens[id]++;
      this.handles[id] = null;
      this.paths[id] = null;
      this.states[id] = AssetRequestState.Free;
    }
    this.idsByPath.clear();
    this.assetCount = 0;
    this.readyCount = 0;
  }
}

export const AssetStoreRes = defineResource<AssetStore>('assetStore', () => {
  throw new Error('AssetStore not initialized. Call AssetStoreRes.set(world, new AssetStore(loader, meshopt)).');
});
