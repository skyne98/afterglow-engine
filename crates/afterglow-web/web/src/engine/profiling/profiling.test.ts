import { describe, expect, test } from 'bun:test';
import { Profiling } from './profiling.ts';
import type { ProfilingFrame, ProfilingRenderer } from './profiling.ts';

function mockRenderer(opts: { renderCalls?: number; triangles?: number; resolve?: (t: string) => number }): ProfilingRenderer & { resolveTimestampsAsync: (t: string) => Promise<number> } {
  const info = {
    render: { calls: opts.renderCalls ?? 3, drawCalls: opts.renderCalls ?? 3, triangles: opts.triangles ?? 1000, points: 0, lines: 0 },
    compute: { calls: 1 },
    memory: { textures: 5, texturesSize: 1024, uniformBuffers: 8, uniformBuffersSize: 2048, geometries: 2 },
    reset() {},
  };
  return {
    info,
    resolveTimestampsAsync: async (t: string) => opts.resolve?.(t) ?? 0,
    // Three-private backend surface toggled by setEnabled:
    backend: { trackTimestamp: false, timestampQueryPool: {} },
  } as unknown as ProfilingRenderer & { resolveTimestampsAsync: (t: string) => Promise<number> };
}

describe('Profiling', () => {
  test('rejects invalid capacity', () => {
    expect(() => new Profiling({ renderer: mockRenderer({}) }, { capacity: 0 })).toThrow();
  });

  test('gather snapshots renderer.info counts and GPU timings when enabled', async () => {
    const renderer = mockRenderer({ renderCalls: 4, triangles: 2000, resolve: (t) => (t === 'render' ? 1.5 : 0.3) });
    const p = new Profiling({ renderer, deltaSource: () => 16.7 }, { capacity: 8 });
    p.setEnabled(true);
    const f = await p.gather(42);
    expect(f.frameId).toBe(42);
    expect(f.deltaMs).toBe(16.7);
    expect(f.drawCalls).toBe(4);
    expect(f.triangles).toBe(2000);
    expect(f.computeCalls).toBe(1);
    expect(f.textures).toBe(5);
    expect(f.gpuRenderMs).toBe(1.5);
    expect(f.gpuComputeMs).toBe(0.3);
    expect(p.sampleCount()).toBe(1);
  });

  test('GPU timings read 0 when disabled', async () => {
    const renderer = mockRenderer({ resolve: () => 9.9 });
    const p = new Profiling({ renderer });
    p.setEnabled(false);
    const f = await p.gather(0);
    expect(f.gpuRenderMs).toBe(0);
    expect(f.gpuComputeMs).toBe(0);
  });

  test('latest returns oldest-first and respects ring capacity', async () => {
    const renderer = mockRenderer({});
    const p = new Profiling({ renderer, deltaSource: () => 16 }, { capacity: 3 });
    for (let i = 0; i < 5; i++) await p.gather(i);
    expect(p.sampleCount()).toBe(3); // capacity capped
    const out: ProfilingFrame[] = [];
    const n = p.latest(2, out);
    expect(n).toBe(2);
    expect(out[0]?.frameId).toBe(3); // most-recent 2 of frames 2,3,4 -> 3,4 oldest-first
    expect(out[1]?.frameId).toBe(4);
  });

  test('exportChromeTrace emits a gpu.render complete-event per timed frame', async () => {
    const renderer = mockRenderer({ resolve: (t) => (t === 'render' ? 2.0 : 0) });
    const p = new Profiling({ renderer, deltaSource: () => 16.7 });
    p.setEnabled(true);
    await p.gather(0);
    const trace = JSON.parse(p.exportChromeTrace());
    expect(trace.traceEvents.length).toBe(1);
    expect(trace.traceEvents[0]).toMatchObject({ name: 'gpu.render', cat: 'gpu', ph: 'X', dur: 2000 });
  });
});
