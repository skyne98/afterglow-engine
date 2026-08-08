import * as THREE from 'three';
import { EngineMetric, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import type { ModelGeometryLod } from './model-lod.ts';

export type GeometryArrayKind =
  | 'f32' | 'u32' | 'u16' | 'u8' | 'i32' | 'i16' | 'i8';

export interface GeometryAttributeLayout {
  readonly name: string;
  readonly itemSize: number;
  readonly kind: GeometryArrayKind;
  readonly normalized?: boolean;
}

export interface GeometryMorphLayout extends GeometryAttributeLayout {
  /** Exact target count keeps Three shader/binding layouts prewarmable. */
  readonly targets: number;
}

export interface GeometryArenaBucketConfig {
  readonly slots: number;
  readonly maxVertices: number;
  readonly maxIndices: number;
  readonly maxGroups: number;
  readonly indexKind: 'u16' | 'u32';
  readonly attributes: readonly GeometryAttributeLayout[];
  readonly morphAttributes: readonly GeometryMorphLayout[];
  readonly label?: string;
}

export interface GeometryArenaOptions {
  readonly buckets: readonly GeometryArenaBucketConfig[];
  readonly telemetry?: EngineTelemetry;
}

export interface GeometryArenaStats {
  reservedSlots: number;
  activeSlots: number;
  slotHighWater: number;
  rejectedPublications: number;
  publicationCount: number;
  uploadBytes: number;
  reservedGpuBytes: number;
  activeGpuBytes: number;
  activeGpuByteHighWater: number;
}

type GeometryTypedArray =
  Float32Array | Uint32Array | Uint16Array | Uint8Array |
  Int32Array | Int16Array | Int8Array;

interface GeometryGroupRecord {
  start: number;
  count: number;
  materialIndex: number;
}

interface GeometryArenaSlot {
  readonly bucket: number;
  readonly slot: number;
  generation: number;
  active: boolean;
  activeBytes: number;
  readonly geometry: THREE.BufferGeometry;
  readonly index: THREE.BufferAttribute;
  readonly attributes: readonly THREE.BufferAttribute[];
  readonly morphAttributes: readonly (readonly THREE.BufferAttribute[])[];
  readonly groups: GeometryGroupRecord[];
}

export interface GeometryArenaLevel extends ModelGeometryLod {
  readonly arenaBucket: number;
  readonly arenaSlot: number;
  readonly arenaGeneration: number;
}

export interface GeometryArenaPublication {
  readonly levels: readonly GeometryArenaLevel[];
  readonly activeBytes: number;
  released: boolean;
}

function makeArray(kind: GeometryArrayKind, length: number): GeometryTypedArray {
  switch (kind) {
    case 'f32': return new Float32Array(length);
    case 'u32': return new Uint32Array(length);
    case 'u16': return new Uint16Array(length);
    case 'u8': return new Uint8Array(length);
    case 'i32': return new Int32Array(length);
    case 'i16': return new Int16Array(length);
    case 'i8': return new Int8Array(length);
  }
}

function arrayKind(array: ArrayLike<number>): GeometryArrayKind | null {
  if (array instanceof Float32Array) return 'f32';
  if (array instanceof Uint32Array) return 'u32';
  if (array instanceof Uint16Array) return 'u16';
  if (array instanceof Uint8Array || array instanceof Uint8ClampedArray) return 'u8';
  if (array instanceof Int32Array) return 'i32';
  if (array instanceof Int16Array) return 'i16';
  if (array instanceof Int8Array) return 'i8';
  return null;
}

function markUpdated(attribute: THREE.BufferAttribute, componentCount: number): void {
  attribute.clearUpdateRanges();
  attribute.addUpdateRange(0, componentCount);
  attribute.needsUpdate = true;
}

function attributeBytes(layout: GeometryAttributeLayout, vertices: number): number {
  return makeArray(layout.kind, 0).BYTES_PER_ELEMENT * layout.itemSize * vertices;
}

function slotBytes(config: Readonly<GeometryArenaBucketConfig>): number {
  let bytes = config.maxIndices * (config.indexKind === 'u16' ? 2 : 4);
  for (const attribute of config.attributes) bytes += attributeBytes(attribute, config.maxVertices);
  for (const morph of config.morphAttributes)
    bytes += attributeBytes(morph, config.maxVertices) * morph.targets;
  return bytes;
}

/** Fixed Three-compatible geometry slots. All arrays/geometries exist before seal. */
export class GeometryArena {
  private readonly configs: readonly GeometryArenaBucketConfig[];
  private readonly slots: GeometryArenaSlot[][];
  private readonly free: Uint32Array[];
  private readonly freeTop: Uint32Array;
  private readonly stats: GeometryArenaStats = {
    reservedSlots: 0, activeSlots: 0, slotHighWater: 0,
    rejectedPublications: 0, publicationCount: 0, uploadBytes: 0,
    reservedGpuBytes: 0, activeGpuBytes: 0, activeGpuByteHighWater: 0,
  };
  private disposed = false;

  constructor(private readonly options: Readonly<GeometryArenaOptions>) {
    if (options.buckets.length === 0) throw new RangeError('geometry arena requires a bucket');
    this.configs = options.buckets.map(config => ({
      ...config,
      attributes: config.attributes.map(attribute => ({ ...attribute })),
      morphAttributes: config.morphAttributes.map(attribute => ({ ...attribute })),
    }));
    this.slots = new Array(this.configs.length);
    this.free = new Array(this.configs.length);
    this.freeTop = new Uint32Array(this.configs.length);
    for (let bucket = 0; bucket < this.configs.length; bucket++) {
      const config = this.configs[bucket]!;
      this.validateConfig(config);
      const slots: GeometryArenaSlot[] = new Array(config.slots);
      const free = new Uint32Array(config.slots);
      for (let slot = 0; slot < config.slots; slot++) {
        slots[slot] = this.createSlot(bucket, slot, config);
        free[slot] = config.slots - slot - 1;
      }
      this.slots[bucket] = slots;
      this.free[bucket] = free;
      this.freeTop[bucket] = config.slots;
      this.stats.reservedSlots += config.slots;
      this.stats.reservedGpuBytes += config.slots * slotBytes(config);
    }
  }

  private validateConfig(config: Readonly<GeometryArenaBucketConfig>): void {
    if (!Number.isInteger(config.slots) || config.slots < 1 ||
        !Number.isInteger(config.maxVertices) || config.maxVertices < 1 ||
        !Number.isInteger(config.maxIndices) || config.maxIndices < 3 ||
        !Number.isInteger(config.maxGroups) || config.maxGroups < 1)
      throw new RangeError('invalid geometry arena bucket capacity');
    const names = new Set<string>();
    for (const attribute of config.attributes) {
      if (!attribute.name || names.has(attribute.name) ||
          !Number.isInteger(attribute.itemSize) || attribute.itemSize < 1 || attribute.itemSize > 4)
        throw new RangeError('invalid geometry arena attribute layout');
      names.add(attribute.name);
    }
    if (!names.has('position')) throw new RangeError('geometry arena bucket requires position');
    for (const morph of config.morphAttributes) {
      if (!morph.name || !Number.isInteger(morph.targets) || morph.targets < 1 ||
          !Number.isInteger(morph.itemSize) || morph.itemSize < 1 || morph.itemSize > 4)
        throw new RangeError('invalid geometry arena morph layout');
    }
  }

  private createSlot(
    bucket: number,
    slot: number,
    config: Readonly<GeometryArenaBucketConfig>,
  ): GeometryArenaSlot {
    const geometry = new THREE.BufferGeometry();
    const indexArray = config.indexKind === 'u16'
      ? new Uint16Array(config.maxIndices)
      : new Uint32Array(config.maxIndices);
    const index = new THREE.BufferAttribute(indexArray, 1, false);
    geometry.setIndex(index);
    const attributes: THREE.BufferAttribute[] = new Array(config.attributes.length);
    for (let index = 0; index < config.attributes.length; index++) {
      const layout = config.attributes[index]!;
      const attribute = new THREE.BufferAttribute(
        makeArray(layout.kind, config.maxVertices * layout.itemSize),
        layout.itemSize,
        layout.normalized ?? false,
      );
      geometry.setAttribute(layout.name, attribute);
      attributes[index] = attribute;
    }
    const morphAttributes: THREE.BufferAttribute[][] = new Array(config.morphAttributes.length);
    for (let attributeIndex = 0; attributeIndex < config.morphAttributes.length; attributeIndex++) {
      const layout = config.morphAttributes[attributeIndex]!;
      const targets: THREE.BufferAttribute[] = new Array(layout.targets);
      for (let target = 0; target < layout.targets; target++) {
        targets[target] = new THREE.BufferAttribute(
          makeArray(layout.kind, config.maxVertices * layout.itemSize),
          layout.itemSize,
          layout.normalized ?? false,
        );
      }
      (geometry.morphAttributes as Record<string, THREE.BufferAttribute[] | undefined>)[layout.name] = targets;
      morphAttributes[attributeIndex] = targets;
    }
    const groups: GeometryGroupRecord[] = new Array(config.maxGroups);
    for (let group = 0; group < groups.length; group++)
      groups[group] = { start: 0, count: 0, materialIndex: 0 };
    geometry.boundingBox = new THREE.Box3();
    geometry.boundingSphere = new THREE.Sphere();
    geometry.setDrawRange(0, 0);
    return {
      bucket, slot, generation: 0, active: false, activeBytes: 0,
      geometry, index, attributes, morphAttributes, groups,
    };
  }

  private compatibleBucket(geometry: THREE.BufferGeometry): number {
    const index = geometry.index;
    const position = geometry.getAttribute('position');
    if (!index || !position) return -1;
    for (let bucket = 0; bucket < this.configs.length; bucket++) {
      const config = this.configs[bucket]!;
      if ((this.freeTop[bucket] ?? 0) === 0 || position.count > config.maxVertices ||
          index.count > config.maxIndices || geometry.groups.length > config.maxGroups ||
          arrayKind(index.array) !== config.indexKind) continue;
      const sourceNames = Object.keys(geometry.attributes);
      if (sourceNames.length !== config.attributes.length) continue;
      let compatible = true;
      for (const layout of config.attributes) {
        const attribute = geometry.getAttribute(layout.name);
        if (!(attribute instanceof THREE.BufferAttribute) || attribute.count !== position.count || attribute.itemSize !== layout.itemSize ||
            attribute.normalized !== (layout.normalized ?? false) || arrayKind(attribute.array) !== layout.kind) {
          compatible = false;
          break;
        }
      }
      if (!compatible) continue;
      const sourceMorphs = geometry.morphAttributes as Record<string, THREE.BufferAttribute[] | undefined>;
      const morphNames = Object.keys(sourceMorphs).filter(name =>
        (sourceMorphs[name]?.length ?? 0) > 0,
      );
      if (morphNames.length !== config.morphAttributes.length) continue;
      for (const layout of config.morphAttributes) {
        const targets = sourceMorphs[layout.name];
        if (!targets || targets.length !== layout.targets || targets.some((attribute: THREE.BufferAttribute) =>
          attribute.count !== position.count || attribute.itemSize !== layout.itemSize ||
          attribute.normalized !== (layout.normalized ?? false) || arrayKind(attribute.array) !== layout.kind)) {
          compatible = false;
          break;
        }
      }
      if (compatible) return bucket;
    }
    return -1;
  }

  private acquire(bucket: number): GeometryArenaSlot {
    const top = (this.freeTop[bucket] ?? 0) - 1;
    this.freeTop[bucket] = top;
    const slotIndex = this.free[bucket]![top] ?? 0;
    const slot = this.slots[bucket]![slotIndex]!;
    slot.generation = (slot.generation + 1) >>> 0 || 1;
    slot.active = true;
    this.stats.activeSlots++;
    if (this.stats.activeSlots > this.stats.slotHighWater)
      this.stats.slotHighWater = this.stats.activeSlots;
    return slot;
  }

  private copyInto(slot: GeometryArenaSlot, source: THREE.BufferGeometry): number {
    const config = this.configs[slot.bucket]!;
    const startedAt = performance.now();
    const sourceIndex = source.index!;
    (slot.index.array as GeometryTypedArray).set(sourceIndex.array as GeometryTypedArray, 0);
    markUpdated(slot.index, sourceIndex.count);
    let activeBytes = sourceIndex.array.byteLength;
    for (let index = 0; index < config.attributes.length; index++) {
      const layout = config.attributes[index]!;
      const sourceAttribute = source.getAttribute(layout.name);
      const target = slot.attributes[index]!;
      (target.array as GeometryTypedArray).set(sourceAttribute.array as GeometryTypedArray, 0);
      markUpdated(target, sourceAttribute.count * sourceAttribute.itemSize);
      activeBytes += sourceAttribute.array.byteLength;
    }
    for (let attributeIndex = 0; attributeIndex < config.morphAttributes.length; attributeIndex++) {
      const layout = config.morphAttributes[attributeIndex]!;
      const sourceTargets = (source.morphAttributes as Record<string, THREE.BufferAttribute[] | undefined>)[layout.name]!;
      const targetTargets = slot.morphAttributes[attributeIndex]!;
      for (let target = 0; target < layout.targets; target++) {
        const sourceAttribute = sourceTargets[target]!;
        const targetAttribute = targetTargets[target]!;
        (targetAttribute.array as GeometryTypedArray).set(sourceAttribute.array as GeometryTypedArray, 0);
        markUpdated(targetAttribute, sourceAttribute.count * sourceAttribute.itemSize);
        activeBytes += sourceAttribute.array.byteLength;
      }
    }
    for (let group = 0; group < source.groups.length; group++) {
      const from = source.groups[group]!, to = slot.groups[group]!;
      to.start = from.start; to.count = from.count; to.materialIndex = from.materialIndex ?? 0;
      slot.geometry.groups[group] = to;
    }
    slot.geometry.groups.length = source.groups.length;
    slot.geometry.setDrawRange(0, sourceIndex.count);
    slot.geometry.morphTargetsRelative = source.morphTargetsRelative;
    if (!source.boundingBox) source.computeBoundingBox();
    if (!source.boundingSphere) source.computeBoundingSphere();
    slot.geometry.boundingBox!.copy(source.boundingBox!);
    slot.geometry.boundingSphere!.copy(source.boundingSphere!);
    slot.activeBytes = activeBytes;
    this.stats.activeGpuBytes += activeBytes;
    this.stats.uploadBytes += activeBytes;
    if (this.stats.activeGpuBytes > this.stats.activeGpuByteHighWater)
      this.stats.activeGpuByteHighWater = this.stats.activeGpuBytes;
    const elapsedNs = Math.max(1, Math.floor((performance.now() - startedAt) * 1_000_000));
    this.options.telemetry?.trace.spanBegin(
      EngineTraceDescriptor.GeometryUpload,
      slot.generation,
      activeBytes,
      slot.slot,
    );
    this.options.telemetry?.trace.spanEnd(
      EngineTraceDescriptor.GeometryUpload,
      slot.generation,
      activeBytes,
      slot.slot,
    );
    this.options.telemetry?.metrics.histogramLog2(EngineMetric.GeometryUploadNs, elapsedNs);
    this.options.telemetry?.metrics.maximum(
      EngineMetric.ModelGpuBytesHighWater, this.stats.activeGpuBytes,
    );
    return activeBytes;
  }

  publish(levels: readonly ModelGeometryLod[]): GeometryArenaPublication | null {
    if (this.disposed || levels.length === 0) return null;
    const buckets = new Int32Array(levels.length);
    const required = new Uint32Array(this.configs.length);
    for (let level = 0; level < levels.length; level++) {
      const bucket = this.compatibleBucket(levels[level]!.geometry);
      if (bucket < 0) {
        this.stats.rejectedPublications++;
        return null;
      }
      buckets[level] = bucket;
      required[bucket] = (required[bucket] ?? 0) + 1;
    }
    for (let bucket = 0; bucket < required.length; bucket++) {
      if ((required[bucket] ?? 0) > (this.freeTop[bucket] ?? 0)) {
        this.stats.rejectedPublications++;
        return null;
      }
    }
    const published: GeometryArenaLevel[] = new Array(levels.length);
    let bytes = 0;
    let acquired = 0;
    try {
      for (let level = 0; level < levels.length; level++) {
        const source = levels[level]!;
        const slot = this.acquire(buckets[level]!);
        acquired++;
        published[level] = {
          geometry: slot.geometry,
          ratio: source.ratio,
          triangleCount: source.triangleCount,
          arenaBucket: slot.bucket,
          arenaSlot: slot.slot,
          arenaGeneration: slot.generation,
        };
        bytes += this.copyInto(slot, source.geometry);
      }
    } catch {
      const rollback = published.slice(0, acquired).filter(level => level !== undefined);
      this.release({ levels: rollback, activeBytes: bytes, released: false });
      this.stats.rejectedPublications++;
      return null;
    }
    this.stats.publicationCount++;
    return { levels: published, activeBytes: bytes, released: false };
  }

  release(publication: GeometryArenaPublication): boolean {
    if (publication.released) return false;
    publication.released = true;
    for (const level of publication.levels) {
      const slot = this.slots[level.arenaBucket]?.[level.arenaSlot];
      if (!slot || !slot.active || slot.generation !== level.arenaGeneration) continue;
      slot.active = false;
      this.stats.activeSlots--;
      this.stats.activeGpuBytes -= slot.activeBytes;
      slot.activeBytes = 0;
      slot.geometry.setDrawRange(0, 0);
      slot.geometry.groups.length = 0;
      const top = this.freeTop[level.arenaBucket] ?? 0;
      this.free[level.arenaBucket]![top] = level.arenaSlot;
      this.freeTop[level.arenaBucket] = top + 1;
    }
    return true;
  }

  /** Geometries that a renderer warm scene must draw once before sealing. */
  visitWarmGeometries(visitor: (geometry: THREE.BufferGeometry) => void): void {
    for (const bucket of this.slots)
      for (const slot of bucket) visitor(slot.geometry);
  }

  getStats(): Readonly<GeometryArenaStats> { return this.stats; }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const bucket of this.slots)
      for (const slot of bucket) slot.geometry.dispose();
  }
}
