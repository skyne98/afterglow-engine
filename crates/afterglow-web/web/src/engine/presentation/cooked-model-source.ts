import * as THREE from 'three/webgpu';
import { readBigHeader, type FetchRangeLoader } from '../assets/asset-range.ts';
import type { ChunkInfo } from '../assets/big-format.ts';
import { createPlatformRangeLoader } from '../assets/platform-range-loader.ts';
import { createPlatformMeshOptimizer } from '../assets/platform-workers.ts';

export interface MeshoptVertexDecoder {
  decodeVertexBuffer(buffer: Uint8Array, vertexCount: number, vertexSize: number): Promise<Uint8Array>;
}
export interface OwnedMeshoptVertexDecoder extends MeshoptVertexDecoder {
  close(): void | Promise<void>;
}
export interface CookedModelLoadOptions {
  containerPath: string;
  assetName: string;
  maxHeaderBytes: number;
  source?: FetchRangeLoader;
  /** Test/platform injection; games use the engine-owned default worker. */
  createDecoder?(): Promise<OwnedMeshoptVertexDecoder>;
}
export interface CookedModelLevel {
  readonly geometry: THREE.BufferGeometry;
  readonly triangleCount: number;
}

/** Decoded cooked model LODs. Worker lifetime ends before this asset is returned. */
export class CookedModelAsset {
  private disposed = false;
  private ownsLevels = true;
  constructor(readonly levels: readonly CookedModelLevel[]) {}
  /** Transfer decoded levels into ModelSystem without duplicate ownership. */
  takeLevels(): readonly CookedModelLevel[] {
    if (this.disposed || !this.ownsLevels) throw new Error('cooked model levels already released');
    this.ownsLevels = false;
    return this.levels;
  }
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.ownsLevels) for (const level of this.levels) level.geometry.dispose();
    this.ownsLevels = false;
  }
}

async function createDefaultDecoder(): Promise<OwnedMeshoptVertexDecoder> {
  const optimizer = await createPlatformMeshOptimizer();
  if (!optimizer.decodeVertexBuffer) {
    await optimizer.close();
    throw new Error('platform mesh processor does not expose vertex decoding');
  }
  return optimizer as unknown as OwnedMeshoptVertexDecoder;
}

/** Load one offline-optimized model and release its bootstrap decoder. */
export async function loadCookedModel(options: CookedModelLoadOptions): Promise<CookedModelAsset> {
  if (!options.containerPath || !options.assetName) throw new Error('static mesh source and asset names are required');
  const source = options.source ?? createPlatformRangeLoader();
  const header = await readBigHeader(source, options.containerPath, options.maxHeaderBytes);
  const entry = header.assets.find((candidate) => candidate.name === options.assetName);
  if (!entry || entry.assetType !== 'Mesh') throw new Error(`static mesh asset not found: ${options.assetName}`);
  if (entry.chunks.length < 2) throw new Error('static mesh asset requires at least two levels');
  const chunks = entry.chunks.slice().sort((left, right) => left.lodLevel - right.lodLevel);
  for (let index = 0; index < chunks.length; index++) {
    if (chunks[index]?.lodLevel !== index) throw new Error('static mesh levels must be contiguous');
  }
  const decoder = await (options.createDecoder ?? createDefaultDecoder)();
  const levels: CookedModelLevel[] = [];
  let closeAttempted = false;
  try {
    for (const chunk of chunks) levels.push(await decodeLevel(source, options.containerPath, chunk, decoder));
    closeAttempted = true;
    await decoder.close();
    return new CookedModelAsset(levels);
  } catch (error) {
    for (const level of levels) level.geometry.dispose();
    if (!closeAttempted) {
      try { await decoder.close(); }
      catch (closeError) { if (error instanceof Error && error.cause === undefined) error.cause = closeError; }
    }
    throw error;
  }
}

async function decodeLevel(
  source: FetchRangeLoader, containerPath: string, chunk: ChunkInfo, decoder: MeshoptVertexDecoder,
): Promise<CookedModelLevel> {
  if (chunk.meta.type !== 'Mesh' || chunk.compression !== 'Meshopt')
    throw new Error('static LOD chunk has invalid metadata');
  const size = Number(chunk.uncompressedSize);
  const compressedSize = Number(chunk.compressedSize);
  const offset = Number(chunk.offset);
  if (!Number.isSafeInteger(size) || !Number.isSafeInteger(compressedSize) ||
      !Number.isSafeInteger(offset) || offset < 0)
    throw new RangeError('static LOD chunk exceeds browser safe size');
  const compressed = await source.read(containerPath, offset, compressedSize);
  const decoded = await decoder.decodeVertexBuffer(compressed, Math.ceil(size / 4), 4);
  if (decoded.byteLength < size) throw new Error('static LOD decoder returned a truncated chunk');
  // RPC postcard payloads may begin at an unaligned byte offset. Retain one
  // aligned owned copy so index/f32 views are valid for the geometry lifetime.
  const payload = new Uint8Array(size);
  payload.set(decoded.subarray(0, size));
  const data = new DataView(payload.buffer);
  const indexCount = data.getUint32(0, true);
  const expectedIndices = chunk.meta.indexCount ?? 0;
  const vertexCount = chunk.meta.vertexCount ?? 0;
  const positionStride = chunk.meta.positionStride ?? 0;
  const uvStride = chunk.meta.uvStride ?? 0;
  if (indexCount !== expectedIndices || positionStride !== 12 || uvStride !== 8)
    throw new Error('static LOD chunk layout is unsupported');
  const indicesOffset = 4;
  const positionsOffset = indicesOffset + indexCount * 4;
  const uvsOffset = positionsOffset + vertexCount * positionStride;
  if (uvsOffset + vertexCount * uvStride > size) throw new Error('static LOD chunk layout exceeds payload');
  const indices = new Uint32Array(payload.buffer, indicesOffset, indexCount);
  const positions = new Float32Array(payload.buffer, positionsOffset, vertexCount * 3);
  const uvs = new Float32Array(payload.buffer, uvsOffset, vertexCount * 2);
  const geometry = new THREE.BufferGeometry();
  geometry.setIndex(new THREE.BufferAttribute(indices, 1));
  geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute('uv', new THREE.BufferAttribute(uvs, 2));
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
  return { geometry, triangleCount: indexCount / 3 };
}

/** @alloc-effect none */
export function projectedCoverage(radius: number, distance: number, verticalFovRadians: number): number {
  if (!(radius > 0) || !(distance > 0) || !(verticalFovRadians > 0)) return 0;
  return Math.min(1, (2 * radius) / (distance * 2 * Math.tan(verticalFovRadians * 0.5)));
}
