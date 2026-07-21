import { describe, expect, test } from 'bun:test';
import { GpuProfiler } from './gpu-profiler.ts';

// WebGPU globals are absent in bun; stub the usage flags the profiler uses.
(globalThis as unknown as { GPUBufferUsage?: Record<string, number> }).GPUBufferUsage ??= {
  MAP_READ: 1, COPY_SRC: 4, COPY_DST: 8, QUERY_RESOLVE: 512,
};

// Minimal WebGPU mocks sufficient for the profiler's API surface.
function mockDevice(supported: boolean): GPUDevice {
  let bufferId = 0;
  let querySetId = 0;
  const features = new Set<string>(supported ? ['timestamp-query'] : []);
  return {
    features,
    createQuerySet: (desc: { type: string; count: number }) => ({
      type: desc.type, count: desc.count, id: querySetId++, destroy() {},
    }),
    createBuffer: (desc: { size: number; usage: number }) => ({
      size: desc.size, usage: desc.usage, id: bufferId++, mapped: false,
      destroy() {},
      mapAsync: async () => {},
      getMappedRange: () => new ArrayBuffer(desc.size),
      unmap() {},
    }),
  } as unknown as GPUDevice;
}

function mockQueue(period = 1) {
  return { getTimestampPeriod: () => period } as unknown as GPUQueue;
}

function mockEncoder() {
  const calls: string[] = [];
  return {
    encoder: {
      resolveQuerySet: () => calls.push('resolve'),
      copyBufferToBuffer: () => calls.push('copy'),
      writeTimestamp: () => calls.push('write'),
    } as unknown as GPUCommandEncoder,
    calls,
  };
}

describe('GpuProfiler', () => {
  test('is a no-op when timestamp-query is unsupported', () => {
    const profiler = new GpuProfiler(mockDevice(false), mockQueue());
    expect(profiler.isSupported()).toBe(false);
    const frame = profiler.beginFrame();
    const desc = {} as GPURenderPassDescriptor;
    expect(frame.withPass('p', desc)).toBe(desc);
    expect(desc.timestampWrites).toBeUndefined();
    const enc = mockEncoder().encoder;
    expect(frame.scope('z', enc)).toBeDefined();
    profiler.endFrame(enc);
    // poll resolves nothing.
    expect(profiler.poll()).resolves.toEqual([]);
    profiler.dispose();
  });

  test('attaches timestampWrites and resolves when supported', async () => {
    const profiler = new GpuProfiler(mockDevice(true), mockQueue(1));
    expect(profiler.isSupported()).toBe(true);
    const frame = profiler.beginFrame();
    const desc = {} as GPURenderPassDescriptor;
    frame.withPass('main', desc);
    expect(desc.timestampWrites).toBeDefined();
    expect(desc.timestampWrites).toMatchObject({ beginningOfPassWriteIndex: 0, endOfPassWriteIndex: 1 });
    const { encoder, calls } = mockEncoder();
    profiler.endFrame(encoder);
    expect(calls).toContain('resolve');
    expect(calls).toContain('copy');
    profiler.dispose();
  });

  test('rejects invalid capacity options', () => {
    expect(() => new GpuProfiler(mockDevice(true), mockQueue(), { framesInFlight: 0 })).toThrow();
    expect(() => new GpuProfiler(mockDevice(true), mockQueue(), { maxScopesPerFrame: 0 })).toThrow();
  });

  test('exportChromeTrace emits a complete-event per scope', () => {
    const profiler = new GpuProfiler(mockDevice(true), mockQueue());
    const trace = profiler.exportChromeTrace([
      { name: 'main', startNs: 1000n, endNs: 2_000_000n, durationMs: 2.0 },
    ]);
    const parsed = JSON.parse(trace);
    expect(parsed.traceEvents).toHaveLength(1);
    expect(parsed.traceEvents[0]).toMatchObject({ name: 'main', ph: 'X', cat: 'gpu' });
    expect(parsed.traceEvents[0].dur).toBe(2000);
    profiler.dispose();
  });
});
