// Core types for the afterglow-engine render adapter.

export type EntityId = number;
export type RenderDescriptorId = number;
export type RenderProxyHandle = number;

export const NULL_ENTITY = 0;
export const NONE_U32 = 0xffff_ffff;

export const enum RenderTier {
  None = 0,
  Instanced = 1,
  Unique = 2,
  GpuDriven = 3,
}

export const enum RenderDirty {
  None = 0,
  Transform = 1 << 0,
  Appearance = 1 << 1,
  Animation = 1 << 2,
  Structural = 1 << 3,
  WorldOnly = 1 << 4,
}

export interface RenderFrame {
  readonly frameId: number;
  readonly deltaSeconds: number;
  readonly elapsedSeconds: number;
}
