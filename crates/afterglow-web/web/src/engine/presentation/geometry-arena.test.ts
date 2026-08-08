import { describe, expect, test } from 'bun:test';
import * as THREE from 'three';
import { GeometryArena } from './geometry-arena.ts';

function geometry(scale: number): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setIndex(new THREE.Uint16BufferAttribute([0, 1, 2], 1));
  geometry.setAttribute('position', new THREE.Float32BufferAttribute([
    0, 0, 0, scale, 0, 0, 0, scale, 0,
  ], 3));
  geometry.setAttribute('normal', new THREE.Float32BufferAttribute([
    0, 0, 1, 0, 0, 1, 0, 0, 1,
  ], 3));
  geometry.setAttribute('skinIndex', new THREE.Uint16BufferAttribute([
    0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0,
  ], 4));
  geometry.setAttribute('skinWeight', new THREE.Float32BufferAttribute([
    1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
  ], 4));
  geometry.morphAttributes.position = [new THREE.Float32BufferAttribute([
    0, 0, 0, 0, 0, scale * 0.1, 0, 0, 0,
  ], 3)];
  geometry.addGroup(0, 3, 0);
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();
  return geometry;
}

const bucket = {
  slots: 2,
  maxVertices: 4,
  maxIndices: 6,
  maxGroups: 2,
  indexKind: 'u16' as const,
  attributes: [
    { name: 'position', itemSize: 3, kind: 'f32' as const },
    { name: 'normal', itemSize: 3, kind: 'f32' as const },
    { name: 'skinIndex', itemSize: 4, kind: 'u16' as const },
    { name: 'skinWeight', itemSize: 4, kind: 'f32' as const },
  ],
  morphAttributes: [
    { name: 'position', itemSize: 3, kind: 'f32' as const, targets: 1 },
  ],
};

describe('GeometryArena', () => {
  test('publishes complete deformation layouts into fixed reusable slots', () => {
    const arena = new GeometryArena({ buckets: [bucket] });
    const a = arena.publish([{ geometry: geometry(1), ratio: 1, triangleCount: 1 }]);
    const b = arena.publish([{ geometry: geometry(2), ratio: 0.5, triangleCount: 1 }]);
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(arena.publish([{ geometry: geometry(3), ratio: 1, triangleCount: 1 }])).toBeNull();
    const backing = a!.levels[0]!.geometry.getAttribute('position').array;
    expect(Array.from(backing.slice(0, 9))).toEqual([0, 0, 0, 1, 0, 0, 0, 1, 0]);
    expect(a!.levels[0]!.geometry.morphAttributes.position?.length).toBe(1);
    expect(a!.levels[0]!.geometry.groups).toEqual([{ start: 0, count: 3, materialIndex: 0 }]);
    expect(arena.release(a!)).toBe(true);
    expect(arena.release(a!)).toBe(false);
    const c = arena.publish([{ geometry: geometry(4), ratio: 1, triangleCount: 1 }]);
    expect(c).not.toBeNull();
    expect(c!.levels[0]!.geometry.getAttribute('position').array).toBe(backing);
    expect(c!.levels[0]!.arenaGeneration).not.toBe(a!.levels[0]!.arenaGeneration);
    expect(arena.getStats().activeSlots).toBe(2);
    expect(arena.getStats().reservedSlots).toBe(2);
    arena.release(b!);
    arena.release(c!);
    expect(arena.getStats().activeGpuBytes).toBe(0);
  });

  test('rejects incompatible attributes without consuming a slot', () => {
    const arena = new GeometryArena({ buckets: [bucket] });
    const incompatible = geometry(1);
    incompatible.deleteAttribute('normal');
    expect(arena.publish([{ geometry: incompatible, ratio: 1, triangleCount: 1 }])).toBeNull();
    expect(arena.getStats().activeSlots).toBe(0);
    expect(arena.getStats().rejectedPublications).toBe(1);
  });
});
