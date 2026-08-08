import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import { CookedModelAsset } from './cooked-model-source.ts';
import { ModelSystem } from './model-system.ts';

function geometry(scale = 1): THREE.BufferGeometry {
  const result = new THREE.BufferGeometry();
  result.setIndex([0, 1, 2, 2, 1, 3]);
  result.setAttribute('position', new THREE.Float32BufferAttribute([
    0, 0, 0, scale, 0, 0, 0, scale, 0, scale, scale, 0,
  ], 3));
  result.setAttribute('uv', new THREE.Float32BufferAttribute([0, 0, 1, 0, 0, 1, 1, 1], 2));
  result.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(new Array(16).fill(0), 4));
  result.setAttribute('skinWeight', new THREE.Float32BufferAttribute([
    1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
  ], 4));
  return result;
}

function optimizer() {
  return {
    async optimizeVertexCache(indices: Uint32Array) { return indices.slice(); },
    async optimizeOverdraw(indices: Uint32Array) { return indices.slice(); },
    async simplifyWithUvs(indices: Uint32Array, _p: Float32Array, _ps: number, _u: Float32Array, _us: number, _w: number, target: number) {
      return indices.slice(0, target);
    },
    async simplifyWithAttributes(indices: Uint32Array, _p: Float32Array, _ps: number, _a: Float32Array, _as: number, _aw: Float32Array, _l: Uint8Array, target: number) {
      return indices.slice(0, target);
    },
    async analyzeVertexCache() { return new Float32Array(4); },
    async encodeIndexBuffer() { return new Uint8Array(); },
    poll() {},
  };
}

async function settle(system: ModelSystem): Promise<void> {
  await Promise.resolve();
  await new Promise(resolve => setTimeout(resolve, 0));
  system.poll();
}

function arena(indexKind: 'u16' | 'u32', slots: number) {
  return { buckets: [{
    slots, maxVertices: 4, maxIndices: 6, maxGroups: 1, indexKind,
    attributes: [
      { name: 'position', itemSize: 3, kind: 'f32' as const },
      { name: 'uv', itemSize: 2, kind: 'f32' as const },
      { name: 'skinIndex', itemSize: 4, kind: 'u16' as const },
      { name: 'skinWeight', itemSize: 4, kind: 'f32' as const },
    ],
    morphAttributes: [],
  }] };
}

describe('ModelSystem', () => {
  test('publishes optimized rig-compatible RAM revisions atomically', async () => {
    const system = new ModelSystem(optimizer(), {
      maxModels: 2, maxPendingOptimizations: 1, maxResidentCpuBytes: 1024 * 1024,
      completionsPerPoll: 1, ratios: [1, 0.5], targetError: 0.02,
      geometryArena: arena('u32', 4),
    });
    const handle = system.createRuntimeModel(geometry());
    expect(handle).not.toBe(0);
    if (handle === 0) throw new Error('model registration failed');
    expect(system.getView(handle)?.status).toBe('optimizing');
    await settle(system);
    const first = system.getView(handle);
    expect(first?.status).toBe('ready');
    expect(first?.levels).toHaveLength(2);
    expect(first?.levels[1]?.geometry.getAttribute('skinWeight').count).toBe(4);
    expect(first?.levels[1]?.geometry.drawRange.count).toBe(3);
    const oldLevels = first!.levels;
    expect(system.reviseRuntimeModel(handle, geometry(2))).toBe(true);
    expect(system.getView(handle)?.levels).toBe(oldLevels);
    await settle(system);
    expect(system.getView(handle)?.revision).toBe(2);
    expect(system.getView(handle)?.levels).not.toBe(oldLevels);
    system.dispose();
  });

  test('atomically swaps complete revisions through fixed GPU geometry slots', async () => {
    const system = new ModelSystem(optimizer(), {
      maxModels: 1, maxPendingOptimizations: 1, maxResidentCpuBytes: 1024 * 1024,
      completionsPerPoll: 1, ratios: [1, 0.5], targetError: 0.02,
      geometryArena: arena('u32', 4),
    });
    const handle = system.createRuntimeModel(geometry());
    if (handle === 0) throw new Error('model registration failed');
    await settle(system);
    const first = system.getView(handle)!.levels;
    expect(system.getGeometryStats().activeSlots).toBe(2);
    expect(system.reviseRuntimeModel(handle, geometry(2))).toBe(true);
    expect(system.getView(handle)!.levels).toBe(first);
    await settle(system);
    expect(system.getView(handle)!.levels).not.toBe(first);
    expect(system.getGeometryStats().activeSlots).toBe(2);
    expect(system.getGeometryStats().slotHighWater).toBe(4);
    system.destroyModel(handle);
    expect(system.getGeometryStats().activeSlots).toBe(0);
    system.dispose();
  });

  test('adopts cooked disk LODs into the same bounded handle space', () => {
    const system = new ModelSystem(optimizer(), {
      maxModels: 1, maxPendingOptimizations: 1, maxResidentCpuBytes: 1024 * 1024,
      completionsPerPoll: 1, ratios: [1, 0.5], targetError: 0.02,
      geometryArena: arena('u16', 2),
    });
    const first = geometry(), second = geometry();
    const asset = new CookedModelAsset([
      { geometry: first, triangleCount: 2 }, { geometry: second, triangleCount: 1 },
    ]);
    const handle = system.adoptCookedModel(asset);
    expect(handle).not.toBe(0);
    asset.dispose(); // ownership was transferred; geometries remain live.
    expect(first.index).not.toBeNull();
    expect(system.activeModels).toBe(1);
    system.dispose();
  });
});
