import { describe, expect, test } from 'bun:test';
import { RendererSeal, warmRendererVariants } from './renderer-seal.ts';

describe('RendererSeal', () => {
  test('counts pipeline creation and reports only post-seal violations', () => {
    let renders = 0;
    let computes = 0;
    const backend = {
      createRenderPipeline() { renders++; },
      createComputePipeline() { computes++; },
    };
    const seal = new RendererSeal(backend);
    backend.createRenderPipeline({}, []);
    backend.createComputePipeline({}, {});
    expect(seal.violations).toBe(0);
    seal.seal();
    backend.createRenderPipeline({}, []);
    expect(renders).toBe(2);
    expect(computes).toBe(1);
    expect(seal.renderPipelineViolations).toBe(1);
    expect(() => seal.assertNoViolations()).toThrow('after seal');
  });

  test('warms every declared scene/camera pair before seal', async () => {
    const seen: unknown[] = [];
    const renderer = { async compileAsync(scene: unknown, camera: unknown) { seen.push(scene, camera); } };
    const variants = [{ scene: { id: 1 }, camera: { id: 2 } }, { scene: { id: 3 }, camera: { id: 4 } }];
    await warmRendererVariants(renderer as never, variants as never);
    expect(seen).toEqual([{ id: 1 }, { id: 2 }, { id: 3 }, { id: 4 }]);
  });
});
