import { describe, expect, test } from 'bun:test';
import { VirtualTextureFeedbackPass } from './virtual-texture-feedback-pass.ts';
import { VT_FEEDBACK_WGSL } from './virtual-texture.ts';
import { createPackedPageTableLayout } from './virtual-texture-layout.ts';
import { encodeFeedback } from './virtual-texture-feedback.ts';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe('VirtualTextureFeedbackPass reuse', () => {
  test('selects mip from physical pixels while addressing a separate displaced UV', () => {
    expect(VT_FEEDBACK_WGSL).toContain('sampleUV: vec2f');
    expect(VT_FEEDBACK_WGSL).toContain('gradientUV: vec2f');
    expect(VT_FEEDBACK_WGSL).toContain('dpdx(gradientUV * virtualSize) * feedbackPixelScale.x');
    expect(VT_FEEDBACK_WGSL).toContain('dpdy(gradientUV * virtualSize) * feedbackPixelScale.y');
    expect(VT_FEEDBACK_WGSL).toContain('addressMode: u32');
    expect(VT_FEEDBACK_WGSL).toContain('addressed_uv = fract(sampleUV)');
    expect(VT_FEEDBACK_WGSL).not.toContain('dpdx(sampleUV');
  });

  test('tracks the exact reduced-feedback to physical-pixel derivative scale', () => {
    const pass = new VirtualTextureFeedbackPass(0.125);
    pass.resize(1920, 1080);
    expect(pass.pixelScale.x).toBe(0.125);
    expect(pass.pixelScale.y).toBeCloseTo(135 / 1080);
    pass.resize(1919, 1079);
    expect(pass.pixelScale.x).toBeCloseTo(240 / 1919);
    expect(pass.pixelScale.y).toBeCloseTo(135 / 1079);
    expect(() => pass.resize(0, 1080)).toThrow('positive');
    pass.dispose();
  });

  test('records center proximity and pixel coverage while deduplicating', async () => {
    const pass = new VirtualTextureFeedbackPass(1);
    pass.resize(3, 1);
    const layout = createPackedPageTableLayout(4, 4);
    const entry = {
      textureId: 3, path: 'page', textureMaxMip: 2, maxMip: 2,
      tailFirstMip: null, pageTableLayout: layout,
    };
    const encoded = encodeFeedback(3, 0, 1, 2);
    const words = new Uint32Array([...encoded, ...encoded, ...encoded]);
    const renderer = {
      getRenderTarget: () => null,
      setRenderTarget() {}, render() {},
      async readRenderTargetPixelsAsync() { return words; },
    };
    const store = { getEntryById: (id: number) => id === 3 ? entry : undefined };
    pass.submit(renderer as never, {} as never, {} as never, store as never);
    await flush();
    const request = pass.consume()!.values().next().value;
    expect(request).toMatchObject({ screenPriority: 0, coverage: 3 });
    pass.dispose();
  });

  test('reuses two maps and pooled request records across readbacks', async () => {
    const pass = new VirtualTextureFeedbackPass(1);
    pass.resize(2, 1);
    const layout = createPackedPageTableLayout(4, 4);
    const entry = {
      textureId: 3, path: 'page', textureMaxMip: 2, maxMip: 2,
      tailFirstMip: null, pageTableLayout: layout,
    };
    let words = new Uint32Array([...encodeFeedback(3, 0, 1, 2), 0, 0]);
    const renderer = {
      getRenderTarget: () => null,
      setRenderTarget() {}, render() {},
      async readRenderTargetPixelsAsync() { return words; },
    };
    const store = { getEntryById: (id: number) => id === 3 ? entry : undefined };

    expect(pass.submit(renderer as never, {} as never, {} as never, store as never)).toBe(true);
    await flush();
    expect(pass.submit(renderer as never, {} as never, {} as never, store as never)).toBe(false);
    const first = pass.consume()!;
    const firstRequest = first.values().next().value;
    expect(firstRequest).toMatchObject({ path: 'page', mip: 0, x: 1, y: 2 });

    words = new Uint32Array([...encodeFeedback(3, 1, 0, 1), 0, 0]);
    pass.submit(renderer as never, {} as never, {} as never, store as never);
    await flush();
    const second = pass.consume()!;
    expect(second).not.toBe(first);

    words = new Uint32Array([...encodeFeedback(3, 0, 2, 3), 0, 0]);
    pass.submit(renderer as never, {} as never, {} as never, store as never);
    await flush();
    const third = pass.consume()!;
    expect(third).toBe(first);
    expect(third.values().next().value).toBe(firstRequest);
    expect(firstRequest).toMatchObject({ path: 'page', mip: 0, x: 2, y: 3 });
    pass.dispose();
  });
});
