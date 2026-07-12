// InstanceShard — a fixed-capacity InstancedMesh batch with dense slots.
//
// Entities with the same geometry+material share one shard. Slots are
// allocated sequentially and freed via swap-remove (O(1), preserves
// contiguity). Matrix and appearance data are written directly to the
// InstancedMesh's Float32Array buffers — no setMatrixAt() calls.

import * as THREE from 'three';
import { NONE_U32, type EntityId, type RenderDescriptorId } from './types.js';
import type { InstancedRenderDescriptor } from './descriptors.js';
import { DirtySlotRanges } from './dirty-ranges.js';

export class InstanceShard {
  readonly mesh: THREE.InstancedMesh;
  readonly slotToEntity: Uint32Array;
  readonly tintOpacityAttribute: THREE.InstancedBufferAttribute;

  private readonly matrixDirty: DirtySlotRanges;
  private readonly appearanceDirty: DirtySlotRanges;

  readonly matrixData: Float32Array;
  readonly appearanceData: Float32Array;

  count = 0;

  constructor(
    readonly id: number,
    readonly descriptorId: RenderDescriptorId,
    readonly descriptor: InstancedRenderDescriptor,
    scene: THREE.Scene,
  ) {
    const capacity = descriptor.shardCapacity;

    this.slotToEntity = new Uint32Array(capacity);
    this.slotToEntity.fill(NONE_U32);

    this.tintOpacityAttribute = new THREE.InstancedBufferAttribute(
      new Float32Array(capacity * 4),
      4,
    );
    this.tintOpacityAttribute.setUsage(THREE.DynamicDrawUsage);

    const material = descriptor.createMaterial(this.tintOpacityAttribute);

    this.mesh = new THREE.InstancedMesh(descriptor.geometry, material, capacity);
    this.mesh.count = 0;
    this.mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    this.mesh.matrixAutoUpdate = false;
    this.mesh.matrixWorldAutoUpdate = false;

    this.mesh.castShadow = descriptor.castShadow ?? false;
    this.mesh.receiveShadow = descriptor.receiveShadow ?? false;
    this.mesh.renderOrder = descriptor.renderOrder ?? 0;

    if (descriptor.layersMask !== undefined) {
      this.mesh.layers.mask = descriptor.layersMask;
    }

    this.mesh.frustumCulled = descriptor.boundsPolicy !== 'dynamic-disable-three-culling';
    descriptor.configureMesh?.(this.mesh);

    this.matrixData = this.mesh.instanceMatrix.array as Float32Array;
    this.appearanceData = this.tintOpacityAttribute.array as Float32Array;

    this.matrixDirty = new DirtySlotRanges(capacity);
    this.appearanceDirty = new DirtySlotRanges(capacity);

    scene.add(this.mesh);
  }

  hasCapacity(): boolean {
    return this.count < this.slotToEntity.length;
  }

  allocate(entity: EntityId): number {
    if (!this.hasCapacity()) throw new Error('Instance shard full');
    const slot = this.count++;
    this.slotToEntity[slot] = entity;
    this.mesh.count = this.count;
    this.matrixDirty.mark(slot);
    this.appearanceDirty.mark(slot);
    return slot;
  }

  remove(slot: number, entityToSlot: Uint32Array, entityToHandle: Uint32Array): void {
    const lastSlot = this.count - 1;
    const movedEntity = this.slotToEntity[lastSlot];

    this.count = lastSlot;
    this.mesh.count = lastSlot;

    if (slot !== lastSlot) {
      // Swap-remove: copy last slot's data to the freed slot.
      this.matrixData.copyWithin(slot * 16, lastSlot * 16, lastSlot * 16 + 16);
      this.appearanceData.copyWithin(slot * 4, lastSlot * 4, lastSlot * 4 + 4);
      this.slotToEntity[slot] = movedEntity;
      entityToSlot[movedEntity] = slot;
      entityToHandle[movedEntity] = this.id;
      this.matrixDirty.mark(slot);
      this.appearanceDirty.mark(slot);
    }

    this.slotToEntity[lastSlot] = NONE_U32;
  }

  markMatrix(slot: number): void {
    this.matrixDirty.mark(slot);
  }

  markAppearance(slot: number): void {
    this.appearanceDirty.mark(slot);
  }

  flushUploads(): void {
    this.matrixDirty.flush(this.mesh.instanceMatrix, 16, this.count);
    this.appearanceDirty.flush(this.tintOpacityAttribute, 4, this.count);
  }
}
