import { beforeAll, describe, expect, mock, test } from 'bun:test';
import * as RealThree from 'three/webgpu';
import { packedPageTableIndex } from './virtual-texture-layout.ts';
import {
  perceptualImportanceBucket,
  type PageRequest,
  type VirtualPageRequest,
} from './virtual-texture-request.ts';
import { FORMAT_RGBA, SLOT_SIZE } from './virtual-texture-format.ts';
import { VT_SAMPLE_FROM_LEVEL_WGSL } from './virtual-texture-shaders.ts';
import {
  DEFAULT_VIRTUAL_TEXTURE_TUNING,
  VirtualTextureTuning,
} from './virtual-texture-tuning.ts';
import {
  EngineMetric, EngineTraceDescriptor, ENGINE_METRIC_DESCRIPTORS, ENGINE_TRACE_DESCRIPTORS,
} from '../telemetry/catalog.ts';
import { EngineTelemetry, TELEMETRY_RECORD_BYTES } from '../telemetry/telemetry.ts';

class Texture { needsUpdate = false; dispose() {} }
class DataTexture extends Texture {
  minFilter: unknown; magFilter: unknown; generateMipmaps = false;
  constructor(public data: Uint8Array | Uint32Array, public width: number, public height: number) { super(); }
}
class CompressedTexture extends Texture {}
class RenderTarget {
  texture = { name: '' };
  constructor(public width: number, public height: number, _options: unknown) {}
  setSize(width: number, height: number) { this.width = width; this.height = height; }
  dispose() {}
}
const threeMock = {
  ...RealThree,
  Texture, DataTexture, CompressedTexture, RenderTarget,
  RGBAFormat: 1, RedIntegerFormat: 2, UnsignedIntType: 3, RGIntegerFormat: 6,
  RGBA_BPTC_Format: 36492, RGBA_ASTC_4x4_Format: 37808,
  LinearFilter: 4, NearestFilter: 5,
};
mock.module('three', () => threeMock);
mock.module('three/webgpu', () => threeMock);

type VTModule = typeof import('./virtual-texture.ts');
let VT: VTModule;
beforeAll(async () => { VT = await import('./virtual-texture.ts'); });

const PAGE_BYTES = 136 * 136 * 4;
const TEST_CAPACITIES = { maxPendingPages: 64, maxPendingBytes: 8 * 1024 * 1024 } as const;
const NO_CACHE_CAPACITIES = { maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024 } as const;
const flush = () => new Promise(resolve => setTimeout(resolve, 0));
const settle = async (store: { poll(): void }) => {
  await flush(); await flush();
  store.poll();
  await flush(); await flush();
  for (let i = 1; i < 32; i++) store.poll();
};

describe('VirtualTextureStore residency identity', () => {
  test('disposes loaded texture tables and atlas idempotently', () => {
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES);
    store.loadTexture('dispose', { width: 128, height: 128 });
    expect(store.getEntry('dispose')).toBeDefined();
    store.dispose();
    store.dispose();
    expect(store.getEntry('dispose')).toBeUndefined();
  });

  test('requires explicit positive pending-page and byte capacities', () => {
    expect(() => new VT.VirtualTextureStore({
      maxPendingPages: 0, maxPendingBytes: PAGE_BYTES,
    })).toThrow('runtime capacities');
    expect(() => new VT.VirtualTextureStore({
      maxPendingPages: 1, maxPendingBytes: PAGE_BYTES - 1,
    })).toThrow('runtime capacities');
  });

  test('displaced samples walk coarser pages with stable base gradients', () => {
    expect(VT_SAMPLE_FROM_LEVEL_WGSL).toContain('mip <= maxLevel');
    expect(VT_SAMPLE_FROM_LEVEL_WGSL).toContain('dpdx(gradientUV) * gradientScale');
    expect(VT_SAMPLE_FROM_LEVEL_WGSL).toContain('tailEntry');
  });

  test('quantizes every bounded perceptual weight into 25 monotonic buckets', () => {
    const seen = new Set<number>();
    let previous = perceptualImportanceBucket(1);
    seen.add(previous);
    for (let weight = 2; weight <= 5610; weight++) {
      const bucket = perceptualImportanceBucket(weight);
      expect(bucket).toBeLessThanOrEqual(previous);
      seen.add(bucket);
      previous = bucket;
    }
    expect(perceptualImportanceBucket(1)).toBe(24);
    expect(perceptualImportanceBucket(5610)).toBe(0);
    expect(seen.size).toBe(25);
  });

  test('atlas-only tuning override preserves upload defaults and drains a constrained store', async () => {
    const tuning = new VirtualTextureTuning({ atlasMaxDimension: SLOT_SIZE * 4 });
    expect(tuning.atlasMaxDimension).toBe(SLOT_SIZE * 4);
    expect(tuning.uploadsPerPoll).toBe(DEFAULT_VIRTUAL_TEXTURE_TUNING.baselineUploadsPerPoll);
    expect(tuning.uploadBudgetMs).toBe(DEFAULT_VIRTUAL_TEXTURE_TUNING.baselineUploadBudgetMs);
    expect(tuning.bestSafeUploadsPerPoll).toBe(DEFAULT_VIRTUAL_TEXTURE_TUNING.baselineUploadsPerPoll);
    expect(tuning.bestSafeUploadBudgetMs).toBe(DEFAULT_VIRTUAL_TEXTURE_TUNING.baselineUploadBudgetMs);

    const store = new VT.VirtualTextureStore(
      TEST_CAPACITIES,
      async () => new Uint8Array(PAGE_BYTES),
      FORMAT_RGBA,
      undefined,
      tuning,
    );
    expect(store.getStats().atlasSlotsTotal).toBe(16);
    store.loadTexture('constrained', { width: 512, height: 512 });
    await settle(store);
    expect(store.getStats().completedUploads).toBeGreaterThan(0);
    expect(store.getStats().atlasSlotsUsed).toBeGreaterThan(0);
  });

  test('correlates feedback through scheduler, load, upload, and publication', async () => {
    let tick = 1;
    const telemetry = new EngineTelemetry(
      ENGINE_TRACE_DESCRIPTORS,
      ENGINE_METRIC_DESCRIPTORS,
      new ArrayBuffer(TELEMETRY_RECORD_BYTES * 256),
      new Float64Array(256),
      () => tick++,
    );
    const store = new VT.VirtualTextureStore(
      TEST_CAPACITIES,
      async () => new Uint8Array(PAGE_BYTES),
      FORMAT_RGBA,
      undefined,
      new VirtualTextureTuning({ atlasMaxDimension: SLOT_SIZE * 4 }),
      telemetry,
    );
    store.loadTexture('traced', { width: 512, height: 512 });
    await settle(store);
    telemetry.trace.arm(9);
    store.setPublicationFrameId(42);
    store.processFeedback(new Map([['visible', {
      path: 'traced', mip: 0, x: 0, y: 0, screenPriority: 0, coverage: 8,
    }]]));
    await settle(store);
    telemetry.trace.stop();
    const snapshot = telemetry.trace.snapshot();
    if (snapshot === null) throw new Error('missing telemetry snapshot');
    const words = new Uint32Array(snapshot.buffer);
    const descriptors: number[] = [];
    let schedulerBegins = 0;
    let schedulerEnds = 0;
    let publishedFrame = -1;
    for (let index = 0; index < snapshot.count; index++) {
      const descriptor = words[index * 10 + 8] ?? -1;
      const phase = (words[index * 10 + 9] ?? 0) & 0xff;
      descriptors.push(descriptor);
      if (descriptor === EngineTraceDescriptor.VtSchedulerWait && phase === 4) schedulerBegins++;
      if (descriptor === EngineTraceDescriptor.VtSchedulerWait && phase === 5) schedulerEnds++;
      if (descriptor === EngineTraceDescriptor.VtPagePublished) publishedFrame = words[index * 10 + 6] ?? -1;
    }
    expect(descriptors).toContain(EngineTraceDescriptor.VtFeedbackDetected);
    expect(descriptors).toContain(EngineTraceDescriptor.VtSchedulerWait);
    expect(descriptors).toContain(EngineTraceDescriptor.VtPageLoad);
    expect(descriptors).toContain(EngineTraceDescriptor.VtUpload);
    expect(descriptors).toContain(EngineTraceDescriptor.VtPagePublished);
    expect(schedulerBegins).toBe(schedulerEnds);
    expect(schedulerBegins).toBeGreaterThan(0);
    expect(publishedFrame).toBe(42);
    expect(telemetry.metrics.readCell(EngineMetric.VtPagesRequested)).toBeGreaterThan(0);
    expect(telemetry.metrics.readCell(EngineMetric.VtPagesLoaded)).toBeGreaterThan(0);
    store.dispose();
  });

  test('central tuning probes upward only after stability and rolls back a bad probe', () => {
    const tuning = new VirtualTextureTuning({
      minUploadsPerPoll: 1,
      baselineUploadsPerPoll: 2,
      maxUploadsPerPoll: 3,
      minUploadBudgetMs: 0.10,
      baselineUploadBudgetMs: 0.20,
      maxUploadBudgetMs: 0.30,
      uploadBudgetStepMs: 0.05,
      targetFrameMs: 10,
      overloadMultiplier: 1.25,
      overloadSamples: 2,
      sampleWindow: 3,
      stableWindowsBeforeProbe: 1,
      probeCooldownWindows: 1,
    });
    const stable = () => { tuning.recordFrameTime(10, 1); tuning.recordFrameTime(10, 1); tuning.recordFrameTime(10, 1); };
    const overloaded = () => { tuning.recordFrameTime(20, 1); tuning.recordFrameTime(20, 1); tuning.recordFrameTime(10, 1); };

    // Sustained overload tightens the baseline and records that new fallback.
    overloaded();
    expect(tuning.uploadsPerPoll).toBe(1);
    expect(tuning.bestSafeUploadsPerPoll).toBe(1);
    // Cooldown consumes one stable window; the next one probes 1 -> 2.
    stable(); stable();
    expect(tuning.uploadsPerPoll).toBe(2);
    expect(tuning.probes).toBe(1);
    // A clean probe is promoted to the known-safe setting.
    stable();
    expect(tuning.bestSafeUploadsPerPoll).toBe(2);
    expect(tuning.recoveries).toBe(1);
    // A later stable window probes 2 -> 3 and promotes it. A workload change
    // then rolls the whole promoted ladder back to the validated 2-page cap.
    stable();
    expect(tuning.uploadsPerPoll).toBe(3);
    stable();
    expect(tuning.bestSafeUploadsPerPoll).toBe(3);
    overloaded();
    expect(tuning.uploadsPerPoll).toBe(2);
    expect(tuning.bestSafeUploadsPerPoll).toBe(2);
    expect(tuning.probeRejections).toBe(1);
  });

  test('central tuning preserves clean evidence across transiently empty backlogs', () => {
    const tuning = new VirtualTextureTuning({
      minUploadsPerPoll: 1, baselineUploadsPerPoll: 2, maxUploadsPerPoll: 3,
      minUploadBudgetMs: 0.10, baselineUploadBudgetMs: 0.20, maxUploadBudgetMs: 0.30,
      uploadBudgetStepMs: 0.05, targetFrameMs: 10, overloadMultiplier: 1.25,
      overloadSamples: 2, sampleWindow: 3, stableWindowsBeforeProbe: 1,
      probeCooldownWindows: 1,
    });
    tuning.recordFrameTime(10, 1);
    tuning.recordFrameTime(10, 1);
    for (let idle = 0; idle < 60; idle++) tuning.recordFrameTime(10, 0);
    tuning.recordFrameTime(10, 1);
    expect(tuning.uploadsPerPoll).toBe(3);
    expect(tuning.probes).toBe(1);
  });

  test('deduplicates in-flight requests but keeps texture paths distinct', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (path, req) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      await flush();
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('a', { width: 512, height: 512 });
    store.loadTexture('b', { width: 512, height: 512 });
    await settle(store);
    calls.length = 0;

    const a: VirtualPageRequest = { path: 'a', mip: 0, x: 0, y: 0 };
    const b: VirtualPageRequest = { path: 'b', mip: 0, x: 0, y: 0 };
    store.processFeedback(new Map([['a', a], ['b', b]]));
    store.processFeedback(new Map([['a duplicate', a], ['b duplicate', b]]));
    store.poll();
    expect(calls.filter(key => key === 'a:0:0:0')).toHaveLength(1);
    expect(calls.filter(key => key === 'b:0:0:0')).toHaveLength(1);

    await settle(store);
    const entryA = store.getEntry('a')!;
    const entryB = store.getEntry('b')!;
    expect(entryA.pageTable[packedPageTableIndex(entryA.pageTableLayout, 0, 0, 0)] & 1).toBe(1);
    expect(entryB.pageTable[packedPageTableIndex(entryB.pageTableLayout, 0, 0, 0)] & 1).toBe(1);
  });

  test('cancels asynchronous work before ready-time residency acquisition', async () => {
    let resolvePage!: (data: Uint8Array) => void;
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, () => new Promise(resolve => { resolvePage = resolve; }));
    store.loadTexture('temporary', { width: 128, height: 128 });
    expect(store.getStats().pendingPages).toBe(1);
    store.unloadTexture('temporary');
    expect(store.getStats().pendingPages).toBe(0);
    expect(store.getEntry('temporary')).toBeUndefined();
    resolvePage(new Uint8Array(PAGE_BYTES));
    await flush();
    expect(store.getStats().atlasSlotsUsed).toBe(0);
  });

  test('does not reserve or evict a physical slot before page data is ready', async () => {
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) =>
      req.mip === 0 ? never : new Uint8Array(PAGE_BYTES));
    store.loadTexture('ready-time', { width: 512, height: 512 });
    await settle(store);
    const before = store.getStats().atlasSlotsUsed;
    store.processFeedback(new Map([['fine', { path: 'ready-time', mip: 0, x: 0, y: 0 }]]));
    store.poll();
    expect(store.getStats().pendingPages).toBe(1);
    expect(store.getStats().atlasSlotsUsed).toBe(before);
  });

  test('loads one packed mip tail into a pinned physical slot', async () => {
    const requests: Array<{ mip: number; tail?: boolean }> = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      requests.push(req);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('tail', { width: 512, height: 512, mipTail: true });
    await settle(store);
    expect(requests.filter(request => request.tail)).toHaveLength(1);
    const entry = store.getEntry('tail')!;
    expect(entry.tailFirstMip).toBe(3);
    expect(entry.textureMaxMip).toBe(9);
    expect(entry.tailEntry & 1).toBe(1);
  });

  test('lays out startup RGBA pages as texel rows in the CPU atlas', async () => {
    const page = new Uint8Array(PAGE_BYTES);
    for (let row = 0; row < 136; row++) page.fill(row, row * 136 * 4, (row + 1) * 136 * 4);
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async () => page);
    store.loadTexture('cpu-rgba', { width: 128, height: 128 });
    await settle(store);
    const atlas = (store.atlasTexture as unknown as DataTexture).data as Uint8Array;
    const originX = 14 * 136, originY = 14 * 136, atlasWidth = 15 * 136;
    expect(atlas[(originY * atlasWidth + originX) * 4]).toBe(0);
    expect(atlas[((originY + 1) * atlasWidth + originX) * 4]).toBe(1);
    expect(atlas[((originY + 135) * atlasWidth + originX + 135) * 4]).toBe(135);
  });

  test('uses texel rows for attached RGBA subregion writes', async () => {
    const layouts: Array<{ bytesPerRow?: number; rowsPerImage?: number }> = [];
    const device = {
      limits: { maxTextureDimension2D: 8192 },
      queue: { writeTexture(_dst: unknown, _data: unknown, layout: { bytesPerRow?: number; rowsPerImage?: number }) { layouts.push(layout); } },
    } as unknown as GPUDevice;
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async () => new Uint8Array(PAGE_BYTES));
    store.loadTexture('rgba', { width: 512, height: 512 });
    await settle(store);
    store.attachRenderer({ backend: { device, get: () => ({ texture: {} as GPUTexture }) } });
    const req: VirtualPageRequest = { path: 'rgba', mip: 0, x: 0, y: 0 };
    store.processFeedback(new Map([['page', req]]));
    await settle(store);
    expect(layouts.some(layout => layout.bytesPerRow === 136 * 4 && layout.rowsPerImage === 136)).toBe(true);
  });

  test('expands perceptual albedo demand while preserving channel mip biases', async () => {
    const calls: Array<{ path: string; mip: number; tier: string | undefined }> = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (path, req) => {
      calls.push({ path, mip: req.mip, tier: req.batchTier });
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadMaterialSet(
      { albedo: 'color', normal: 'normal', roughness: 'rough', ao: 'ao' },
      { width: 4096, height: 4096 },
    );
    await settle(store); calls.length = 0;
    const request: VirtualPageRequest = {
      path: 'color', mip: 0, x: 0, y: 0,
      screenPriority: 0, coverage: 8, perceptualWeight: 120,
    };
    store.processFeedback(new Map([['visible', request]]));
    await settle(store);
    expect(new Set(calls.map(call => call.path))).toEqual(new Set(['color', 'normal', 'rough', 'ao']));
    expect(Math.min(...calls.filter(call => call.path === 'color').map(call => call.mip))).toBe(0);
    expect(Math.min(...calls.filter(call => call.path === 'normal').map(call => call.mip))).toBe(1);
    expect(Math.min(...calls.filter(call => call.path === 'rough').map(call => call.mip))).toBe(2);
    expect(Math.min(...calls.filter(call => call.path === 'ao').map(call => call.mip))).toBe(2);
    expect(calls.some(call => call.tier === 'urgent')).toBe(true);
    expect(calls.some(call => call.tier === 'focus')).toBe(true);
    expect(calls[0]).toMatchObject({ path: 'color', mip: 2, tier: 'urgent' });
    expect(calls.find(call => call.tier === 'focus')).toMatchObject({ path: 'color', mip: 0 });
  });

  test('accepts material-configurable channel mip biases', async () => {
    const calls: Array<{ path: string; mip: number }> = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (path, req) => {
      calls.push({ path, mip: req.mip });
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadMaterialSet(
      { albedo: 'color', normal: 'normal', masks: 'masks' },
      { width: 4096, height: 4096, mipBiases: { normal: 3, masks: 1 } },
    );
    await settle(store); calls.length = 0;
    store.processFeedback(new Map([['visible', { path: 'color', mip: 0, x: 0, y: 0 }]]));
    await settle(store);
    expect(Math.min(...calls.filter(call => call.path === 'color').map(call => call.mip))).toBe(0);
    expect(Math.min(...calls.filter(call => call.path === 'normal').map(call => call.mip))).toBe(3);
    expect(Math.min(...calls.filter(call => call.path === 'masks').map(call => call.mip))).toBe(1);
  });

  test('links already-loaded channels and unions shared material roles', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async path => {
      calls.push(path);
      return new Uint8Array(PAGE_BYTES);
    });
    for (const path of ['color', 'normal', 'masks', 'emissive'])
      store.loadTexture(path, { width: 4096, height: 4096 });
    await settle(store); calls.length = 0;
    store.linkMaterialSet({
      albedo: store.getEntry('color')!, normal: store.getEntry('normal')!, masks: store.getEntry('masks')!,
    });
    store.linkMaterialSet({ albedo: store.getEntry('color')!, emissive: store.getEntry('emissive')! });
    store.processFeedback(new Map([['visible', { path: 'color', mip: 0, x: 0, y: 0 }]]));
    await settle(store);
    expect(new Set(calls)).toEqual(new Set(['color', 'normal', 'masks', 'emissive']));
  });

  test('evicts with a bounded second-chance clock at full capacity', async () => {
    const device = { limits: { maxTextureDimension2D: 408 } } as GPUDevice; // 3x3 slots
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async () => new Uint8Array(PAGE_BYTES), FORMAT_RGBA, device);
    store.loadTexture('clock', { width: 512, height: 512 }); // five pinned pages
    await settle(store);
    for (let page = 0; page < 5; page++) {
      const request = { path: 'clock', mip: 0, x: page % 4, y: Math.floor(page / 4) };
      store.processFeedback(new Map([[`${page}`, request]]));
      await settle(store);
    }
    const stats = store.getStats();
    expect(stats.atlasSlotsUsed).toBe(9);
    expect(store.getDebugSnapshot().textures[0].residentPages).toBe(9);
  });

  test('evicts material channels independently under atlas pressure', async () => {
    const device = { limits: { maxTextureDimension2D: 816 } } as GPUDevice; // 6x6 slots
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async () => new Uint8Array(PAGE_BYTES), FORMAT_RGBA, device);
    const material = store.loadMaterialSet(
      { albedo: 'group-color', normal: 'group-normal', masks: 'group-masks' },
      { width: 512, height: 512 },
    );
    await settle(store);
    for (let page = 0; page < 12; page++) {
      const request = { path: 'group-color', mip: 0, x: page % 4, y: Math.floor(page / 4) };
      store.processFeedback(new Map([[page, request]]));
      await settle(store);
    }
    let mixedPages = 0;
    for (let page = 0; page < 16; page++) {
      const index = packedPageTableIndex(material.albedo.pageTableLayout, 0, page % 4, Math.floor(page / 4));
      const states = [material.albedo, material.normal!, material.masks!]
        .map(entry => entry.pageTable[index] & 1);
      if (new Set(states).size > 1) mixedPages++;
    }
    expect(mixedPages).toBeGreaterThan(0);
  });

  test('merges several feedback passes into one visibility epoch', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (path) => {
      calls.push(path);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('batch-a', { width: 512, height: 512 });
    store.loadTexture('batch-b', { width: 512, height: 512 });
    await settle(store);
    calls.length = 0;
    const maps = [
      new Map([['a', { path: 'batch-a', mip: 0, x: 0, y: 0 }]]),
      new Map([['b', { path: 'batch-b', mip: 0, x: 1, y: 0 }]]),
    ];
    const result = store.processFeedbackBatch(maps, maps.length);
    expect(result.totalRequests).toBeGreaterThan(1);
    await settle(store);
    expect(calls).toContain('batch-a');
    expect(calls).toContain('batch-b');
  });

  test('owns async page coordinates when scheduler scratch slots are reused', async () => {
    const captured: PageRequest[] = [];
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      if (req.mip !== 0) return new Uint8Array(PAGE_BYTES);
      captured.push(req);
      return never;
    });
    store.loadTexture('owned-request', { width: 512, height: 512 });
    await settle(store);
    store.processFeedback(new Map([['first', { path: 'owned-request', mip: 0, x: 0, y: 0 }]]));
    store.poll();
    store.processFeedback(new Map([['second', { path: 'owned-request', mip: 0, x: 1, y: 0 }]]));
    store.poll();
    expect(captured.map(req => [req.mip, req.x, req.y])).toEqual([[0, 0, 0], [0, 1, 0]]);
  });

  test('commits at most two completed page uploads per poll', async () => {
    const releases: Array<() => void> = [];
    const page = new Uint8Array(PAGE_BYTES);
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      // Let pinned coarse mips settle immediately, then release four fine pages
      // together to emulate a worker completion burst.
      if (req.mip !== 0) return page;
      return new Promise<Uint8Array>(resolve => releases.push(() => resolve(page)));
    });
    store.loadTexture('paced', { width: 512, height: 512 });
    await settle(store);
    const feedback = new Map<string, VirtualPageRequest>();
    for (let pageIndex = 0; pageIndex < 4; pageIndex++) {
      feedback.set(`${pageIndex}`, {
        path: 'paced', mip: 0, x: pageIndex, y: 0,
      });
    }
    store.processFeedback(feedback);
    store.poll();
    expect(releases).toHaveLength(4);
    for (const release of releases) release();
    await flush(); await flush();

    const before = store.getStats().completedUploads;
    store.poll();
    const firstPoll = store.getStats().completedUploads - before;
    expect(firstPoll).toBeGreaterThan(0);
    expect(firstPoll).toBeLessThanOrEqual(2);
    for (let poll = 0; poll < 4 && store.getStats().readyUploads !== 0; poll++) store.poll();
    expect(store.getStats().completedUploads - before).toBe(4);
    expect(store.getStats().readyUploads).toBe(0);
  });

  test('persists visible requests across polls instead of dropping the frame overflow', async () => {
    const calls: string[] = [];
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (path, req) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      return req.mip >= 4 ? new Uint8Array(PAGE_BYTES) : never;
    });
    store.loadTexture('persistent', { width: 4096, height: 4096 });
    await settle(store);
    calls.length = 0;
    const feedback = new Map<string, VirtualPageRequest>();
    for (let page = 0; page < 16; page++) {
      const request = { path: 'persistent', mip: 0, x: (page & 3) << 3, y: (page >> 2) << 3 };
      feedback.set(`${page}`, request);
    }
    const first = store.processFeedback(feedback);
    expect(first.loaded).toBe(0);
    expect(first.queuedRequests).toBe(32);
    for (let frame = 0; frame < 8; frame++) store.poll();
    expect(calls.filter(call => call.startsWith('persistent:2:'))).toHaveLength(16);
    expect(calls).toHaveLength(32);
    expect(store.getStats().scheduledRequests).toBe(0);
  });

  test('prioritizes perceptual quality correction before lower-value restoration', async () => {
    const calls: Array<{ mip: number; x: number; y: number }> = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      calls.push({ mip: req.mip, x: req.x, y: req.y });
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('priority', { width: 4096, height: 4096 });
    await settle(store);
    // Build one coordinate through mip 1 so its next request is an ultra mip-0
    // upgrade, while the other coordinates remain at the pinned fallback.
    store.processFeedback(new Map([['middle', {
      path: 'priority', mip: 1, x: 0, y: 0, screenPriority: 0, coverage: 8,
    }]]));
    await settle(store);
    calls.length = 0;
    store.setDebugPageBudget(1);
    store.processFeedback(new Map([
      ['ultra-center', { path: 'priority', mip: 0, x: 0, y: 0, screenPriority: 0, coverage: 8 }],
      ['coarse-edge', { path: 'priority', mip: 0, x: 24, y: 24, screenPriority: 255, coverage: 1 }],
      ['coarse-center', { path: 'priority', mip: 0, x: 16, y: 16, screenPriority: 0, coverage: 8 }],
    ]));
    for (let dispatch = 0; dispatch < 7; dispatch++) {
      store.poll();
      await flush();
    }
    expect(calls.slice(0, 6)).toEqual([
      { mip: 0, x: 16, y: 16 },
      { mip: 2, x: 0, y: 0 },
      { mip: 2, x: 4, y: 4 },
      { mip: 0, x: 0, y: 0 },
      { mip: 0, x: 24, y: 24 },
      { mip: 2, x: 6, y: 6 },
    ]);
  });

  test('balances predicted center against camera-close edge detail', async () => {
    const firstCalls: Array<{ mip: number; x: number }> = [];
    const edgeStore = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      firstCalls.push({ mip: req.mip, x: req.x });
      return new Uint8Array(PAGE_BYTES);
    });
    edgeStore.loadTexture('balanced-edge', { width: 4096, height: 4096 });
    await settle(edgeStore);
    firstCalls.length = 0;
    edgeStore.setDebugPageBudget(1);
    edgeStore.processFeedback(new Map([
      ['deep-center', {
        path: 'balanced-edge', mip: 0, x: 0, y: 0,
        coverage: 1, perceptualWeight: 8,
      }],
      ['close-edge', {
        path: 'balanced-edge', mip: 0, x: 16, y: 0,
        coverage: 1, perceptualWeight: 15,
      }],
    ]));
    edgeStore.poll();
    await flush();
    expect(firstCalls[0]?.x).toBeGreaterThan(0);

    const inverseCalls: Array<{ mip: number; x: number }> = [];
    const centerStore = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      inverseCalls.push({ mip: req.mip, x: req.x });
      return new Uint8Array(PAGE_BYTES);
    });
    centerStore.loadTexture('balanced-center', { width: 4096, height: 4096 });
    await settle(centerStore);
    inverseCalls.length = 0;
    centerStore.setDebugPageBudget(1);
    centerStore.processFeedback(new Map([
      ['distant-edge', {
        path: 'balanced-center', mip: 0, x: 16, y: 0,
        coverage: 1, perceptualWeight: 8,
      }],
      ['close-center', {
        path: 'balanced-center', mip: 0, x: 0, y: 0,
        coverage: 1, perceptualWeight: 15,
      }],
    ]));
    centerStore.poll();
    await flush();
    expect(inverseCalls[0]?.x).toBe(0);
    edgeStore.dispose();
    centerStore.dispose();
  });

  test('cooperatively cancels stale reads before they enter later stages', async () => {
    let aborted = false;
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req, signal) => {
      if (req.mip !== 0) return new Uint8Array(PAGE_BYTES);
      return new Promise<Uint8Array>((_resolve, reject) => {
        signal?.addEventListener('abort', () => {
          aborted = true;
          reject(new Error('canceled'));
        }, { once: true });
      });
    });
    store.loadTexture('cancel-stage', { width: 512, height: 512 });
    await settle(store);
    store.processFeedback(new Map([['fine', { path: 'cancel-stage', mip: 0, x: 0, y: 0 }]]));
    store.poll();
    for (let epoch = 0; epoch < 17; epoch++) store.processFeedback(new Map());
    await flush();
    expect(aborted).toBe(true);
    expect(store.getStats().pendingPages).toBe(0);
    expect(store.getStats().failedLoads).toBe(0);
  });

  test('drops queued requests after two absent feedback snapshots', async () => {
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) =>
      req.mip >= 4 ? new Uint8Array(PAGE_BYTES) : never);
    store.loadTexture('stale', { width: 4096, height: 4096 });
    await settle(store);
    const feedback = new Map<string, VirtualPageRequest>();
    for (let page = 0; page < 16; page++) {
      feedback.set(`${page}`, { path: 'stale', mip: 0, x: (page & 3) << 3, y: (page >> 2) << 3 });
    }
    store.processFeedback(feedback);
    store.poll();
    for (let epoch = 0; epoch < 2; epoch++) {
      store.processFeedback(new Map());
      store.poll();
    }
    expect(store.getStats().staleCancellations).toBeGreaterThan(0);
    expect(store.getStats().scheduledRequests).toBe(0);
  });

  test('bounds admitted asynchronous page work at the selected sixteen-page profile', async () => {
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(NO_CACHE_CAPACITIES, async (_path, req) =>
      req.mip >= 4 ? new Uint8Array(PAGE_BYTES) : never);
    for (let texture = 0; texture < 5; texture++) {
      store.loadTexture(`bounded-${texture}`, { width: 4096, height: 4096 });
      await settle(store);
    }
    const feedback = new Map<string, VirtualPageRequest>();
    for (let texture = 0; texture < 5; texture++) for (let page = 0; page < 16; page++) {
      const request = {
        path: `bounded-${texture}`, mip: 0,
        x: (page & 3) << 3, y: (page >> 2) << 3,
      };
      feedback.set(`${texture}:${page}`, request);
    }
    store.processFeedback(feedback);
    for (let frame = 0; frame < 12; frame++) store.poll();
    const stats = store.getStats();
    expect(stats.pendingPages).toBe(16);
    expect(stats.maxPendingPages).toBe(16);
    expect(stats.maxPendingBytes).toBe(2 * 1024 * 1024);
    expect(stats.scheduledRequests).toBeGreaterThan(0);
    expect(stats.failedLoads).toBe(0);
    expect(stats.rejectedAdmissions).toBeGreaterThan(0);
  });

  test('preempts worse in-flight edge work for a newly visible center page', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (path, req, signal) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      if (req.mip >= 4) return new Uint8Array(PAGE_BYTES);
      return new Promise<Uint8Array>((_resolve, reject) =>
        signal?.addEventListener('abort', () => reject(new Error('canceled')), { once: true }));
    });
    for (let texture = 0; texture < 6; texture++) {
      store.loadTexture(`preempt-${texture}`, { width: 4096, height: 4096 });
      await settle(store);
    }
    calls.length = 0;
    const edge = new Map<string, VirtualPageRequest>();
    for (let texture = 0; texture < 5; texture++) for (let page = 0; page < 16; page++) {
      edge.set(`${texture}:${page}`, {
        path: `preempt-${texture}`, mip: 0,
        x: (page & 3) << 3, y: (page >> 2) << 3,
        screenPriority: 255, coverage: 1,
      });
    }
    store.processFeedback(edge);
    for (let frame = 0; frame < 8; frame++) store.poll();
    expect(store.getStats().pendingPages).toBe(64);

    store.processFeedback(new Map([['new-center', {
      path: 'preempt-5', mip: 0, x: 0, y: 0, screenPriority: 0, coverage: 8,
    }]]));
    store.poll();
    expect(store.getStats().priorityPreemptions).toBe(1);
    await flush();
    store.poll();
    expect(store.getStats().pendingPages).toBe(64);
    expect(calls).toContain('preempt-5:0:0:0');
  });

  test('coarsens progressive feedback only when its working set exceeds atlas capacity', async () => {
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async () => new Uint8Array(PAGE_BYTES));
    for (let texture = 0; texture < 14; texture++) {
      store.loadTexture(`oversubscribed-${texture}`, { width: 4096, height: 4096 });
      await settle(store);
    }
    const feedback = new Map<string, VirtualPageRequest>();
    for (let texture = 0; texture < 14; texture++) for (let page = 0; page < 16; page++) {
      const request = {
        path: `oversubscribed-${texture}`, mip: 0,
        x: (page & 3) << 3, y: (page >> 2) << 3,
      };
      feedback.set(`${texture}:${page}`, request);
    }
    const result = store.processFeedback(feedback);
    expect(result.totalRequests).toBeLessThanOrEqual(155);
    expect(result.lodBias).toBeGreaterThan(0);
  });

  test('retains pinned startup overflow in the fixed scheduler', async () => {
    const store = new VT.VirtualTextureStore(
      { maxPendingPages: 1, maxPendingBytes: PAGE_BYTES },
      async () => new Uint8Array(PAGE_BYTES),
    );
    store.loadTexture('first-tail', { width: 128, height: 128, mipTail: true });
    store.loadTexture('second-tail', { width: 128, height: 128, mipTail: true });
    expect(store.getStats().pendingPages).toBe(1);
    expect(store.getStats().scheduledRequests).toBeGreaterThan(0);
    for (let iteration = 0; iteration < 16; iteration++) {
      await flush();
      store.poll();
    }
    expect((store.getEntry('first-tail')?.tailEntry ?? 0) & 1).toBe(1);
    expect((store.getEntry('second-tail')?.tailEntry ?? 0) & 1).toBe(1);
    expect(store.getStats().scheduledRequests).toBe(0);
  });

  test('pins terminal mips relative to each texture size', async () => {
    const calls: Array<{ mip: number; x: number; y: number }> = [];
    const store = new VT.VirtualTextureStore(TEST_CAPACITIES, async (_path, req) => {
      calls.push(req);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('small', { width: 512, height: 512 });
    await flush();
    expect(calls.some(req => req.mip === 2)).toBe(true);
    expect(calls.filter(req => req.mip === 1)).toHaveLength(4);
    expect(calls.some(req => req.mip > 2)).toBe(false);
  });
});
