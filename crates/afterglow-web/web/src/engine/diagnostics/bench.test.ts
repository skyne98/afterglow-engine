import { describe, expect, test } from 'bun:test';
import { BenchStartStatus, FrameBench } from './bench.ts';

describe('FrameBench fixed capture', () => {
  test('rejects invalid capacities and sample counts without growing', () => {
    expect(() => new FrameBench({ capacity: 9 })).toThrow(RangeError);
    const bench = new FrameBench({ capacity: 12 });
    expect(bench.start(13)).toBe(BenchStartStatus.InvalidSampleCount);
    expect(bench.start(9)).toBe(BenchStartStatus.InvalidSampleCount);
    expect(bench.isRunning).toBe(false);
  });

  test('captures into fixed storage and completes only on an explicit diagnostic finish', () => {
    let callbacks = 0;
    const bench = new FrameBench({ capacity: 10, thresholdFps: 55, onDone() { callbacks++; } });
    expect(bench.start()).toBe(BenchStartStatus.Started);
    for (let frame = 0; frame <= 10; frame++) bench.tick(frame * 16);
    expect(bench.isRunning).toBe(false);
    expect(bench.hasPendingResults).toBe(true);
    expect(callbacks).toBe(0);
    const result = bench.finish();
    expect(result).not.toBeNull();
    expect(callbacks).toBe(1);
    expect(result?.n).toBe(10);
    expect(result?.p50Ms).toBe(16);
    expect(result?.p99Ms).toBe(16);
    expect(result?.belowThreshold).toBe(0);
    expect(bench.finish()).toBeNull();
  });

  test('reuses one result object across runs and reports threshold misses', () => {
    const bench = new FrameBench({ capacity: 10, thresholdFps: 55 });
    bench.start();
    for (let frame = 0; frame <= 10; frame++) bench.tick(frame * 20);
    const first = bench.finish();
    expect(first?.belowThreshold).toBe(10);
    bench.start();
    for (let frame = 0; frame <= 10; frame++) bench.tick(frame * 10);
    const second = bench.finish();
    expect(Object.is(first, second)).toBe(true);
    expect(second?.belowThreshold).toBe(0);
  });
});
