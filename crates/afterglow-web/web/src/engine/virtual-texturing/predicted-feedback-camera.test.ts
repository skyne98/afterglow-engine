import { describe, expect, test } from 'bun:test';
import * as THREE from 'three';
import { PredictedFeedbackCamera } from './predicted-feedback-camera.ts';

describe('PredictedFeedbackCamera', () => {
  test('keeps a static camera unchanged', () => {
    const source = new THREE.PerspectiveCamera(60, 1.5, 0.1, 100);
    source.position.set(1, 2, 3);
    source.updateMatrixWorld(true);
    const predictor = new PredictedFeedbackCamera(source, 100);
    predictor.sample(source, 0);
    predictor.sample(source, 0.016);
    expect(predictor.camera.position.toArray()).toEqual([1, 2, 3]);
    expect(predictor.resetCount).toBe(0);
  });

  test('extrapolates translation one hundred milliseconds ahead', () => {
    const source = new THREE.PerspectiveCamera(60, 1, 0.1, 100);
    source.updateMatrixWorld(true);
    const predictor = new PredictedFeedbackCamera(source, 100);
    predictor.sample(source, 0);
    source.position.x = 1;
    source.updateMatrixWorld(true);
    predictor.sample(source, 0.1);
    expect(predictor.camera.position.x).toBeCloseTo(2);
    expect(source.position.x).toBe(1);
  });

  test('extrapolates shortest-path quaternion rotation', () => {
    const source = new THREE.PerspectiveCamera(60, 1, 0.1, 100);
    source.updateMatrixWorld(true);
    const predictor = new PredictedFeedbackCamera(source, 100);
    predictor.sample(source, 0);
    source.rotation.y = Math.PI / 4;
    source.updateMatrixWorld(true);
    predictor.sample(source, 0.1);
    const direction = new THREE.Vector3(0, 0, -1).applyQuaternion(predictor.camera.quaternion);
    expect(direction.x).toBeCloseTo(-1, 5);
    expect(direction.z).toBeCloseTo(0, 5);
  });

  test('uses world pose without mutating a parented source camera', () => {
    const parent = new THREE.Object3D();
    const source = new THREE.PerspectiveCamera(60, 1, 0.1, 100);
    parent.position.set(4, 0, 0);
    source.position.set(1, 0, 0);
    parent.add(source);
    parent.updateMatrixWorld(true);
    const predictor = new PredictedFeedbackCamera(source, 100);
    predictor.sample(source, 0);
    expect(predictor.camera.position.x).toBe(5);
    expect(source.position.x).toBe(1);
    expect(predictor.camera.parent).toBeNull();
  });

  test('resets after suspension or a teleport-like step', () => {
    const source = new THREE.PerspectiveCamera(60, 1, 0.1, 20);
    source.updateMatrixWorld(true);
    const predictor = new PredictedFeedbackCamera(source, 100);
    predictor.sample(source, 0);
    source.position.x = 8;
    source.updateMatrixWorld(true);
    predictor.sample(source, 0.016);
    expect(predictor.camera.position.x).toBe(8);
    predictor.sample(source, 1);
    expect(predictor.resetCount).toBe(2);
  });

  test('rejects an invalid horizon', () => {
    expect(() => new PredictedFeedbackCamera(new THREE.Camera(), 0)).toThrow('horizon');
  });
});
