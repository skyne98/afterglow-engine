import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import { ModelLodBinding, buildModelGeometryLods } from './model-lod.ts';

function optimizer() {
  return {
    async optimizeVertexCache(indices: Uint32Array) { return indices.slice(); },
    async optimizeOverdraw(indices: Uint32Array) { return indices.slice(); },
    async simplifyWithUvs(indices: Uint32Array, _p: Float32Array, _ps: number, _uv: Float32Array, _us: number, _uw: number, target: number) {
      return indices.slice(0, Math.max(3, target));
    },
    async simplifyWithAttributes(
      indices: Uint32Array, _positions: Float32Array, _positionStride: number,
      attributes: Float32Array, attributeStride: number, weights: Float32Array,
      locks: Uint8Array, target: number,
    ) {
      expect(attributeStride).toBeGreaterThanOrEqual(36); // UV + normal + skin weights.
      expect(attributes.length).toBe((attributeStride / 4) * 4);
      expect(weights.length).toBe(attributeStride / 4);
      expect(locks.length).toBe(4);
      return indices.slice(0, Math.max(3, target));
    },
    async analyzeVertexCache() { return new Float32Array(4); },
    async encodeIndexBuffer() { return new Uint8Array(); },
    poll() {},
  };
}

function riggedGeometry(): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setIndex([0, 1, 2, 2, 1, 3]);
  geometry.setAttribute('position', new THREE.Float32BufferAttribute([
    0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0,
  ], 3));
  geometry.setAttribute('normal', new THREE.Float32BufferAttribute([
    0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1,
  ], 3));
  geometry.setAttribute('uv', new THREE.Float32BufferAttribute([0, 0, 1, 0, 0, 1, 1, 1], 2));
  geometry.setAttribute('skinIndex', new THREE.Uint16BufferAttribute([
    0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0,
  ], 4));
  geometry.setAttribute('skinWeight', new THREE.Float32BufferAttribute([
    1, 0, 0, 0, 0.8, 0.2, 0, 0, 0.5, 0.5, 0, 0, 0, 1, 0, 0,
  ], 4));
  geometry.morphAttributes.position = [new THREE.Float32BufferAttribute([
    0, 0, 0,
    0, 0.1, 0,
    0, 0, 0.2,
    0, 0.3, 0,
  ], 3)];
  geometry.addGroup(0, 6, 2);
  return geometry;
}

describe('deformation-aware model LODs', () => {
  test('simplifies and compacts rig, morph, and material-group data together', async () => {
    const levels = await buildModelGeometryLods(riggedGeometry(), optimizer(), {
      ratios: [1, 0.5], targetError: 0.02,
    });
    expect(levels).toHaveLength(2);
    expect(levels[1]!.triangleCount).toBe(1);
    const lod = levels[1]!.geometry;
    expect(lod.getAttribute('skinIndex').count).toBe(3);
    expect(lod.getAttribute('skinWeight').count).toBe(3);
    expect(lod.morphAttributes.position?.[0]?.count).toBe(3);
    expect(lod.groups).toEqual([{ start: 0, count: 3, materialIndex: 2 }]);
    for (const level of levels) level.geometry.dispose();
  });

  test('shares one skeleton and morph state across selected LOD meshes', async () => {
    const levels = await buildModelGeometryLods(riggedGeometry(), optimizer(), {
      ratios: [1, 0.5], targetError: 0.02,
    });
    const bone = new THREE.Bone();
    const source = new THREE.SkinnedMesh(riggedGeometry(), new THREE.MeshBasicMaterial());
    source.add(bone);
    source.bind(new THREE.Skeleton([bone]));
    source.morphTargetInfluences = [0.4];
    const binding = new ModelLodBinding(source, levels, new Float32Array([0.2]), 0.1);
    expect((binding.meshes[0] as THREE.SkinnedMesh).skeleton).toBe(source.skeleton);
    expect((binding.meshes[1] as THREE.SkinnedMesh).skeleton).toBe(source.skeleton);
    expect(binding.meshes[1]!.morphTargetInfluences).toBe(source.morphTargetInfluences);
    expect(binding.select(0.1)).toBe(1);
    expect(binding.meshes[1]!.visible).toBe(true);
    binding.dispose();
    source.geometry.dispose();
    (source.material as THREE.Material).dispose();
  });
});
