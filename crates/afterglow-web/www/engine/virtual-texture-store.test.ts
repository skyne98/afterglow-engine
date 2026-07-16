import { beforeAll, describe, expect, mock, test } from 'bun:test';
import { packedPageTableIndex } from './virtual-texture-layout.ts';
import type { PageRequest, VirtualPageRequest } from './virtual-texture.ts';

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
mock.module('three', () => ({
  Texture, DataTexture, CompressedTexture, RenderTarget,
  RGBAFormat: 1, RedIntegerFormat: 2, UnsignedIntType: 3, RGIntegerFormat: 6,
  LinearFilter: 4, NearestFilter: 5,
}));

type VTModule = typeof import('./virtual-texture.ts');
let VT: VTModule;
beforeAll(async () => { VT = await import('./virtual-texture.ts'); });

const PAGE_BYTES = 136 * 136 * 4;
const loader = { read: async () => new Uint8Array(), poll() {} };
const flush = () => new Promise(resolve => setTimeout(resolve, 0));
const settle = async (store: { poll(): void }) => {
  await flush(); await flush();
  store.poll();
  await flush(); await flush();
  for (let i = 1; i < 32; i++) store.poll();
};

describe('VirtualTextureStore residency identity', () => {
  test('deduplicates in-flight requests but keeps texture paths distinct', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(loader, async (path, req) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      await flush();
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('a', { width: 512, height: 512 });
    store.loadTexture('b', { width: 512, height: 512 });
    await flush();

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
    const store = new VT.VirtualTextureStore(loader, () => new Promise(resolve => { resolvePage = resolve; }));
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
    const store = new VT.VirtualTextureStore(loader, async (_path, req) =>
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
    const store = new VT.VirtualTextureStore(loader, async (_path, req) => {
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
    const store = new VT.VirtualTextureStore(loader, async () => page);
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
    const store = new VT.VirtualTextureStore(loader, async () => new Uint8Array(PAGE_BYTES));
    store.loadTexture('rgba', { width: 512, height: 512 });
    await settle(store);
    store.attachRenderer({ backend: { device, get: () => ({ texture: {} as GPUTexture }) } });
    const req: VirtualPageRequest = { path: 'rgba', mip: 0, x: 0, y: 0 };
    store.processFeedback(new Map([['page', req]]));
    await settle(store);
    expect(layouts.some(layout => layout.bytesPerRow === 136 * 4 && layout.rowsPerImage === 136)).toBe(true);
  });

  test('expands one material feedback page to every linked PBR channel', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(loader, async (path, req) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadMaterialSet({ albedo: 'color', normal: 'normal', roughness: 'rough', ao: 'ao' }, { width: 512, height: 512 });
    await settle(store); calls.length = 0;
    const request: VirtualPageRequest = { path: 'color', mip: 0, x: 0, y: 0 };
    store.processFeedback(new Map([['visible', request]]));
    await settle(store);
    expect(new Set(calls.map(call => call.split(':')[0]))).toEqual(new Set(['color', 'normal', 'rough', 'ao']));
  });

  test('expands packed-mask material feedback to three physical pages', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(loader, async (path) => {
      calls.push(path);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadMaterialSet({ albedo: 'color', normal: 'normal', masks: 'masks' }, { width: 512, height: 512 });
    await settle(store); calls.length = 0;
    store.processFeedback(new Map([['visible', { path: 'color', mip: 0, x: 0, y: 0 }]]));
    await settle(store);
    expect(new Set(calls)).toEqual(new Set(['color', 'normal', 'masks']));
  });

  test('evicts with a bounded second-chance clock at full capacity', async () => {
    const device = { limits: { maxTextureDimension2D: 408 } } as GPUDevice; // 3x3 slots
    const store = new VT.VirtualTextureStore(loader, async () => new Uint8Array(PAGE_BYTES), VT.FORMAT_RGBA, device);
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

  test('evicts one logical packed-mask material page across every channel', async () => {
    const device = { limits: { maxTextureDimension2D: 816 } } as GPUDevice; // 6x6 slots
    const store = new VT.VirtualTextureStore(loader, async () => new Uint8Array(PAGE_BYTES), VT.FORMAT_RGBA, device);
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
    for (let page = 0; page < 16; page++) {
      const index = packedPageTableIndex(material.albedo.pageTableLayout, 0, page % 4, Math.floor(page / 4));
      const states = [material.albedo, material.normal!, material.masks!]
        .map(entry => entry.pageTable[index] & 1);
      expect(new Set(states).size).toBe(1);
    }
  });

  test('owns async page coordinates when scheduler scratch slots are reused', async () => {
    const captured: PageRequest[] = [];
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(loader, async (_path, req) => {
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

  test('persists visible requests across polls instead of dropping the frame overflow', () => {
    const calls: string[] = [];
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(loader, async (path, req) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      return never;
    });
    store.loadTexture('persistent', { width: 4096, height: 4096 });
    const feedback = new Map<string, VirtualPageRequest>();
    for (let page = 0; page < 20; page++) {
      const request = { path: 'persistent', mip: 0, x: page % 32, y: Math.floor(page / 32) };
      feedback.set(`${page}`, request);
    }
    const first = store.processFeedback(feedback);
    expect(first.loaded).toBe(0);
    expect(first.queuedRequests).toBe(20);
    for (let frame = 0; frame < 3; frame++) store.poll();
    expect(calls.filter(call => call.startsWith('persistent:0:'))).toHaveLength(20);
    expect(store.getStats().scheduledRequests).toBe(0);
  });

  test('cooperatively cancels stale reads before they enter later stages', async () => {
    let aborted = false;
    const store = new VT.VirtualTextureStore(loader, async (_path, req, signal) => {
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

  test('drops queued requests after sixteen newer feedback snapshots', () => {
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(loader, async () => never);
    store.loadTexture('stale', { width: 4096, height: 4096 });
    const feedback = new Map<string, VirtualPageRequest>();
    for (let page = 0; page < 100; page++) {
      feedback.set(`${page}`, { path: 'stale', mip: 0, x: page % 32, y: Math.floor(page / 32) });
    }
    store.processFeedback(feedback);
    store.poll();
    for (let epoch = 0; epoch < 17; epoch++) {
      store.processFeedback(new Map());
      store.poll();
    }
    expect(store.getStats().staleCancellations).toBeGreaterThan(0);
    expect(store.getStats().scheduledRequests).toBe(0);
  });

  test('bounds admitted asynchronous page work', () => {
    const never = new Promise<Uint8Array>(() => {});
    const store = new VT.VirtualTextureStore(loader, async () => never);
    store.loadTexture('bounded', { width: 4096, height: 4096 });
    for (let batch = 0; batch < 20; batch++) {
      const feedback = new Map<string, VirtualPageRequest>();
      for (let i = 0; i < 32; i++) {
        const page = batch * 32 + i;
        const request = { path: 'bounded', mip: 0, x: page % 32, y: Math.floor(page / 32) % 32 };
        feedback.set(`${page}`, request);
      }
      store.processFeedback(feedback);
      store.poll();
    }
    const stats = store.getStats();
    expect(stats.pendingPages).toBe(64);
    expect(stats.maxPendingPages).toBe(64);
    expect(stats.rejectedAdmissions).toBeGreaterThan(0);
  });

  test('coarsens GPU feedback only when its working set exceeds atlas capacity', async () => {
    const store = new VT.VirtualTextureStore(loader, async () => new Uint8Array(PAGE_BYTES));
    store.loadTexture('oversubscribed', { width: 4096, height: 4096 });
    await settle(store);
    const feedback = new Map<string, VirtualPageRequest>();
    for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) {
      const request = { path: 'oversubscribed', mip: 0, x, y };
      feedback.set(`${x}:${y}`, request);
    }
    const result = store.processFeedback(feedback);
    expect(result.totalRequests).toBe(64);
    expect(result.lodBias).toBe(1);
  });

  test('pins terminal mips relative to each texture size', async () => {
    const calls: Array<{ mip: number; x: number; y: number }> = [];
    const store = new VT.VirtualTextureStore(loader, async (_path, req) => {
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
