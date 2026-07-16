import { describe, expect, test } from 'bun:test';
import { parseBigHeader } from './big-parser.ts';

const root = new URL('../', import.meta.url);

describe('pipeline-packed rigged VT demo', () => {
  test('ships both raw models and all extracted virtual images in one BIG', async () => {
    const file = Bun.file(new URL('../rigged-vt.big', import.meta.url));
    const prefix = new Uint8Array(await file.slice(0, 16).arrayBuffer());
    const dataOffset = Number(new DataView(prefix.buffer).getBigUint64(8, true));
    const headerBytes = new Uint8Array(await file.slice(0, dataOffset).arrayBuffer());
    const { header } = parseBigHeader(headerBytes);
    const model = header.assets.find(asset => asset.name === 'model.glb')!;
    expect(model.assetType).toBe('Mesh');
    expect(model.chunks).toHaveLength(1);
    expect(model.chunks[0].meta.type).toBe('Raw');
    expect(model.chunks[0].compression).toBe('None');
    expect(model.chunks[0].uncompressedSize).toBeGreaterThan(2_000_000n);
    for (let image = 0; image < 3; image++) {
      const texture = header.assets.find(asset => asset.name === `model.glb#image-${image}`)!;
      expect(texture.assetType).toBe('VirtualTexture');
      expect([texture.virtualTexture!.width, texture.virtualTexture!.height]).toEqual([256, 256]);
      expect(texture.virtualTexture!.encoding).toBe('Basis');
      expect(texture.virtualTexture!.tail).not.toBeNull();
    }
    const second = header.assets.find(asset => asset.name === 'model-2.glb')!;
    expect(second.assetType).toBe('Mesh');
    expect(second.chunks[0].meta.type).toBe('Raw');
    const dragonTextures = header.assets.filter(asset => asset.name.startsWith('model-2.glb#image-'));
    expect(dragonTextures).toHaveLength(45);
    expect([dragonTextures[0].virtualTexture!.width, dragonTextures[0].virtualTexture!.height]).toEqual([4096, 4096]);
  });

  test('uses ordinary packed loading, runtime meshopt, exact animated feedback, and inertial controls', async () => {
    const source = await Bun.file(new URL('../rigged-vt-demo.ts', import.meta.url)).text();
    expect(source).toContain("new BigContainerAssetLoader(rangeLoader, 'rigged-vt.big', header)");
    expect(source).toContain("assetStore.loadOptimizedGLTF('model.glb', new GLTFLoader())");
    expect(source).toContain("assetStore.loadOptimizedGLTF('model-2.glb', new GLTFLoader())");
    expect(source).toContain("albedo: 'model.glb#image-0'");
    expect(source).toContain('secondGltf.materialTextures.map(layout => [layout.name, layout])');
    expect(source).toContain('secondMaxFeedbackChannels');
    expect(source).toContain("if (keyName === '1') setActiveModel(1)");
    expect(source).toContain("else if (keyName === '2') setActiveModel(2)");
    expect(source).toContain('skinnedMeshes[index].material = enabled ? pair.feedbackMaterial : visibleMaterials[index]');
    expect(source).toContain('camera.layers.set(MODEL_LAYER)');
    expect(source).toContain('mesh.getVertexPosition(index, deformedVertex)');
    expect(source).toContain('modelPivot.position.y -= animatedBounds.min.y');
    expect(source).toContain("keys.has('d') ? 1 : 0");
    expect(source).toContain("keys.has('s') ? 1 : 0");
    expect(source).toContain('Math.exp(-7 * dt)');
    expect(source).toContain('renderer.shadowMap.enabled = true');
    expect(source).toContain('key.castShadow = true');
    expect(source).toContain('floor.receiveShadow = true');
    expect(source).toContain('object.castShadow = true');
    expect(source).toContain('renderer.shadowMap.enabled = false');
    const license = await Bun.file(new URL('../../../assets/rigged-vt/LICENSE.txt', root)).text();
    expect(license).toContain('CC-BY-4.0');
    expect(license).toContain('KallMor');
    const dragonLicense = await Bun.file(new URL('../../../assets/rigged-vt/LICENSE-DRAGON.txt', root)).text();
    expect(dragonLicense).toContain('Spooky Iluha');
    expect(dragonLicense).toContain('CC-BY-NC-4.0');
  });
});
