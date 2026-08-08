import { describe, expect, test } from 'bun:test';
import {
  ENGINE_METRIC_DESCRIPTORS,
  ENGINE_TRACE_DESCRIPTORS,
  EngineMetric,
  EngineTraceDescriptor,
} from './catalog.ts';

describe('engine telemetry catalog ABI', () => {
  test('appends model, mutable-texture, geometry, and storage descriptors', () => {
    expect(ENGINE_TRACE_DESCRIPTORS[EngineTraceDescriptor.VtPagePublished]?.name)
      .toBe('vt.page_published');
    expect(ENGINE_TRACE_DESCRIPTORS[EngineTraceDescriptor.MutableTextureWrite]?.name)
      .toBe('vt.mutable_write');
    expect(ENGINE_TRACE_DESCRIPTORS[EngineTraceDescriptor.ModelPublished]?.name)
      .toBe('model.published');
    expect(ENGINE_TRACE_DESCRIPTORS[EngineTraceDescriptor.BlobWrite]?.name)
      .toBe('blob.write');
    expect(ENGINE_METRIC_DESCRIPTORS[EngineMetric.MutableTextureWrites]?.name)
      .toBe('mutable_texture_writes');
    expect(ENGINE_METRIC_DESCRIPTORS[EngineMetric.ModelGpuBytesHighWater]?.name)
      .toBe('model_gpu_bytes_high_water');
    expect(ENGINE_METRIC_DESCRIPTORS[EngineMetric.BlobWriteBytes]?.name)
      .toBe('blob_write_bytes');
  });
});
