import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import type { OptimizedGltfAsset, GltfMaterialTextureLayout } from './asset-store.ts';
import { VirtualGltfBinding } from './virtual-gltf-binding.ts';
import type { VirtualGltfMaterialPair } from './virtual-texture-material.ts';
import { VirtualTextureStore } from './virtual-texture.ts';

const loader = { async read() { return new Uint8Array(); }, poll() {} };
const makeStore = (): VirtualTextureStore =>
  new VirtualTextureStore(loader, async () => new Uint8Array(136 * 136 * 4));
const layout = (index: number, image: number | null): GltfMaterialTextureLayout => ({
  index, name: 'duplicate', baseColorImage: image, metallicRoughnessImage: null,
  normalImage: null, emissiveImage: null,
});
function asset(scene: THREE.Group, layouts: GltfMaterialTextureLayout[], indices: ReadonlyMap<THREE.Material, number>): OptimizedGltfAsset {
  return { scene, animations: [], materialIndices: indices, materialTextures: layouts, meshOptimization: [] };
}
function pair(passCount = 1): VirtualGltfMaterialPair {
  const feedbackMaterials = Array.from({ length: passCount }, () => new THREE.MeshBasicNodeMaterial());
  const feedbackMaterial = feedbackMaterials[0];
  if (!feedbackMaterial) throw new Error('test pair requires feedback');
  return {
    material: new THREE.MeshStandardNodeMaterial(), feedbackMaterial, feedbackMaterials,
    feedbackEntries: [],
  };
}

describe('VirtualGltfBinding', () => {
  test('binds duplicate names by stable parser index and preserves source factors', () => {
    const scene = new THREE.Group();
    const red = new THREE.MeshStandardMaterial({ color: 0xff0000 });
    const green = new THREE.MeshStandardMaterial({ color: 0x00ff00 });
    red.name = green.name = 'same';
    const first = new THREE.Mesh(new THREE.BoxGeometry(), red);
    const second = new THREE.Mesh(new THREE.BoxGeometry(), green);
    scene.add(first, second);
    const vt = makeStore();
    vt.loadTexture('0', { width: 128, height: 128 });
    vt.loadTexture('1', { width: 128, height: 128 });
    const images: number[] = [], colors: number[] = [];
    const binding = VirtualGltfBinding.create(
      asset(scene, [layout(0, 0), layout(1, 1)], new Map([[red, 0], [green, 1]])), vt,
      {
        primitiveCapacity: 2, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.PerspectiveCamera(),
        feedbackPixelScale: new THREE.Vector2(1, 1),
        resolveImage(index) { images.push(index); return vt.getEntry(String(index)); },
        pairFactory(_store, _set, _pixel, options) {
          colors.push(options.baseColorFactor?.[0] ?? -1); return pair();
        },
      },
    );
    expect(images).toEqual([0, 1]);
    expect(colors).toEqual([1, 0]);
    expect(first.material).not.toBe(second.material);
    binding.dispose();
  });

  test('shares one material pair and restores visibility around feedback', () => {
    const scene = new THREE.Group();
    const source = new THREE.MeshStandardMaterial();
    const first = new THREE.Mesh(new THREE.BoxGeometry(), source);
    const second = new THREE.Mesh(new THREE.BoxGeometry(), source);
    const unbound = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
    first.visible = false;
    scene.add(first, second, unbound);
    const vt = makeStore();
    vt.loadTexture('0', { width: 128, height: 128 });
    let factories = 0;
    const binding = VirtualGltfBinding.create(
      asset(scene, [layout(0, 0), layout(1, null)], new Map([[source, 0], [unbound.material, 1]])), vt,
      {
        primitiveCapacity: 3, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.Camera(), feedbackPixelScale: new THREE.Vector2(1, 1),
        resolveImage() { return vt.getEntry('0'); }, pairFactory() { factories++; return pair(2); },
      },
    );
    expect(factories).toBe(1);
    expect(binding.feedbackPassCount).toBe(2);
    expect(first.material).toBe(second.material);
    binding.beginFeedbackPass(1);
    expect(first.visible).toBe(false);
    expect(unbound.visible).toBe(false);
    expect(first.material).toBe(second.material);
    binding.endFeedbackPass();
    expect(first.visible).toBe(false);
    expect(second.visible).toBe(true);
    expect(unbound.visible).toBe(true);
    binding.dispose();
  });

  test('rejects missing indices and rolls back prior replacements', () => {
    const scene = new THREE.Group();
    const indexed = new THREE.MeshStandardMaterial();
    const missing = new THREE.MeshStandardMaterial();
    const first = new THREE.Mesh(new THREE.BoxGeometry(), indexed);
    scene.add(first, new THREE.Mesh(new THREE.BoxGeometry(), missing));
    const vt = makeStore();
    vt.loadTexture('0', { width: 128, height: 128 });
    expect(() => VirtualGltfBinding.create(
      asset(scene, [layout(0, 0)], new Map([[indexed, 0]])), vt,
      {
        primitiveCapacity: 2, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.Camera(), feedbackPixelScale: new THREE.Vector2(1, 1),
        resolveImage() { return vt.getEntry('0'); }, pairFactory() { return pair(); },
      },
    )).toThrow('no stable parser index');
    expect(first.material).toBe(indexed);
  });

  test('fails deterministically when primitive capacity is exceeded', () => {
    const scene = new THREE.Group();
    const material = new THREE.MeshStandardMaterial();
    scene.add(new THREE.Mesh(new THREE.BoxGeometry(), material), new THREE.Mesh(new THREE.BoxGeometry(), material));
    expect(() => VirtualGltfBinding.create(
      asset(scene, [layout(0, null)], new Map([[material, 0]])), makeStore(),
      {
        primitiveCapacity: 1, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.Camera(), feedbackPixelScale: new THREE.Vector2(1, 1),
        resolveImage() { return undefined; }, pairFactory() { return pair(); },
      },
    )).toThrow('capacity exceeded');
  });
});
