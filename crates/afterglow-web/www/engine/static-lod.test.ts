import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import { LodSet, StaticMeshAsset, projectedCoverage } from './static-lod.ts';

function meshes(count: number): THREE.Mesh[] {
  const result: THREE.Mesh[] = [];
  for (let index = 0; index < count; index++) result.push(new THREE.Mesh());
  return result;
}

describe('StaticMeshAsset', () => {
  test('owns decoded geometries without retaining a decoder', () => {
    const geometry = new THREE.BufferGeometry();
    let disposals = 0;
    geometry.addEventListener('dispose', () => { disposals++; });
    const asset = new StaticMeshAsset([{ geometry, triangleCount: 1 }]);
    asset.dispose();
    asset.dispose();
    expect(disposals).toBe(1);
    expect(Object.keys(asset)).not.toContain('decoder');
  });
});

describe('LodSet', () => {
  test('selects levels with hysteresis and one visible mesh', () => {
    const levels = meshes(4);
    const lod = new LodSet(levels, [0.3, 0.15, 0.06], 0.1, 4);
    expect(lod.select(0.5)).toBe(0);
    expect(lod.select(0.28)).toBe(0);
    expect(lod.select(0.26)).toBe(1);
    expect(lod.select(0.31)).toBe(1);
    expect(lod.select(0.34)).toBe(0);
    expect(lod.select(0.01)).toBe(3);
    expect(levels.filter((mesh) => mesh.visible)).toHaveLength(1);
  });

  test('rejects invalid capacities and thresholds', () => {
    expect(() => new LodSet(meshes(4), [0.3, 0.15, 0.06], 0.1, 3)).toThrow('capacity');
    expect(() => new LodSet(meshes(3), [0.1, 0.2], 0.1, 3)).toThrow('descending');
    expect(() => new LodSet(meshes(3), [0.2], 0.1, 3)).toThrow('separate');
  });
});

describe('static LOD demo boundary', () => {
  test('contains game presentation code rather than worker or container plumbing', async () => {
    const source = await Bun.file(new URL('../lod-demo.ts', import.meta.url)).text();
    expect(source).toContain('await loadStaticMesh({');
    expect(source).not.toContain('StaticLodSession');
    expect(source).not.toContain('MeshoptClient');
    expect(source).not.toContain('Rpc.create');
    expect(source).not.toContain('createDecoder');
  });
});

describe('projectedCoverage', () => {
  test('decreases monotonically with distance and is bounded', () => {
    expect(projectedCoverage(1, 2, Math.PI / 2)).toBeGreaterThan(projectedCoverage(1, 4, Math.PI / 2));
    expect(projectedCoverage(100, 0.1, Math.PI / 2)).toBe(1);
    expect(projectedCoverage(0, 2, Math.PI / 2)).toBe(0);
  });
});
