import { describe, expect, test } from 'bun:test';
import {
  POM_SELF_SHADOW_WGSL,
  POM_UV_WGSL,
  assertPomGeneratedWgsl,
  marchPomReference,
  pomDistanceFade,
  pomLayerCount,
  type PomReferenceResult,
} from './surface-detail.ts';
import { parseHeightR16 } from './height-texture.ts';

const out = (): PomReferenceResult => ({ u: 0, v: 0, depth: 0, layers: 0, samples: 0, hit: false });
const constant = (height: number) => (_u: number, _v: number) => height;
const march = (
  height: (u: number, v: number) => number,
  options: Partial<{
    u: number; v: number; viewX: number; viewY: number; viewZ: number;
    scale: number; maxOffsetRatio: number; minLayers: number; maxLayers: number; maxDistance: number; distance: number;
  }> = {},
) => marchPomReference(
  height,
  options.u ?? 0.5, options.v ?? 0.5,
  options.viewX ?? 1, options.viewY ?? 0, options.viewZ ?? 1,
  options.scale ?? 0.2,
  options.maxOffsetRatio ?? 2,
  options.minLayers ?? 8, options.maxLayers ?? 32,
  options.maxDistance ?? 4, options.distance ?? 1,
  out(),
);

describe('low-core POM shader contract', () => {
  test('has bounded adaptive work, distance fade, and explicit-LOD height reads', () => {
    expect(POM_UV_WGSL).toContain('viewDistance >= maxDistance');
    expect(POM_UV_WGSL).toContain('smoothstep(maxDistance * 0.65, maxDistance');
    expect(POM_UV_WGSL).toContain('i < layerCount');
    expect(POM_UV_WGSL).toContain('textureSampleLevel(heightTexture, heightSampler');
    expect(POM_UV_WGSL).toContain('boundedSlope');
    expect(POM_UV_WGSL).not.toContain('selfShadow');
  });

  test('bounds a physical-height light ray and rejects back-facing lights', () => {
    expect(POM_SELF_SHADOW_WGSL).toContain('l.z <= 0.001');
    expect(POM_SELF_SHADOW_WGSL).toContain('min(requestedSteps, 16u)');
    expect(POM_SELF_SHADOW_WGSL).toContain('remainingHeight = 1.0 - hitHeight');
    expect(POM_SELF_SHADOW_WGSL).toContain('terrainHeight > rayHeight + bias');
    expect(POM_SELF_SHADOW_WGSL).toContain('boundedSlope');
  });

  test('converts physical height to ray depth instead of treating white as deep', () => {
    expect(POM_UV_WGSL).toContain('surfaceDepth = 1.0 - textureSampleLevel');
    expect(POM_UV_WGSL).toContain('previousSurfaceDepth = 1.0 - textureSampleLevel');
    expect(POM_UV_WGSL).toContain('if (surfaceDepth < currentDepth)');
  });

  const generated = (main: string) => `fn pomMarchUV() {}\n@fragment\nfn main() {\n${main}\n}`;
  const samples = 'a = vtSampleFromLevel();\nb = vtSampleFromLevel();\nc = vtSampleFromLevel();';

  test('accepts one geometric-TBN march before all three linked PBR samples', () => {
    expect(() => assertPomGeneratedWgsl(generated(`uv = pomMarchUV(positionViewDirection * mat3x3(t,b,n));\n${samples}`))).not.toThrow();
  });

  test('rejects duplicate marches, late initialization, and a normal-mapped TBN cycle', () => {
    expect(() => assertPomGeneratedWgsl(generated(`uv = pomMarchUV(x * mat3x3(t,b,n));\nuv2 = pomMarchUV(x * mat3x3(t,b,n));\n${samples}`))).toThrow('exactly once');
    expect(() => assertPomGeneratedWgsl(generated(`${samples}\nuv = pomMarchUV(x * mat3x3(t,b,n));`))).toThrow('before displaced UV');
    expect(() => assertPomGeneratedWgsl(generated(`uv = pomMarchUV(x * TBNViewMatrix);\n${samples}`))).toThrow('geometric TBN');
  });

  test('rejects missing or unlinked PBR samples', () => {
    expect(() => assertPomGeneratedWgsl('fn pomMarchUV() {}')).toThrow('fragment entry');
    expect(() => assertPomGeneratedWgsl(generated(samples))).toThrow('march invocation');
    expect(() => assertPomGeneratedWgsl(generated('uv = pomMarchUV(x * mat3x3(t,b,n));\na = vtSampleFromLevel();'))).toThrow('expected 3');
  });
});

describe('POM layer and distance math', () => {
  test('selects the configured layer endpoints and midpoint', () => {
    expect(pomLayerCount(1, 8, 32)).toBe(8);
    expect(pomLayerCount(0.5, 8, 32)).toBe(20);
    expect(pomLayerCount(0, 8, 32)).toBe(32);
  });

  test('normalizes reversed and invalid layer ranges deterministically', () => {
    expect(pomLayerCount(1, 32, 8)).toBe(8);
    expect(pomLayerCount(0, 32, 8)).toBe(8);
    expect(pomLayerCount(1, 0, 0)).toBe(1);
  });

  test('distance fade is flat, smooth, and exactly zero at cutoff', () => {
    expect(pomDistanceFade(0, 4)).toBe(1);
    expect(pomDistanceFade(2.6, 4)).toBe(1);
    expect(pomDistanceFade(3.3, 4)).toBeCloseTo(0.5, 12);
    expect(pomDistanceFade(4, 4)).toBe(0);
    expect(pomDistanceFade(5, 4)).toBe(0);
    expect(pomDistanceFade(0, 0)).toBe(1); // nonpositive cutoff disables radial fading
  });

  test('fade decreases monotonically through its transition', () => {
    let previous = 1;
    for (let distance = 2.6; distance <= 4; distance += 0.05) {
      const current = pomDistanceFade(distance, 4);
      expect(current).toBeLessThanOrEqual(previous + 1e-12);
      previous = current;
    }
  });
});

describe('POM oracle on analytically predictable height fields', () => {
  test('a fully raised plane intersects at the undisplaced top surface', () => {
    const result = march(constant(1));
    expect(result.hit).toBe(true);
    expect(result.depth).toBeCloseTo(0, 12);
    expect(result.u).toBeCloseTo(0.5, 12);
    expect(result.v).toBeCloseTo(0.5, 12);
  });

  test('a fully recessed plane traverses the entire configured relief depth', () => {
    const result = march(constant(0));
    expect(result.hit).toBe(false);
    expect(result.depth).toBeCloseTo(1, 12);
    expect(result.u).toBeCloseTo(0.3, 12); // viewX/viewZ=1, full scale=0.2
  });

  test('a half-height plane intersects at exactly half displacement', () => {
    for (const layers of [4, 8, 16, 32, 64]) {
      const result = march(constant(0.5), { minLayers: layers, maxLayers: layers });
      expect(result.hit).toBe(true);
      expect(result.depth).toBeCloseTo(0.5, 12);
      expect(result.u).toBeCloseTo(0.4, 12);
    }
  });

  test('displacement is monotonic with physical height', () => {
    let previousU = -Infinity;
    for (const height of [0, 0.1, 0.25, 0.5, 0.75, 0.9, 1]) {
      const result = march(constant(height));
      expect(result.u).toBeGreaterThanOrEqual(previousU - 1e-12);
      expect(result.u).toBeCloseTo(0.3 + height * 0.2, 10);
      previousU = result.u;
    }
  });

  test('positive and negative view directions shift UV in opposite directions', () => {
    const left = march(constant(0), { viewX: 1 });
    const right = march(constant(0), { viewX: -1 });
    expect(left.u).toBeCloseTo(0.3, 12);
    expect(right.u).toBeCloseTo(0.7, 12);
  });

  test('vertical view displacement affects V independently', () => {
    const result = march(constant(0), { viewX: 0, viewY: 1 });
    expect(result.u).toBeCloseTo(0.5, 12);
    expect(result.v).toBeCloseTo(0.3, 12);
  });

  test('head-on views preserve UV for every height while retaining bounded work', () => {
    for (const height of [0, 0.25, 0.5, 0.75, 1]) {
      const result = march(constant(height), { viewX: 0, viewY: 0, viewZ: 1 });
      expect(result.u).toBeCloseTo(0.5, 12);
      expect(result.v).toBeCloseTo(0.5, 12);
      expect(result.samples).toBeLessThanOrEqual(result.layers + 1);
    }
  });

  test('height samples are clamped to the physical [0,1] domain', () => {
    expect(march(constant(2)).u).toBeCloseTo(march(constant(1)).u, 12);
    expect(march(constant(-2)).u).toBeCloseTo(march(constant(0)).u, 12);
  });

  test('height scale changes displacement linearly on a flat field', () => {
    expect(march(constant(0), { scale: 0.05 }).u).toBeCloseTo(0.45, 12);
    expect(march(constant(0), { scale: 0.1 }).u).toBeCloseTo(0.4, 12);
    expect(march(constant(0), { scale: 0.2 }).u).toBeCloseTo(0.3, 12);
  });

  test('distance fade scales displacement without changing the height intersection', () => {
    const full = march(constant(0.5), { distance: 1 });
    const half = march(constant(0.5), { distance: 3.3 });
    const none = march(constant(0.5), { distance: 4 });
    expect(full.depth).toBeCloseTo(0.5, 12);
    expect(half.depth).toBeCloseTo(0.5, 12);
    expect(full.u).toBeCloseTo(0.4, 12);
    expect(half.u).toBeCloseTo(0.45, 12);
    expect(none.u).toBeCloseTo(0.5, 12);
    expect(none.samples).toBe(0);
  });

  test('a raised step occludes a recessed floor near the analytic boundary', () => {
    // Start over a recessed right half and march left into a fully raised step.
    const step = (u: number) => u < 0.5 ? 1 : 0;
    const result = march(step, { u: 0.75, scale: 0.5, minLayers: 32, maxLayers: 32 });
    expect(result.hit).toBe(true);
    expect(result.u).toBeGreaterThan(0.47);
    expect(result.u).toBeLessThan(0.51);
    expect(result.depth).toBeGreaterThan(0.45);
    expect(result.depth).toBeLessThan(0.6);
  });

  test('a thin raised ridge cannot be tunneled through at maximum layer count', () => {
    const ridge = (u: number) => u >= 0.48 && u <= 0.52 ? 1 : 0;
    const result = march(ridge, { u: 0.75, scale: 0.5, minLayers: 32, maxLayers: 32 });
    expect(result.hit).toBe(true);
    expect(result.u).toBeGreaterThanOrEqual(0.47);
    expect(result.u).toBeLessThanOrEqual(0.53);
  });

  test('solves a rising linear ramp at its analytic ray intersection', () => {
    // h(u)=u, ray u=0.5-0.2d, surface depth=1-u=0.5+0.2d;
    // d=0.5+0.2d => d=0.625 and u=0.375.
    const result = march(u => u, { minLayers: 64, maxLayers: 64 });
    expect(result.hit).toBe(true);
    expect(result.depth).toBeCloseTo(0.625, 10);
    expect(result.u).toBeCloseTo(0.375, 10);
  });

  test('solves a descending linear ramp at its analytic ray intersection', () => {
    // h(u)=1-u, surface depth=u=0.5-0.2d; d=0.5/1.2.
    const expectedDepth = 0.5 / 1.2;
    const result = march(u => 1 - u, { minLayers: 64, maxLayers: 64 });
    expect(result.hit).toBe(true);
    expect(result.depth).toBeCloseTo(expectedDepth, 10);
    expect(result.u).toBeCloseTo(0.5 - 0.2 * expectedDepth, 10);
  });

  test('offset limiting bounds nearly tangent views instead of exploding UVs', () => {
    const result = march(constant(0), { viewX: 1, viewZ: 0, maxOffsetRatio: 2 });
    expect(result.u).toBeCloseTo(0.1, 10);
    const tighter = march(constant(0), { viewX: 1, viewZ: 0, maxOffsetRatio: 0.5 });
    expect(tighter.u).toBeCloseTo(0.4, 10);
  });

  test('a circular raised island is intersected symmetrically from either side', () => {
    const island = (u: number, v: number) => Math.hypot(u - 0.5, v - 0.5) <= 0.1 ? 1 : 0;
    const fromRight = march(island, { u: 0.75, viewX: 1, scale: 0.5, minLayers: 64, maxLayers: 64 });
    const fromLeft = march(island, { u: 0.25, viewX: -1, scale: 0.5, minLayers: 64, maxLayers: 64 });
    expect(fromRight.hit).toBe(true);
    expect(fromLeft.hit).toBe(true);
    expect(fromRight.u).toBeCloseTo(1 - fromLeft.u, 2);
    expect(fromRight.depth).toBeCloseTo(fromLeft.depth, 10);
  });

  test('sample work never exceeds one initial read plus the selected layer count', () => {
    for (const viewZ of [0.05, 0.25, 0.5, 0.75, 1]) {
      for (const height of [0, 0.25, 0.5, 0.75, 1]) {
        const result = march(constant(height), { viewX: Math.sqrt(1 - viewZ * viewZ), viewZ });
        expect(result.layers).toBeGreaterThanOrEqual(8);
        expect(result.layers).toBeLessThanOrEqual(32);
        expect(result.samples).toBeLessThanOrEqual(result.layers + 1);
      }
    }
  });

  test('zero view vectors and disabled scales fail closed to the base UV', () => {
    const zeroView = march(constant(0), { viewX: 0, viewY: 0, viewZ: 0 });
    const zeroScale = march(constant(0), { scale: 0 });
    expect([zeroView.u, zeroView.v, zeroView.samples]).toEqual([0.5, 0.5, 0]);
    expect([zeroScale.u, zeroScale.v, zeroScale.samples]).toEqual([0.5, 0.5, 0]);
  });
});

describe('Dungeon POM integration assets and limits', () => {
  test('constants remain inside the measured 680M tier', async () => {
    const source = await Bun.file(new URL('../dungeon.ts', import.meta.url)).text();
    expect(source).toContain('POM_MIN_LAYERS=8,POM_MAX_LAYERS=32');
    expect(source).toContain('POM_HEIGHT_SCALE=.05,POM_MAX_OFFSET_RATIO=2,POM_MAX_DISTANCE=0,POM_SHADOW_STEPS=8,POM_SHADOW_BIAS=.01,POM_SHADOW_STRENGTH=.82');
    expect(source).toContain("heightSource:'resident ambientCG displacement',heightFormat:'r32float-from-r16'");
    expect(source).toContain('function pomTbn(){const side=THREE.faceDirection');
    expect(source).toContain('THREE.normalViewGeometry.mul(side)');
    expect(source).toContain('THREE.positionViewDirection.mul(pomTbn())');
    expect(source).toContain('lightDirection.mul(pomTbn())');
    expect(source).toContain('directDiffuse.mulAssign(visibility)');
    expect(source).toContain('directSpecular.mulAssign(visibility)');
    expect(source).toContain('loadHeightTextureR16(THREE,renderer.backend.device');
    expect(source).toContain('assertHeightTextureGpuFormat(renderer.backend,height)');
    expect(source).not.toContain('TextureLoader().loadAsync(`dungeon-height');
    expect(source).toContain('THREE.vec2(1,-1)');
    expect(source).toContain('VT_QUALITY_BIAS=0, FEEDBACK_INTERVAL=8');
    expect(source).not.toContain('VT_LOD_BIAS');
    expect(source).toContain('feedbackPixelScale:uniform(feedbackPass.pixelScale)');
    expect(source).toContain('sampleUV=usePom?pomUV(set):gradientUV');
    expect(source).toContain('baseFeedbackMaterial=feedbackMaterial(set,false),pomFeedbackMaterial=feedbackMaterial(set,true)');
    expect(source).toContain('wall.feedbackMesh.material=pomEnabled?wall.pomFeedbackMaterial:wall.baseFeedbackMaterial');
    expect(source).toContain('trackTimestamp:false');
    expect(source).toContain('viewDir:pomViewDirection(),heightScale');
    expect(source).not.toContain('viewDir:THREE.parallaxDirection');
    expect(source).toContain('displacedUV.assign(sampleUV);resolvedMip.assign(mip)');
    expect(source).toContain('sampleEntryFromLevel(set.normal,resolvedMip,displacedUV)');
    expect(source).toContain('sampleEntryFromLevel(set.masks,resolvedMip,displacedUV)');
  });

  test('ships lossless runtime R16 displacement at source aspect ratios', async () => {
    const expected: Record<string, [number, number]> = {
      Rock064: [1024, 1024], Ground103: [1024, 1024], PavingStones150: [1024, 512],
    };
    for (const [name, dimensions] of Object.entries(expected)) {
      const buffer = await Bun.file(new URL(`../dungeon-height/${name}_Height.r16`, import.meta.url)).arrayBuffer();
      const asset = parseHeightR16(buffer);
      expect([asset.width, asset.height]).toEqual(dimensions);
      // Expanding an 8-bit source to 16-bit produces only 0xNNNN samples. The
      // official maps must retain values outside that quantized set.
      let fullPrecisionSamples = 0;
      for (const value of asset.pixels) if ((value & 0xff) !== (value >>> 8)) fullPrecisionSamples++;
      expect(fullPrecisionSamples).toBeGreaterThan(asset.pixels.length / 2);
    }
  });
});
