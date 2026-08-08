import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import { CookedModelAsset, projectedCoverage } from './cooked-model-source.ts';

describe('CookedModelAsset', () => {
  test('owns decoded geometries without retaining a decoder', () => {
    const geometry = new THREE.BufferGeometry();
    let disposals = 0;
    geometry.addEventListener('dispose', () => { disposals++; });
    const asset = new CookedModelAsset([{ geometry, triangleCount: 1 }]);
    asset.dispose();
    asset.dispose();
    expect(disposals).toBe(1);
    expect(Object.keys(asset)).not.toContain('decoder');
  });
});

describe('cooked model demo boundary', () => {
  test('contains game presentation code rather than worker or container plumbing', async () => {
    const source = await Bun.file(new URL('../../demos/lod/main.ts', import.meta.url)).text();
    expect(source).toContain('await loadCookedModel({');
    expect(source).toContain('await ModelSystem.open({');
    expect(source).toContain('modelSystem.adoptCookedModel(asset)');
    expect(source).toContain('modelSystem.createBinding(');
    expect(source).not.toContain('new ModelLodBinding(');
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
