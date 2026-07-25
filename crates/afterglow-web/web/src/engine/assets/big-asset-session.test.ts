import { describe, expect, test } from 'bun:test';
import {
  BigAssetSession,
  type OwnedMeshOptimizer,
  type OwnedTextureTranscoder,
} from './big-asset-session.ts';
import type { FetchRangeLoader } from './big-parser.ts';

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

describe('BigAssetSession', () => {
  test('owns parsed header, fixed workers, raw loader, one VT store, and reverse shutdown', async () => {
    const events: string[] = [];
    const session = await BigAssetSession.open({
      containerPath: 'scene.big',
      format: 4,
      workerCount: 2,
      transcodeQueueCapacity: 8,
      maxPendingPages: 16,
      maxPendingBytes: 2 * 1024 * 1024,
      maxHeaderBytes: 1024,
      source: source(minimalContainer()),
      async createTranscoder(index) { events.push(`open-${index}`); return worker(index, events); },
      async createMeshOptimizer() { events.push('open-mesh'); return optimizer(events); },
    });
    expect(session.header.version).toBe(5);
    expect(session.stats.workersStarted).toBe(2);
    const assets = await session.createAssetStore(3, 2);
    expect(assets.assetLoader).toBe(session.rawAssets);
    await expect(session.createAssetStore()).rejects.toThrow('already created');
    const store = session.createVirtualTextureStore();
    expect(store).toBeDefined();
    expect(() => session.createVirtualTextureStore()).toThrow('already created');
    await session.close();
    await session.close();
    expect(events).toEqual(['open-0', 'open-1', 'open-mesh', 'close-mesh', 'close-1', 'close-0']);
    expect(session.stats.closed).toBe(true);
    expect(() => session.createVirtualTextureStore()).toThrow('closed');
  });

  test('rejects an oversized header before spawning workers', async () => {
    let workers = 0;
    await expect(BigAssetSession.open({
      containerPath: 'bad.big', format: 4, workerCount: 1,
      transcodeQueueCapacity: 1, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 64,
      source: source(minimalContainer(128)),
      async createTranscoder() { workers++; return worker(0, []); },
    })).rejects.toThrow('exceeds configured capacity');
    expect(workers).toBe(0);
  });

  test('rolls back already-started workers after a later startup failure', async () => {
    const events: string[] = [];
    await expect(BigAssetSession.open({
      containerPath: 'scene.big', format: 4, workerCount: 3,
      transcodeQueueCapacity: 2, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
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
    const session = await BigAssetSession.open({
      containerPath: 'scene.big', format: 4, workerCount: 2,
      transcodeQueueCapacity: 2, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: source(minimalContainer()),
      async createTranscoder(index) { return worker(index, events, index === 1); },
    });
    await expect(session.close()).rejects.toThrow('close failure 1');
    expect(events).toEqual(['close-1', 'close-0']);
    expect(session.stats.closeErrors).toBe(1);
    expect(session.stats.closed).toBe(true);
  });

  test('validates mandatory capacities before I/O', async () => {
    let reads = 0;
    const badSource = source(minimalContainer());
    const originalRead = badSource.read;
    badSource.read = async (...args) => { reads++; return originalRead(...args); };
    await expect(BigAssetSession.open({
      containerPath: 'scene.big', format: 4, workerCount: 0,
      transcodeQueueCapacity: 1, maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: badSource, async createTranscoder() { return worker(0, []); },
    })).rejects.toThrow('workerCount');
    await expect(BigAssetSession.open({
      containerPath: 'scene.big', format: 4, workerCount: 1,
      transcodeQueueCapacity: 1, maxPendingPages: 0,
      maxPendingBytes: 2 * 1024 * 1024, maxHeaderBytes: 1024,
      source: badSource, async createTranscoder() { return worker(0, []); },
    })).rejects.toThrow('pending capacities');
    expect(reads).toBe(0);
  });
});
