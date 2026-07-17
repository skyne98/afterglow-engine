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
    const modelChunk = model.chunks[0];
    if (!modelChunk) throw new Error('packed model has no raw chunk');
    expect(modelChunk.meta.type).toBe('Raw');
    expect(modelChunk.compression).toBe('None');
    expect(modelChunk.uncompressedSize).toBeGreaterThan(2_000_000n);
    const modelBytes = new Uint8Array(await file.slice(
      Number(modelChunk.offset),
      Number(modelChunk.offset + modelChunk.uncompressedSize),
    ).arrayBuffer());
    const jsonLength = new DataView(modelBytes.buffer).getUint32(12, true);
    const document = JSON.parse(new TextDecoder().decode(modelBytes.subarray(20, 20 + jsonLength)));
    expect(document.images).toEqual([]);
    expect(document.textures).toEqual([]);
    expect(document.extensions.AFTERGLOW_virtual_textures.materials).toHaveLength(1);
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

  test('uses canonical ownership, stable material indices, exact animated feedback, and inertial controls', async () => {
    const source = await Bun.file(new URL('../rigged-vt-demo.ts', import.meta.url)).text();
    expect(source).toContain('EngineRuntime.forScene({');
    expect(source).toContain('await BigAssetSession.open({');
    expect(source).toContain('session.createAssetStore(meshopt, 4, 4)');
    expect(source).toContain("assetStore.loadOptimizedGLTF('model.glb', new GLTFLoader())");
    expect(source).toContain("assetStore.loadOptimizedGLTF('model-2.glb', new GLTFLoader())");
    expect(source).toContain('VirtualTextureFeedbackCoordinator(');
    expect(source).toContain('VirtualGltfBinding.create(firstAsset');
    expect(source).toContain('VirtualGltfBinding.create(secondAsset');
    expect(source).toContain('new ModelPrimitives(MODEL_CAPACITY)');
    expect(source).toContain('groundDeformedModel(firstPivot');
    expect(source).toContain('BoundedKeyboardInput');
    expect(source).toContain('DemoInputAction.OrbitRight');
    expect(source).toContain('Math.exp(-7 * dt)');
    expect(source).toContain('rendererHost.renderer.shadowMap.enabled = true');
    expect(source).toContain('keyLight.castShadow = true');
    expect(source).toContain('floor.receiveShadow = true');
    expect(source).toContain('mesh.castShadow = true');
    expect(source).not.toContain('window.THREE');
    expect(source).not.toContain('new BigContainerAssetLoader');
    expect(source).not.toContain('new VirtualTextureFeedbackPass');
    const license = await Bun.file(new URL('../../../assets/rigged-vt/LICENSE.txt', root)).text();
    expect(license).toContain('CC-BY-4.0');
    expect(license).toContain('KallMor');
    const dragonLicense = await Bun.file(new URL('../../../assets/rigged-vt/LICENSE-DRAGON.txt', root)).text();
    expect(dragonLicense).toContain('Spooky Iluha');
    expect(dragonLicense).toContain('CC-BY-NC-4.0');
  });
});
