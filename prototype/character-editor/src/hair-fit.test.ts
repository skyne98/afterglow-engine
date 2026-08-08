import { describe, expect, test } from 'bun:test';
import { HairFitRuntime, type HairFitDocument } from './hair-fit.ts';

function fixture(): HairFitDocument {
  return {
    version: 4,
    driverVertexCount: 4,
    driverNeutral: [
      0, 0, 0,
      4, 0, 0,
      0, 6, 0,
      0, 0, 8,
    ],
    targets: {
      wide: [1, 2, 0, 0],
    },
    proxy: {
      id: 'proxy',
      label: 'Proxy support',
      mesh: 'Hair-proxy-support',
      vertexCount: 4,
      parents: [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3],
      weights: [1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0],
      offsets: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
      scales: [
        [0, 1, 2, 0],
        [0, 2, 3, 1],
        [0, 3, 4, 2],
      ],
      neutralMaximumError: 0,
    },
    styles: [{
      id: 'test',
      label: 'Test',
      mesh: 'Hair-test',
      vertexCount: 1,
      sources: [1],
      parents: [0, 1, 2],
      weights: [-0.25, 1, 0.25],
      offsets: [0.5, -0.5, 1],
      scales: [
        [0, 1, 2, 0],
        [0, 2, 3, 1],
        [0, 3, 4, 2],
      ],
      neutralMaximumError: 0,
    }],
  };
}

describe('HairFitRuntime', () => {
  test('applies signed SurfaceWrap values and incremental targets', () => {
    const runtime = new HairFitRuntime(fixture(), ['wide']);
    const style = runtime.style('test')!;
    const output = new Float32Array(3);

    runtime.fit(style, output);
    expect([...output]).toEqual([5, 2, -0.5]);

    expect(runtime.setTarget(0, 0.25)).toBe(true);
    runtime.fit(style, output);
    expect([...output]).toEqual([5.625, 2, -0.5]);

    expect(runtime.setTarget(0, 0)).toBe(true);
    runtime.fit(style, output);
    expect([...output]).toEqual([5, 2, -0.5]);
    expect(runtime.setTarget(0, 0)).toBe(false);

  });

  test('rejects out-of-range parent and scale indices', () => {
    const parentDocument = fixture();
    parentDocument.styles[0].parents[2] = 4;
    expect(() => new HairFitRuntime(parentDocument, ['wide'])).toThrow('surface parent');

    const scaleDocument = fixture();
    scaleDocument.styles[0].scales[0][1] = 4;
    expect(() => new HairFitRuntime(scaleDocument, ['wide'])).toThrow('surface scale');

    const sourceDocument = fixture();
    sourceDocument.styles[0].sources![0] = 2;
    expect(() => new HairFitRuntime(sourceDocument, ['wide'])).toThrow('surface source');
  });
});
