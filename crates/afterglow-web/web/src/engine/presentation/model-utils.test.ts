import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import {
  AnimationSet,
  ModelCollectionStatus,
  ModelNormalizationStatus,
  ModelPrimitives,
  SkeletonDebugAdapter,
  computeDeformedBoundsInto,
  groundDeformedModel,
  normalizeModelPivot,
} from './model-utils.ts';

function triangle(): THREE.Mesh {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute('position', new THREE.Float32BufferAttribute([
    0, 0, 0,
    2, 0, 0,
    0, 3, 0,
  ], 3));
  return new THREE.Mesh(geometry, new THREE.MeshBasicMaterial());
}

describe('model utility primitives', () => {
  test('collects meshes into fixed storage and reports overflow', () => {
    const root = new THREE.Group();
    root.add(triangle(), triangle());
    const primitives = new ModelPrimitives(1);
    expect(primitives.collect(root)).toBe(ModelCollectionStatus.CapacityExceeded);
    expect(primitives.count).toBe(1);
    const larger = new ModelPrimitives(2);
    expect(larger.collect(root)).toBe(ModelCollectionStatus.Complete);
    expect(larger.count).toBe(2);
  });

  test('computes exact transformed vertex bounds into caller scratch', () => {
    const root = new THREE.Group();
    const mesh = triangle();
    mesh.position.set(4, 5, 6);
    root.add(mesh);
    root.updateMatrixWorld(true);
    const primitives = new ModelPrimitives(1);
    primitives.collect(root);
    const bounds = computeDeformedBoundsInto(primitives, new THREE.Box3(), new THREE.Vector3());
    expect(bounds.min.toArray()).toEqual([4, 5, 6]);
    expect(bounds.max.toArray()).toEqual([6, 8, 6]);
  });

  test('normalizes target height, centers X/Z, and grounds Y', () => {
    const pivot = new THREE.Group();
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(2, 4, 6), new THREE.MeshBasicMaterial());
    mesh.position.set(3, 7, -2);
    pivot.add(mesh);
    const status = normalizeModelPivot(
      pivot, 2, new THREE.Box3(), new THREE.Vector3(), new THREE.Vector3(),
    );
    expect(status).toBe(ModelNormalizationStatus.Normalized);
    const bounds = new THREE.Box3().setFromObject(pivot);
    const size = bounds.getSize(new THREE.Vector3());
    expect(size.y).toBeCloseTo(2);
    expect(bounds.min.y).toBeCloseTo(0);
    expect(bounds.getCenter(new THREE.Vector3()).x).toBeCloseTo(0);
    expect(bounds.getCenter(new THREE.Vector3()).z).toBeCloseTo(0);
    expect(normalizeModelPivot(pivot, 0, new THREE.Box3(), new THREE.Vector3(), new THREE.Vector3()))
      .toBe(ModelNormalizationStatus.InvalidTargetHeight);
  });

  test('grounds the current deformed primitive bounds', () => {
    const pivot = new THREE.Group();
    const mesh = triangle();
    mesh.position.y = 4;
    pivot.add(mesh);
    const primitives = new ModelPrimitives(1);
    primitives.collect(pivot);
    expect(groundDeformedModel(pivot, primitives, new THREE.Box3(), new THREE.Vector3())).toBe(true);
    const bounds = computeDeformedBoundsInto(primitives, new THREE.Box3(), new THREE.Vector3());
    expect(bounds.min.y).toBeCloseTo(0);
  });

  test('precreates bounded animation actions and skips disabled updates', () => {
    const root = new THREE.Group();
    const animations = new AnimationSet(root, [new THREE.AnimationClip('idle', 1, [])], 1);
    expect(animations.play(0)).toBe(true);
    animations.update(0.25);
    expect(animations.mixer.time).toBeCloseTo(0.25);
    animations.setEnabled(false);
    animations.update(0.25);
    expect(animations.mixer.time).toBeCloseTo(0.25);
    expect(animations.play(1)).toBe(false);
    animations.dispose();
    animations.dispose();
    expect(animations.play(0)).toBe(false);
    expect(() => new AnimationSet(root, [new THREE.AnimationClip(), new THREE.AnimationClip()], 1))
      .toThrow('exceeds capacity');
  });

  test('owns skeleton helper visibility and idempotent disposal', () => {
    const scene = new THREE.Scene();
    const adapter = new SkeletonDebugAdapter(scene, new THREE.Group());
    expect(scene.children).toContain(adapter.helper);
    adapter.setVisible(true);
    expect(adapter.helper.visible).toBe(true);
    adapter.dispose();
    adapter.dispose();
    expect(scene.children).not.toContain(adapter.helper);
  });
});
