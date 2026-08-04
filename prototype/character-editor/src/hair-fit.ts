export interface HairStyleDocument {
  id: string;
  label: string;
  mesh: string;
  vertexCount: number;
  parents: number[];
  weights: number[];
  offsets: number[];
  scales: Array<[number, number, number, number]>;
  neutralMaximumError: number;
}

export interface HairScalpDocument {
  mesh: string;
  vertexCount: number;
  drivers: number[];
}

export interface HairFitDocument {
  version: number;
  driverVertexCount: number;
  driverNeutral: number[];
  targets: Record<string, number[]>;
  scalp: HairScalpDocument;
  styles: HairStyleDocument[];
}

export interface HairScalpRuntime {
  readonly mesh: string;
  readonly vertexCount: number;
  readonly drivers: Uint16Array;
}

export interface HairStyleRuntime {
  readonly id: string;
  readonly label: string;
  readonly mesh: string;
  readonly vertexCount: number;
  readonly parents: Uint16Array;
  readonly weights: Float32Array;
  readonly offsets: Float32Array;
  readonly scales: ReadonlyArray<readonly [number, number, number, number]>;
}

function fail(message: string): never {
  throw new Error(`Invalid hair-fit data: ${message}`);
}

function finite(values: ArrayLike<number>): boolean {
  for (let index = 0; index < values.length; index++) {
    if (!Number.isFinite(values[index])) return false;
  }
  return true;
}

function calculateScale(
  driver: Float32Array,
  scale: readonly [number, number, number, number],
): number {
  const first = scale[0] * 3 + scale[3];
  const second = scale[1] * 3 + scale[3];
  return Math.abs(driver[first] - driver[second]) / scale[2];
}

export class HairFitRuntime {
  readonly driver: Float32Array;
  readonly scalp: HairScalpRuntime;
  readonly styles: readonly HairStyleRuntime[];
  private readonly morphWeights: Float32Array;
  private readonly targetByMorph: Array<Float32Array | undefined>;
  private readonly styleById = new Map<string, HairStyleRuntime>();

  constructor(document: HairFitDocument, morphNames: readonly string[]) {
    if (document.version !== 1 || !Number.isInteger(document.driverVertexCount) || document.driverVertexCount <= 0) {
      fail('version or driver count');
    }
    if (document.driverNeutral.length !== document.driverVertexCount * 3 || !finite(document.driverNeutral)) {
      fail('neutral driver');
    }
    this.driver = new Float32Array(document.driverNeutral);
    if (
      !document.scalp?.mesh
      || !Number.isInteger(document.scalp.vertexCount)
      || document.scalp.vertexCount <= 0
      || document.scalp.drivers.length !== document.scalp.vertexCount
    ) {
      fail('scalp');
    }
    for (const driver of document.scalp.drivers) {
      if (!Number.isInteger(driver) || driver < 0 || driver >= document.driverVertexCount || driver > 0xffff) {
        fail('scalp driver');
      }
    }
    this.scalp = {
      mesh: document.scalp.mesh,
      vertexCount: document.scalp.vertexCount,
      drivers: new Uint16Array(document.scalp.drivers),
    };
    this.morphWeights = new Float32Array(morphNames.length);
    this.targetByMorph = new Array(morphNames.length);

    for (let morph = 0; morph < morphNames.length; morph++) {
      const source = document.targets[morphNames[morph]];
      if (source === undefined) continue;
      if (source.length % 4 !== 0 || !finite(source)) fail(`target ${morphNames[morph]}`);
      for (let offset = 0; offset < source.length; offset += 4) {
        const vertex = source[offset];
        if (!Number.isInteger(vertex) || vertex < 0 || vertex >= document.driverVertexCount) {
          fail(`target index ${morphNames[morph]}`);
        }
      }
      this.targetByMorph[morph] = new Float32Array(source);
    }

    this.styles = document.styles.map((source) => {
      if (!source.id || !source.mesh || this.styleById.has(source.id)) fail('style identity');
      if (!Number.isInteger(source.vertexCount) || source.vertexCount <= 0) fail(`style count ${source.id}`);
      const componentCount = source.vertexCount * 3;
      if (
        source.parents.length !== componentCount
        || source.weights.length !== componentCount
        || source.offsets.length !== componentCount
        || !finite(source.parents)
        || !finite(source.weights)
        || !finite(source.offsets)
      ) {
        fail(`style arrays ${source.id}`);
      }
      for (const parent of source.parents) {
        if (!Number.isInteger(parent) || parent < 0 || parent >= document.driverVertexCount || parent > 0xffff) {
          fail(`style parent ${source.id}`);
        }
      }
      if (source.scales.length !== 3) fail(`style scales ${source.id}`);
      for (const scale of source.scales) {
        if (
          scale.length !== 4
          || !finite(scale)
          || !Number.isInteger(scale[0])
          || !Number.isInteger(scale[1])
          || scale[0] < 0
          || scale[1] < 0
          || scale[0] >= document.driverVertexCount
          || scale[1] >= document.driverVertexCount
          || !(scale[2] > 0)
          || !Number.isInteger(scale[3])
          || scale[3] < 0
          || scale[3] > 2
        ) {
          fail(`style scale ${source.id}`);
        }
      }
      const style: HairStyleRuntime = {
        id: source.id,
        label: source.label,
        mesh: source.mesh,
        vertexCount: source.vertexCount,
        parents: new Uint16Array(source.parents),
        weights: new Float32Array(source.weights),
        offsets: new Float32Array(source.offsets),
        scales: source.scales,
      };
      this.styleById.set(style.id, style);
      return style;
    });
    if (this.styles.length === 0) fail('no styles');
  }

  style(id: string): HairStyleRuntime | undefined {
    return this.styleById.get(id);
  }

  setTarget(morph: number, amount: number): boolean {
    if (!Number.isInteger(morph) || morph < 0 || morph >= this.morphWeights.length || !Number.isFinite(amount)) {
      fail('morph update');
    }
    const target = this.targetByMorph[morph];
    if (!target) return false;
    const difference = amount - this.morphWeights[morph];
    if (difference === 0) return false;
    this.morphWeights[morph] = amount;
    for (let offset = 0; offset < target.length; offset += 4) {
      const driverOffset = target[offset] * 3;
      this.driver[driverOffset] += target[offset + 1] * difference;
      this.driver[driverOffset + 1] += target[offset + 2] * difference;
      this.driver[driverOffset + 2] += target[offset + 3] * difference;
    }
    return true;
  }

  fitScalp(output: Float32Array): void {
    if (output.length !== this.scalp.vertexCount * 3) fail('scalp output');
    for (let vertex = 0; vertex < this.scalp.vertexCount; vertex++) {
      const outputOffset = vertex * 3;
      const driverOffset = this.scalp.drivers[vertex] * 3;
      output[outputOffset] = this.driver[driverOffset];
      output[outputOffset + 1] = this.driver[driverOffset + 2];
      output[outputOffset + 2] = -this.driver[driverOffset + 1];
    }
  }

  fit(style: HairStyleRuntime, output: Float32Array): void {
    if (output.length !== style.vertexCount * 3) fail(`output ${style.id}`);
    const scaleX = calculateScale(this.driver, style.scales[0]);
    const scaleY = calculateScale(this.driver, style.scales[1]);
    const scaleZ = calculateScale(this.driver, style.scales[2]);
    for (let vertex = 0; vertex < style.vertexCount; vertex++) {
      const component = vertex * 3;
      let x = style.offsets[component] * scaleX;
      let y = style.offsets[component + 1] * scaleY;
      let z = style.offsets[component + 2] * scaleZ;
      for (let parent = 0; parent < 3; parent++) {
        const binding = component + parent;
        const driver = style.parents[binding] * 3;
        const weight = style.weights[binding];
        x += this.driver[driver] * weight;
        y += this.driver[driver + 1] * weight;
        z += this.driver[driver + 2] * weight;
      }
      // Blender exports glTF with Y up: (x, y, z) becomes (x, z, -y).
      output[component] = x;
      output[component + 1] = z;
      output[component + 2] = -y;
    }
  }
}
