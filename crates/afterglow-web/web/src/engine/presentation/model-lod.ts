import * as THREE from 'three/webgpu';
import type { MeshOptimizer } from '../assets/asset-store.ts';

export interface ModelLodBuildOptions {
  readonly ratios: readonly number[];
  readonly targetError: number;
  readonly maxErrorAttributes?: number;
}

export interface ModelGeometryLod {
  readonly geometry: THREE.BufferGeometry;
  readonly ratio: number;
  readonly triangleCount: number;
}

function sourceIndices(geometry: THREE.BufferGeometry): Uint32Array {
  const position = geometry.getAttribute('position');
  if (!position) throw new Error('model LOD requires a position attribute');
  if (geometry.index) {
    const result = new Uint32Array(geometry.index.count);
    for (let index = 0; index < result.length; index++) result[index] = geometry.index.getX(index);
    return result;
  }
  const result = new Uint32Array(position.count);
  for (let index = 0; index < result.length; index++) result[index] = index;
  return result;
}

function positions(geometry: THREE.BufferGeometry): Float32Array {
  const source = geometry.getAttribute('position');
  if (!source || source.itemSize < 3) throw new Error('model LOD requires vec3 positions');
  const result = new Float32Array(source.count * 3);
  for (let index = 0; index < source.count; index++) {
    result[index * 3] = source.getX(index);
    result[index * 3 + 1] = source.getY(index);
    result[index * 3 + 2] = source.getZ(index);
  }
  return result;
}

interface ErrorAttributes {
  readonly values: Float32Array;
  readonly weights: Float32Array;
  readonly stride: number;
  readonly locks: Uint8Array;
}

function appendAttribute(
  output: Float32Array,
  outputStride: number,
  offset: number,
  attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute,
  components: number,
): void {
  for (let vertex = 0; vertex < attribute.count; vertex++) {
    const target = vertex * outputStride + offset;
    if (components > 0) output[target] = attribute.getX(vertex);
    if (components > 1) output[target + 1] = attribute.getY(vertex);
    if (components > 2) output[target + 2] = attribute.getZ(vertex);
    if (components > 3) output[target + 3] = attribute.getW(vertex);
  }
}

/** Build continuous deformation/material attributes used only by meshopt's error metric. */
function errorAttributes(geometry: THREE.BufferGeometry, maxAttributes: number): ErrorAttributes {
  const position = geometry.getAttribute('position');
  if (!position) throw new Error('model LOD requires positions');
  if (!Number.isInteger(maxAttributes) || maxAttributes < 1 || maxAttributes > 16)
    throw new RangeError('meshopt error-attribute capacity must be from 1 through 16');

  const selected: Array<{ attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute; components: number; weight: number }> = [];
  const add = (name: string, components: number, weight: number): void => {
    const attribute = geometry.getAttribute(name);
    if (!attribute || selected.reduce((sum, item) => sum + item.components, 0) + components > maxAttributes) return;
    selected.push({ attribute, components: Math.min(components, attribute.itemSize), weight });
  };
  add('uv', 2, 1);
  add('normal', 3, 0.5);
  const skinIndices = geometry.getAttribute('skinIndex');
  if (skinIndices) {
    add('skinWeight', 4, 2);
    // Joint IDs are discrete, but high attribute weight prevents ordinary
    // collapses from crossing influence sets; coincident discontinuities are
    // additionally locked below.
    add('skinIndex', 4, 4);
  } else {
    add('uv1', 2, 0.5);
    add('tangent', 3, 0.25);
    add('color', 3, 0.25);
  }

  let stride = selected.reduce((sum, item) => sum + item.components, 0);
  const morphPositions = geometry.morphAttributes.position ?? [];
  const includeMorphEnvelope = morphPositions.length !== 0 && stride + 3 <= maxAttributes;
  if (includeMorphEnvelope) stride += 3;
  if (stride === 0) {
    // meshoptimizer requires at least one continuous attribute; a zero channel
    // leaves pure-position simplification behavior unchanged.
    stride = 1;
  }
  const values = new Float32Array(position.count * stride);
  const weights = new Float32Array(stride);
  let offset = 0;
  for (const item of selected) {
    appendAttribute(values, stride, offset, item.attribute, item.components);
    for (let component = 0; component < item.components; component++) weights[offset + component] = item.weight;
    offset += item.components;
  }
  if (includeMorphEnvelope) {
    for (let vertex = 0; vertex < position.count; vertex++) {
      let x = 0, y = 0, z = 0;
      for (const morph of morphPositions) {
        const mx = morph.getX(vertex), my = morph.getY(vertex), mz = morph.getZ(vertex);
        if (Math.abs(mx) > Math.abs(x)) x = mx;
        if (Math.abs(my) > Math.abs(y)) y = my;
        if (Math.abs(mz) > Math.abs(z)) z = mz;
      }
      const target = vertex * stride + offset;
      values[target] = x;
      values[target + 1] = y;
      values[target + 2] = z;
    }
    weights[offset] = 1;
    weights[offset + 1] = 1;
    weights[offset + 2] = 1;
  }

  // Preserve coincident seams that carry different discrete bone sets. Skin
  // weights remain continuous error attributes; joint identifiers do not.
  const locks = new Uint8Array(position.count);
  const joints = skinIndices;
  if (joints) {
    const signatures = new Map<string, { vertex: number; joints: string }>();
    for (let vertex = 0; vertex < position.count; vertex++) {
      const key = `${position.getX(vertex)},${position.getY(vertex)},${position.getZ(vertex)}`;
      const signature = `${joints.getX(vertex)},${joints.getY(vertex)},${joints.getZ(vertex)},${joints.getW(vertex)}`;
      const previous = signatures.get(key);
      if (previous && previous.joints !== signature) {
        locks[previous.vertex] = 1;
        locks[vertex] = 1;
      } else if (!previous) signatures.set(key, { vertex, joints: signature });
    }
  }
  return { values, weights, stride: stride * 4, locks };
}

interface MaterialGroup {
  readonly start: number;
  readonly count: number;
  readonly materialIndex: number;
}

function groupRanges(geometry: THREE.BufferGeometry, indexCount: number): readonly MaterialGroup[] {
  if (geometry.groups.length !== 0) return geometry.groups.map(group => ({
    start: group.start,
    count: group.count,
    materialIndex: group.materialIndex ?? 0,
  }));
  return [{ start: 0, count: indexCount, materialIndex: 0 }];
}

async function simplifyGroups(
  optimizer: MeshOptimizer,
  geometry: THREE.BufferGeometry,
  indices: Uint32Array,
  positionData: Float32Array,
  attributes: ErrorAttributes,
  ratio: number,
  targetError: number,
): Promise<{ indices: Uint32Array; groups: MaterialGroup[] }> {
  const output: number[] = [];
  const groups: MaterialGroup[] = [];
  for (const group of groupRanges(geometry, indices.length)) {
    if (group.start % 3 !== 0 || group.count % 3 !== 0 || group.start + group.count > indices.length)
      throw new Error('model LOD material groups must contain complete triangles');
    const source = indices.slice(group.start, group.start + group.count);
    const target = Math.max(3, Math.floor((source.length * ratio) / 3) * 3);
    const simplified = ratio >= 1
      ? await optimizer.optimizeVertexCache(source, positionData.length / 3)
      : optimizer.simplifyWithAttributes
        ? await optimizer.simplifyWithAttributes(
            source, positionData, 12, attributes.values, attributes.stride,
            attributes.weights, attributes.locks, target, targetError,
          )
        : await optimizer.simplifyWithUvs(
            source, positionData, 12,
            geometry.getAttribute('uv')
              ? Float32Array.from((geometry.getAttribute('uv').array as ArrayLike<number>))
              : new Float32Array((positionData.length / 3) * 2),
            8, 1, target, targetError,
          );
    const cacheOptimized = await optimizer.optimizeVertexCache(simplified, positionData.length / 3);
    const optimized = await optimizer.optimizeOverdraw(cacheOptimized, positionData, 12, 1.05);
    const start = output.length;
    for (const index of optimized) output.push(index);
    groups.push({ start, count: optimized.length, materialIndex: group.materialIndex });
  }
  return { indices: Uint32Array.from(output), groups };
}

type AttributeArray = Float32Array | Uint32Array | Int32Array | Uint16Array |
  Int16Array | Uint8Array | Int8Array;

function cloneAttributeSubset(
  source: THREE.BufferAttribute | THREE.InterleavedBufferAttribute,
  oldByNew: Uint32Array,
): THREE.BufferAttribute {
  const ArrayType = (source.array.constructor as { new(length: number): AttributeArray });
  const output = new ArrayType(oldByNew.length * source.itemSize);
  for (let targetVertex = 0; targetVertex < oldByNew.length; targetVertex++) {
    const sourceVertex = oldByNew[targetVertex] ?? 0;
    const target = targetVertex * source.itemSize;
    if (source.itemSize > 0) output[target] = source.getX(sourceVertex);
    if (source.itemSize > 1) output[target + 1] = source.getY(sourceVertex);
    if (source.itemSize > 2) output[target + 2] = source.getZ(sourceVertex);
    if (source.itemSize > 3) output[target + 3] = source.getW(sourceVertex);
  }
  return new THREE.BufferAttribute(output, source.itemSize, source.normalized);
}

function compactGeometry(
  source: THREE.BufferGeometry,
  sourceIndices: Uint32Array,
  groups: readonly MaterialGroup[],
): THREE.BufferGeometry {
  const position = source.getAttribute('position');
  if (!position) throw new Error('model LOD requires positions');
  const newByOld = new Int32Array(position.count);
  newByOld.fill(-1);
  const old = new Uint32Array(Math.min(position.count, sourceIndices.length));
  const indices = new Uint32Array(sourceIndices.length);
  let vertexCount = 0;
  for (let index = 0; index < sourceIndices.length; index++) {
    const sourceVertex = sourceIndices[index] ?? 0;
    let targetVertex = newByOld[sourceVertex] ?? -1;
    if (targetVertex < 0) {
      targetVertex = vertexCount;
      newByOld[sourceVertex] = targetVertex;
      old[vertexCount++] = sourceVertex;
    }
    indices[index] = targetVertex;
  }
  const oldByNew = old.slice(0, vertexCount);
  const geometry = new THREE.BufferGeometry();
  geometry.setIndex(new THREE.BufferAttribute(indices, 1));
  for (const name of Object.keys(source.attributes)) {
    const attribute = source.getAttribute(name);
    if (attribute) geometry.setAttribute(name, cloneAttributeSubset(attribute, oldByNew));
  }
  const sourceMorphs = source.morphAttributes as Record<string, Array<THREE.BufferAttribute | THREE.InterleavedBufferAttribute> | undefined>;
  const targetMorphs = geometry.morphAttributes as Record<string, THREE.BufferAttribute[]>;
  for (const [name, morphs] of Object.entries(sourceMorphs)) {
    if (morphs) targetMorphs[name] = morphs.map(morph => cloneAttributeSubset(morph, oldByNew));
  }
  geometry.morphTargetsRelative = source.morphTargetsRelative;
  for (const group of groups) geometry.addGroup(group.start, group.count, group.materialIndex);
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();
  return geometry;
}

/**
 * Generate compact LOD geometry for rigid, skinned, and morphed primitives.
 * Every retained vertex attribute and morph target follows the same remap.
 */
export async function buildModelGeometryLods(
  geometry: THREE.BufferGeometry,
  optimizer: MeshOptimizer,
  options: Readonly<ModelLodBuildOptions>,
): Promise<readonly ModelGeometryLod[]> {
  if (options.ratios.length < 1 || options.ratios[0] !== 1 ||
      options.ratios.some((ratio, index) => !(ratio > 0 && ratio <= 1) ||
        (index > 0 && ratio >= (options.ratios[index - 1] ?? 0))))
    throw new RangeError('model LOD ratios must begin at one and strictly descend');
  if (!(options.targetError > 0)) throw new RangeError('model LOD target error must be positive');
  const indices = sourceIndices(geometry);
  if (indices.length % 3 !== 0) throw new Error('model LOD source must be a triangle list');
  const positionData = positions(geometry);
  const hasMorphs = Object.values(geometry.morphAttributes).some(morphs => (morphs?.length ?? 0) !== 0);
  if ((geometry.getAttribute('skinIndex') || hasMorphs) && !optimizer.simplifyWithAttributes)
    throw new Error('rigged and morphed LODs require attribute-aware meshoptimizer support');
  const attributes = errorAttributes(geometry, options.maxErrorAttributes ?? 16);
  const levels: ModelGeometryLod[] = [];
  try {
    for (const ratio of options.ratios) {
      const simplified = await simplifyGroups(
        optimizer, geometry, indices, positionData, attributes, ratio, options.targetError,
      );
      const compact = compactGeometry(geometry, simplified.indices, simplified.groups);
      levels.push({ geometry: compact, ratio, triangleCount: simplified.indices.length / 3 });
    }
    return levels;
  } catch (error) {
    for (const level of levels) level.geometry.dispose();
    throw error;
  }
}

/** One model primitive whose LOD meshes share material, skeleton, and animation state. */
export class ModelLodBinding {
  readonly meshes: readonly THREE.Mesh[];
  private selected = 0;
  private disposed = false;

  constructor(
    source: THREE.Mesh,
    levels: readonly ModelGeometryLod[],
    readonly thresholds: Float32Array,
    private readonly hysteresis: number,
    private readonly ownsGeometries = true,
  ) {
    if (levels.length < 1 || thresholds.length !== levels.length - 1)
      throw new RangeError('model LOD thresholds must separate every level');
    if (!(hysteresis >= 0 && hysteresis < 1)) throw new RangeError('model LOD hysteresis must be in [0, 1)');
    for (let index = 0; index < thresholds.length; index++) {
      const threshold = thresholds[index] ?? 0;
      if (!(threshold > 0) || (index > 0 && threshold >= (thresholds[index - 1] ?? 0)))
        throw new RangeError('model LOD thresholds must be positive and strictly descending');
    }
    const meshes: THREE.Mesh[] = [];
    for (let level = 0; level < levels.length; level++) {
      const mesh = source.clone(false) as THREE.Mesh;
      mesh.geometry = levels[level]!.geometry;
      mesh.material = source.material;
      mesh.visible = level === 0;
      if ((source as THREE.SkinnedMesh).isSkinnedMesh) {
        const sourceSkin = source as THREE.SkinnedMesh;
        const skin = mesh as THREE.SkinnedMesh;
        skin.bind(sourceSkin.skeleton, sourceSkin.bindMatrix);
        skin.bindMode = sourceSkin.bindMode;
      }
      mesh.morphTargetInfluences = source.morphTargetInfluences;
      mesh.morphTargetDictionary = source.morphTargetDictionary;
      meshes.push(mesh);
    }
    this.meshes = meshes;
  }

  /** @alloc-effect none */
  select(coverage: number): number {
    while (this.selected > 0) {
      const boundary = this.thresholds[this.selected - 1] ?? Number.POSITIVE_INFINITY;
      if (coverage < boundary * (1 + this.hysteresis)) break;
      this.selected--;
    }
    while (this.selected < this.meshes.length - 1) {
      const boundary = this.thresholds[this.selected] ?? 0;
      if (coverage >= boundary * (1 - this.hysteresis)) break;
      this.selected++;
    }
    for (let level = 0; level < this.meshes.length; level++) this.meshes[level]!.visible = level === this.selected;
    return this.selected;
  }

  /** @alloc-effect none */
  level(): number { return this.selected; }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.ownsGeometries)
      for (const mesh of this.meshes) mesh.geometry.dispose();
  }
}
