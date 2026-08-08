import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import { VirtualPomSceneBinding } from './virtual-pom-binding.ts';
import type { VirtualPomMaterialPair } from './virtual-texture-material.ts';
import type {
  VirtualTextureHandle,
  VirtualTextureMaterialSet,
  VirtualTextureSystem,
} from './virtual-texture-system.ts';

function pair(): VirtualPomMaterialPair {
  return {
    baseMaterial: new THREE.MeshStandardNodeMaterial(),
    pomMaterial: new THREE.MeshStandardNodeMaterial(),
    baseFeedbackMaterial: new THREE.MeshBasicNodeMaterial(),
    pomFeedbackMaterial: new THREE.MeshBasicNodeMaterial(),
  };
}

const textures = {} as VirtualTextureSystem; // @unsafe-cast reason=FactoryInjectedTestDouble issue=DME-024 expires=2026-10-01
const set: VirtualTextureMaterialSet = {
  albedo: 1 as VirtualTextureHandle, // @unsafe-cast reason=FactoryInjectedTestHandle issue=DME-024 expires=2026-10-01
};

describe('VirtualPomSceneBinding', () => {
  test('owns fixed visible/feedback variants and toggles references', () => {
    const created: VirtualPomMaterialPair[] = [];
    const binding = new VirtualPomSceneBinding({
      camera: new THREE.PerspectiveCamera(), textures, feedbackPixelScale: new THREE.Vector2(1, 1),
      capacity: 1,
      createPair: () => { const value = pair(); created.push(value); return value; },
    });
    const sourceMaterial = new THREE.MeshBasicMaterial();
    const mesh: THREE.Mesh<THREE.BufferGeometry, THREE.Material> =
      new THREE.Mesh(new THREE.PlaneGeometry(), sourceMaterial);
    const result = binding.add(mesh, set, new THREE.DataTexture());
    expect(created[0]).toBeDefined();
    expect(result === created[0]).toBe(true);
    expect(mesh.material === result.pomMaterial).toBe(true);
    expect(binding.feedbackScene.children).toHaveLength(1);
    expect((binding.feedbackScene.children[0] as THREE.Mesh).material).toBe(result.pomFeedbackMaterial); // @unsafe-cast reason=KnownFeedbackMesh issue=DME-024 expires=2026-10-01
    binding.setPomEnabled(false);
    expect(mesh.material === result.baseMaterial).toBe(true);
    expect((binding.feedbackScene.children[0] as THREE.Mesh).material).toBe(result.baseFeedbackMaterial); // @unsafe-cast reason=KnownFeedbackMesh issue=DME-024 expires=2026-10-01
    expect(() => binding.add(new THREE.Mesh(), set, new THREE.DataTexture())).toThrow('capacity');
    binding.seal();
    expect(() => binding.add(new THREE.Mesh(), set, new THREE.DataTexture())).toThrow('sealed');
    binding.dispose();
    binding.dispose();
    expect(binding.feedbackScene.children).toHaveLength(0);
    expect(mesh.visible).toBe(false);
    sourceMaterial.dispose();
    mesh.geometry.dispose();
  });

  test('supports allocation-free feedback gating', () => {
    const binding = new VirtualPomSceneBinding({
      camera: new THREE.PerspectiveCamera(), textures, feedbackPixelScale: new THREE.Vector2(),
      capacity: 1, createPair: () => pair(),
    });
    expect(binding.isFeedbackActive()).toBe(true);
    binding.setFeedbackEnabled(false);
    expect(binding.isFeedbackActive()).toBe(false);
    binding.dispose();
  });
});
