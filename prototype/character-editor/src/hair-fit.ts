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

export interface HairFitDocument {
  version: number;
  driverVertexCount: number;
  driverNeutral: number[];
  targets: Record<string, number[]>;
  scalp: HairStyleDocument;
  styles: HairStyleDocument[];
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

function parseSurface(source: HairStyleDocument, driverVertexCount: number): HairStyleRuntime {
  if (!source.id || !source.mesh) fail('surface identity');
  if (!Number.isInteger(source.vertexCount) || source.vertexCount <= 0) fail(`surface count ${source.id}`);
  const componentCount = source.vertexCount * 3;
  if (
    source.parents.length !== componentCount
    || source.weights.length !== componentCount
    || source.offsets.length !== componentCount
    || !finite(source.parents)
    || !finite(source.weights)
    || !finite(source.offsets)
  ) {
    fail(`surface arrays ${source.id}`);
  }
  for (const parent of source.parents) {
    if (!Number.isInteger(parent) || parent < 0 || parent >= driverVertexCount || parent > 0xffff) {
      fail(`surface parent ${source.id}`);
    }
  }
  if (source.scales.length !== 3) fail(`surface scales ${source.id}`);
  for (const scale of source.scales) {
    if (
      scale.length !== 4
      || !finite(scale)
      || !Number.isInteger(scale[0])
      || !Number.isInteger(scale[1])
      || scale[0] < 0
      || scale[1] < 0
      || scale[0] >= driverVertexCount
      || scale[1] >= driverVertexCount
      || !(scale[2] > 0)
      || !Number.isInteger(scale[3])
      || scale[3] < 0
      || scale[3] > 2
    ) {
      fail(`surface scale ${source.id}`);
    }
  }
  return {
    id: source.id,
    label: source.label,
    mesh: source.mesh,
    vertexCount: source.vertexCount,
    parents: new Uint16Array(source.parents),
    weights: new Float32Array(source.weights),
    offsets: new Float32Array(source.offsets),
    scales: source.scales,
  };
}

export class HairFitRuntime {
  readonly driver: Float32Array;
  readonly scalp: HairStyleRuntime;
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
    this.scalp = parseSurface(document.scalp, document.driverVertexCount);
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
      if (this.styleById.has(source.id)) fail('style identity');
      const style = parseSurface(source, document.driverVertexCount);
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
    this.fit(this.scalp, output);
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
