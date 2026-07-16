import { describe, expect, test } from 'bun:test';
import { POM_UV_WGSL } from './surface-detail.ts';

describe('low-core POM shader', () => {
  test('has bounded adaptive work, distance fade, and explicit-LOD height reads', () => {
    expect(POM_UV_WGSL).toContain('viewDistance >= maxDistance');
    expect(POM_UV_WGSL).toContain('smoothstep(maxDistance * 0.65, maxDistance');
    expect(POM_UV_WGSL).toContain('i < layerCount');
    expect(POM_UV_WGSL).toContain('textureSampleLevel(heightTexture, heightSampler');
    expect(POM_UV_WGSL).not.toContain('selfShadow');
  });

  test('selects the configured low/high layer endpoints by view angle', () => {
    const layers = (viewZ: number, low = 8, high = 32) =>
      Math.max(low, Math.min(high, Math.round(high + (low - high) * Math.abs(viewZ))));
    expect(layers(1)).toBe(8);
    expect(layers(0.5)).toBe(20);
    expect(layers(0)).toBe(32);
  });

  test('the dungeon constants remain inside the measured 680M tier', async () => {
    const source = await Bun.file(new URL('../dungeon.ts', import.meta.url)).text();
    expect(source).toContain('POM_MIN_LAYERS=8,POM_MAX_LAYERS=32');
    expect(source).toContain('POM_HEIGHT_SCALE=.012,POM_MAX_DISTANCE=3.25');
    expect(source).toContain("heightSource:'resident ambientCG AO'");
  });

  test('ships matching resident height assets at their source aspect ratios', async () => {
    const expected: Record<string, [number, number]> = {
      Rock064: [1024, 1024], Ground103: [1024, 1024], PavingStones150: [1024, 512],
    };
    for (const [name, dimensions] of Object.entries(expected)) {
      const bytes = new Uint8Array(await Bun.file(new URL(`../dungeon-height/${name}_Height.png`, import.meta.url)).arrayBuffer());
      expect(String.fromCharCode(...bytes.subarray(1, 4))).toBe('PNG');
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      expect([view.getUint32(16), view.getUint32(20)]).toEqual(dimensions);
    }
  });
});
