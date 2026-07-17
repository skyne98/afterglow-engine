import { describe, expect, test } from 'bun:test';
import * as THREE from 'three/webgpu';
import type { OptimizedGltfAsset, GltfMaterialTextureLayout } from './asset-store.ts';
import { VirtualGltfBinding } from './virtual-gltf-binding.ts';
import type { VirtualGltfMaterialOptions, VirtualGltfMaterialPair } from './virtual-texture-material.ts';
import { VirtualTextureStore } from './virtual-texture.ts';

const loader = { async read() { return new Uint8Array(); }, poll() {} };
const makeStore = (): VirtualTextureStore =>
  new VirtualTextureStore(loader, async () => new Uint8Array(136 * 136 * 4));
const layout = (index: number, image: number | null): GltfMaterialTextureLayout => ({
  index, name: 'duplicate', baseColorImage: image, metallicRoughnessImage: null,
  normalImage: null, emissiveImage: null,
  baseColorSampling: image === null ? null : {
    image, texCoord: 0, offset: [0, 0], rotation: 0, scale: [1, 1],
    wrapS: 10497, wrapT: 10497, minFilter: 9987, magFilter: 9729,
  },
  metallicRoughnessSampling: null, normalSampling: null, emissiveSampling: null,
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
    red.alphaTest = 0.4;
    const first = new THREE.Mesh(new THREE.BoxGeometry(), red);
    const second = new THREE.Mesh(new THREE.BoxGeometry(), green);
    scene.add(first, second);
    const vt = makeStore();
    vt.loadTexture('0', { width: 128, height: 128 });
    vt.loadTexture('1', { width: 128, height: 128 });
    const images: number[] = [], colors: number[] = [];
    const captured: VirtualGltfMaterialOptions[] = [];
    const redLayout = layout(0, 0);
    redLayout.baseColorSampling = {
      image: 0, texCoord: 1, offset: [0.25, 0.5], rotation: 0.2, scale: [2, 3],
      wrapS: 33648, wrapT: 33648, minFilter: 9987, magFilter: 9729,
    };
    const binding = VirtualGltfBinding.create(
      asset(scene, [redLayout, layout(1, 1)], new Map([[red, 0], [green, 1]])), vt,
      {
        primitiveCapacity: 2, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.PerspectiveCamera(),
        feedbackPixelScale: new THREE.Vector2(1, 1),
        resolveImage(index) { images.push(index); return vt.getEntry(String(index)); },
        pairFactory(_store, _set, _pixel, options) {
          colors.push(options.baseColorFactor?.[0] ?? -1); captured.push(options); return pair();
        },
      },
    );
    expect(images).toEqual([0, 1]);
    expect(colors).toEqual([1, 0]);
    expect(captured[0]?.sampling?.albedo?.channel).toBe(1);
    expect(captured[0]?.sampling?.albedo?.addressMode).toBe(2);
    const cosine = Math.cos(0.2), sine = Math.sin(0.2);
    expect(captured[0]?.sampling?.albedo?.matrix).toEqual([
      2 * cosine, 2 * sine, 0, -3 * sine, 3 * cosine, 0, 0.25, 0.5, 1,
    ]);
    expect(captured[0]?.alphaTest).toBe(0.4);
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
    binding.endFeedbackPass(1);
    expect(first.visible).toBe(false);
    expect(second.visible).toBe(true);
    expect(unbound.visible).toBe(true);
    binding.dispose();
    expect(first.visible).toBe(false);
    expect(second.visible).toBe(false);
    expect(unbound.visible).toBe(true);
  });

  test('does not dispose a replaced material texture retained by an unbound material', () => {
    const scene = new THREE.Group();
    const shared = new THREE.Texture();
    let disposed = 0;
    shared.dispose = (): void => { disposed++; };
    const bound = new THREE.MeshStandardMaterial({ normalMap: shared });
    const retained = new THREE.MeshStandardMaterial({ normalMap: shared });
    scene.add(new THREE.Mesh(new THREE.BoxGeometry(), bound), new THREE.Mesh(new THREE.BoxGeometry(), retained));
    const vt = makeStore();
    vt.loadTexture('0', { width: 128, height: 128 });
    const binding = VirtualGltfBinding.create(
      asset(scene, [layout(0, 0), layout(1, null)], new Map([[bound, 0], [retained, 1]])), vt,
      {
        primitiveCapacity: 2, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.Camera(),
        feedbackPixelScale: new THREE.Vector2(1, 1), resolveImage() { return vt.getEntry('0'); },
        pairFactory() { return pair(); },
      },
    );
    expect(disposed).toBe(0);
    binding.dispose();
  });

  test('accepts a mesh-bearing fallback asset without material metadata', () => {
    const scene = new THREE.Group();
    scene.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial()));
    const binding = VirtualGltfBinding.create(asset(scene, [], new Map()), makeStore(), {
      primitiveCapacity: 1, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.Camera(),
      feedbackPixelScale: new THREE.Vector2(1, 1), resolveImage() { return undefined; },
    });
    binding.beginFeedbackPass(0);
    expect(scene.children[0]?.visible).toBe(false);
    binding.endFeedbackPass(0);
    expect(scene.children[0]?.visible).toBe(true);
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

  test('rejects sampler modes that the shared VT atlas cannot preserve', () => {
    const scene = new THREE.Group();
    const material = new THREE.MeshStandardMaterial();
    scene.add(new THREE.Mesh(new THREE.BoxGeometry(), material));
    const vt = makeStore();
    vt.loadTexture('0', { width: 128, height: 128 });
    const invalidLayout = layout(0, 0);
    if (invalidLayout.baseColorSampling) invalidLayout.baseColorSampling = {
      ...invalidLayout.baseColorSampling, wrapT: 33071,
    };
    expect(() => VirtualGltfBinding.create(
      asset(scene, [invalidLayout], new Map([[material, 0]])), vt,
      {
        primitiveCapacity: 1, feedbackScene: new THREE.Scene(), feedbackCamera: new THREE.Camera(),
        feedbackPixelScale: new THREE.Vector2(1, 1), resolveImage() { return vt.getEntry('0'); },
        pairFactory() { return pair(); },
      },
    )).toThrow('identical S/T');
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
