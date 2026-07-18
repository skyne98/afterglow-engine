import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import { LodSet, projectedCoverage } from './static-lod.ts';

function meshes(count: number): THREE.Mesh[] {
  const result: THREE.Mesh[] = [];
  for (let index = 0; index < count; index++) result.push(new THREE.Mesh());
  return result;
}

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

describe('projectedCoverage', () => {
  test('decreases monotonically with distance and is bounded', () => {
    expect(projectedCoverage(1, 2, Math.PI / 2)).toBeGreaterThan(projectedCoverage(1, 4, Math.PI / 2));
    expect(projectedCoverage(100, 0.1, Math.PI / 2)).toBe(1);
    expect(projectedCoverage(0, 2, Math.PI / 2)).toBe(0);
  });
});
