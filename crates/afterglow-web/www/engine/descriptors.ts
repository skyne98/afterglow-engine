// Render descriptors — define the draw-level state for an entity.
// Entities store only the integer descriptor ID; the descriptor determines
// geometry, material, render tier, and how the proxy is created/synced.

import * as THREE from 'three';
import type { EntityId, RenderDescriptorId, RenderTier, RenderFrame, RenderDirty } from './types.js';

export type BoundsPolicy =
  | 'static-compute-once'
  | 'dynamic-disable-three-culling'
  | 'externally-managed';

export interface InstancedRenderDescriptor {
  readonly tier: RenderTier.Instanced;
  readonly geometry: THREE.BufferGeometry;
  readonly createMaterial: (tintOpacity: THREE.InstancedBufferAttribute) => THREE.Material;
  readonly shardCapacity: number;
  readonly boundsPolicy: BoundsPolicy;
  readonly castShadow?: boolean;
  readonly receiveShadow?: boolean;
  readonly renderOrder?: number;
  readonly layersMask?: number;
  readonly configureMesh?: (mesh: THREE.InstancedMesh) => void;
}

export interface UniqueRenderDescriptor {
  readonly tier: RenderTier.Unique;
  readonly instantiate: () => THREE.Object3D;
  readonly sync?: (object: THREE.Object3D, entity: EntityId, dirty: RenderDirty, frame: RenderFrame) => void;
  readonly continuous?: boolean;
}

export interface GpuDrivenRenderDescriptor {
  readonly tier: RenderTier.GpuDriven;
  readonly createBackend: (context: GpuRenderContext) => GpuPopulationBackend;
}

export type RenderDescriptor =
  | InstancedRenderDescriptor
  | UniqueRenderDescriptor
  | GpuDrivenRenderDescriptor;

export interface GpuRenderContext {
  readonly renderer: THREE.WebGPURenderer;
  readonly scene: THREE.Scene;
}

export interface GpuPopulationBackend {
  attachController(entity: EntityId): void;
  detachController(entity: EntityId): void;
  prepare(frame: RenderFrame): void;
  dispose(): void;
}

export class RenderResourceRegistry {
  private readonly descriptors: RenderDescriptor[] = [null as unknown as RenderDescriptor];

  register(descriptor: RenderDescriptor): RenderDescriptorId {
    const id = this.descriptors.length;
    this.descriptors.push(descriptor);
    return id;
  }

  get(id: RenderDescriptorId): RenderDescriptor {
    const descriptor = this.descriptors[id];
    if (!descriptor) throw new Error(`Unknown render descriptor ${id}`);
    return descriptor;
  }
}
