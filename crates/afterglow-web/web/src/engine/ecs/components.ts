// ECS components — Structure of Arrays (SoA) TypedArrays.
//
// bitECS 0.4.0 stores component data in plain objects with arrays. For the
// hot-path transforms we use Float32Array directly (not plain JS arrays) for
// maximum cache locality and JIT auto-vectorization.

export const MAX_ENTITIES = 1_000_000;

export interface TransformStore {
  readonly positionX: Float32Array;
  readonly positionY: Float32Array;
  readonly positionZ: Float32Array;
  readonly rotationX: Float32Array;
  readonly rotationY: Float32Array;
  readonly rotationZ: Float32Array;
  readonly rotationW: Float32Array;
  readonly scaleX: Float32Array;
  readonly scaleY: Float32Array;
  readonly scaleZ: Float32Array;
}

export interface AppearanceStore {
  readonly red: Float32Array;
  readonly green: Float32Array;
  readonly blue: Float32Array;
  readonly opacity: Float32Array;
}

export interface RenderRefStore {
  /** Index into RenderResourceRegistry. Zero means no descriptor. */
  readonly descriptorId: Uint32Array;
}

export function createTransformStore(capacity: number = MAX_ENTITIES): TransformStore {
  const rotationW = new Float32Array(capacity);
  const scaleX = new Float32Array(capacity);
  const scaleY = new Float32Array(capacity);
  const scaleZ = new Float32Array(capacity);

  rotationW.fill(1);
  scaleX.fill(1);
  scaleY.fill(1);
  scaleZ.fill(1);

  return {
    positionX: new Float32Array(capacity),
    positionY: new Float32Array(capacity),
    positionZ: new Float32Array(capacity),
    rotationX: new Float32Array(capacity),
    rotationY: new Float32Array(capacity),
    rotationZ: new Float32Array(capacity),
    rotationW,
    scaleX,
    scaleY,
    scaleZ,
  };
}

export function createAppearanceStore(capacity: number = MAX_ENTITIES): AppearanceStore {
  const red = new Float32Array(capacity);
  const green = new Float32Array(capacity);
  const blue = new Float32Array(capacity);
  const opacity = new Float32Array(capacity);

  red.fill(1);
  green.fill(1);
  blue.fill(1);
  opacity.fill(1);

  return { red, green, blue, opacity };
}

export function createRenderRefStore(capacity: number = MAX_ENTITIES): RenderRefStore {
  return {
    descriptorId: new Uint32Array(capacity),
  };
}
