import { describe, expect, test } from 'bun:test';
import {
  EngineAssets,
  type OwnedMeshOptimizer,
  type OwnedTextureTranscoder,
} from './engine-assets.ts';
import type { FetchRangeLoader } from './asset-range.ts';

function minimalContainer(dataOffset = 19): Uint8Array {
  const bytes = new Uint8Array(Math.max(19, dataOffset));
  bytes.set([0x42, 0x49, 0x47, 0x31, 5, 0, 0, 0]);
  new DataView(bytes.buffer).setBigUint64(8, BigInt(dataOffset), true);
  if (dataOffset === 19) bytes.set([5, 19, 0], 16);
  return bytes;
}

function source(bytes: Uint8Array): FetchRangeLoader {
  return {
    async load(): Promise<Uint8Array> { return bytes.slice(); },
    async size(): Promise<number> { return bytes.byteLength; },
    async identity() { return { size: bytes.byteLength, etag: null, lastModified: null }; },
    async read(_path: string, offset: number, length: number): Promise<Uint8Array> {
      return bytes.slice(offset, offset + length);
    },
  };
}

function worker(index: number, events: string[], closeFailure = false): OwnedTextureTranscoder {
  return {
    async transcode(data) { return data; },
    close() {
      events.push(`close-${index}`);
      if (closeFailure) throw new Error(`close failure ${index}`);
    },
  };
}

function optimizer(events: string[]): OwnedMeshOptimizer {
  return {
    async optimizeVertexCache(indices) { return indices; },
    async optimizeOverdraw(indices) { return indices; },
    async simplifyWithUvs(indices) { return indices; },
    async analyzeVertexCache() { return new Float32Array(4); },
    async encodeIndexBuffer(indices) { return new Uint8Array(indices.buffer.slice(0)); },
    poll() {},
    close() { events.push('close-mesh'); },
  };
}

describe('EngineAssets', () => {
  test('owns parsed header, fixed workers, raw loader, model system, and reverse shutdown', async () => {
    const events: string[] = [];
    const engineAssets = await EngineAssets.open({
      containerPath: 'scene.big',
      format: 4,
      workerCount: 2,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64,
      maxPendingPages: 16,
      maxPendingBytes: 2 * 1024 * 1024,
      maxHeaderBytes: 1024,
      source: source(minimalContainer()),
      async createTranscoder(index) { events.push(`open-${index}`); return worker(index, events); },
      async createMeshOptimizer() { events.push('open-mesh'); return optimizer(events); },
    });
    expect(engineAssets.container.header.version).toBe(5);
    expect(engineAssets.stats.servicesStarted).toBe(2);
    const assets = await engineAssets.createAssetStore(3, 2);
    expect(assets.assetLoader).toBe(engineAssets.container.rawAssets);
    await expect(engineAssets.createAssetStore()).rejects.toThrow('already created');
    const models = await engineAssets.createModelSystem({
      maxModels: 2, maxPendingOptimizations: 1, maxResidentCpuBytes: 1024,
      completionsPerPoll: 1, ratios: [1, 0.5], targetError: 0.02,
      geometryArena: { buckets: [{
        slots: 2, maxVertices: 4, maxIndices: 6, maxGroups: 1, indexKind: 'u16',
        attributes: [{ name: 'position', itemSize: 3, kind: 'f32' }], morphAttributes: [],
      }] },
    });
    expect(models.activeModels).toBe(0);
    await expect(engineAssets.createModelSystem({
      maxModels: 1, maxPendingOptimizations: 1, maxResidentCpuBytes: 1,
      completionsPerPoll: 1, ratios: [1], targetError: 0.02,
      geometryArena: { buckets: [{
        slots: 1, maxVertices: 3, maxIndices: 3, maxGroups: 1, indexKind: 'u16',
        attributes: [{ name: 'position', itemSize: 3, kind: 'f32' }], morphAttributes: [],
      }] },
    })).rejects.toThrow('already created');
    expect(engineAssets.stats.servicesStarted).toBe(3);
    await engineAssets.close();
    await engineAssets.close();
    expect(events).toEqual(['open-0', 'open-1', 'open-mesh', 'close-mesh', 'close-1', 'close-0']);
    expect(engineAssets.stats.closed).toBe(true);
  });

  test('uses the bounded native worker manifest when no override is provided', async () => {
    const events: string[] = [];
    (globalThis as typeof globalThis & { Deno?: unknown }).Deno = { core: { ops: {
      op_afterglow_worker_ids: (service: string) => service === 'texture' ? [7, 8, 9] : [],
      async op_afterglow_rpc_call_async() { return new Uint8Array(); },
    } } };
    try {
      const assets = await EngineAssets.open({
        containerPath: 'scene.big', format: 4,
        transcodeQueueCapacity: 2, urgentBatchDeadlineMs: 1,
        focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64,
        maxPendingPages: 2, maxPendingBytes: 2 * 1024 * 1024,
        maxHeaderBytes: 1024, source: source(minimalContainer()),
        async createTranscoder(index) {
          events.push(`open-${index}`);
          return worker(index, events);
        },
      });
      expect(assets.stats.servicesStarted).toBe(2);
      await assets.close();
      expect(events).toEqual(['open-0', 'open-1', 'close-1', 'close-0']);
    } finally {
      delete (globalThis as typeof globalThis & { Deno?: unknown }).Deno;
    }
  });

  test('rejects an oversized header before spawning workers', async () => {
    let workers = 0;
    await expect(EngineAssets.open({
      containerPath: 'bad.big', format: 4, workerCount: 1,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 64,
      source: source(minimalContainer(128)),
      async createTranscoder() { workers++; return worker(0, []); },
    })).rejects.toThrow('exceeds configured capacity');
    expect(workers).toBe(0);
  });

  test('rolls back already-started workers after a later startup failure', async () => {
    const events: string[] = [];
    await expect(EngineAssets.open({
      containerPath: 'scene.big', format: 4, workerCount: 3,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: source(minimalContainer()),
      async createTranscoder(index) {
        events.push(`open-${index}`);
        if (index === 1) throw new Error('worker startup failed');
        return worker(index, events);
      },
    })).rejects.toThrow('worker startup failed');
    expect(events).toEqual(['open-0', 'open-1', 'close-0']);
  });

  test('continues closing workers and reports the first close failure', async () => {
    const events: string[] = [];
    const engineAssets = await EngineAssets.open({
      containerPath: 'scene.big', format: 4, workerCount: 2,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: source(minimalContainer()),
      async createTranscoder(index) { return worker(index, events, index === 1); },
    });
    await expect(engineAssets.close()).rejects.toThrow('close failure 1');
    expect(events).toEqual(['close-1', 'close-0']);
    expect(engineAssets.stats.closeErrors).toBe(1);
    expect(engineAssets.stats.closed).toBe(true);
  });

  test('validates mandatory capacities before I/O', async () => {
    let reads = 0;
    const badSource = source(minimalContainer());
    const originalRead = badSource.read;
    badSource.read = async (...args) => { reads++; return originalRead(...args); };
    await expect(EngineAssets.open({
      containerPath: 'scene.big', format: 4, workerCount: 0,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: badSource, async createTranscoder() { return worker(0, []); },
    })).rejects.toThrow('workerCount');
    await expect(EngineAssets.open({
      containerPath: 'scene.big', format: 4, workerCount: 1,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64, maxPendingPages: 0,
      maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: badSource, async createTranscoder() { return worker(0, []); },
    })).rejects.toThrow('pending capacities');
    await expect(EngineAssets.open({
      containerPath: 'scene.big', format: 4, workerCount: 1,
      transcodeQueueCapacity: 16, urgentBatchDeadlineMs: 17, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64,
      maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: badSource, async createTranscoder() { return worker(0, []); },
    })).rejects.toThrow('deadlines');
    await expect(EngineAssets.open({
      containerPath: 'scene.big', format: 4, workerCount: 1,
      transcodeQueueCapacity: 1, urgentBatchDeadlineMs: 1, focusBatchDeadlineMs: 16, peripheralBatchDeadlineMs: 64,
      maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: badSource, async createTranscoder() { return worker(0, []); },
    })).rejects.toThrow('cover every admitted VT page');
    expect(reads).toBe(0);
  });
});
