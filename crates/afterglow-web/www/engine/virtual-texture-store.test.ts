import { beforeAll, describe, expect, mock, test } from 'bun:test';
import { packedPageTableIndex } from './virtual-texture-layout.js';
import type { VirtualPageRequest } from './virtual-texture.js';

class Texture { needsUpdate = false; dispose() {} }
class DataTexture extends Texture {
  minFilter: unknown; magFilter: unknown; generateMipmaps = false;
  constructor(public data: Uint8Array | Uint32Array, public width: number, public height: number) { super(); }
}
class CompressedTexture extends Texture {}
mock.module('three', () => ({
  Texture, DataTexture, CompressedTexture,
  RGBAFormat: 1, RedIntegerFormat: 2, UnsignedIntType: 3,
  LinearFilter: 4, NearestFilter: 5,
}));

type VTModule = typeof import('./virtual-texture.js');
let VT: VTModule;
beforeAll(async () => { VT = await import('./virtual-texture.js'); });

const PAGE_BYTES = 136 * 136 * 4;
const loader = { read: async () => new Uint8Array(), poll() {} };
const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe('VirtualTextureStore residency identity', () => {
  test('deduplicates in-flight requests but keeps texture paths distinct', async () => {
    const calls: string[] = [];
    const store = new VT.VirtualTextureStore(loader, async (path, req) => {
      calls.push(`${path}:${req.mip}:${req.x}:${req.y}`);
      await flush();
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('a', { virtualSize: 512 });
    store.loadTexture('b', { virtualSize: 512 });
    await flush();

    const a: VirtualPageRequest = { path: 'a', mip: 0, x: 0, y: 0 };
    const b: VirtualPageRequest = { path: 'b', mip: 0, x: 0, y: 0 };
    store.processFeedback(new Map([['a', a], ['b', b]]));
    store.processFeedback(new Map([['a duplicate', a], ['b duplicate', b]]));
    expect(calls.filter(key => key === 'a:0:0:0')).toHaveLength(1);
    expect(calls.filter(key => key === 'b:0:0:0')).toHaveLength(1);

    await flush(); await flush();
    const entryA = store.getEntry('a')!;
    const entryB = store.getEntry('b')!;
    expect(entryA.pageTable[packedPageTableIndex(entryA.pageTableLayout, 0, 0, 0)] & 1).toBe(1);
    expect(entryB.pageTable[packedPageTableIndex(entryB.pageTableLayout, 0, 0, 0)] & 1).toBe(1);
  });

  test('cancels reserved asynchronous work on unload', async () => {
    let resolvePage!: (data: Uint8Array) => void;
    const store = new VT.VirtualTextureStore(loader, () => new Promise(resolve => { resolvePage = resolve; }));
    store.loadTexture('temporary', { virtualSize: 128 });
    expect(store.getStats().pendingPages).toBe(1);
    store.unloadTexture('temporary');
    expect(store.getStats().pendingPages).toBe(0);
    expect(store.getEntry('temporary')).toBeUndefined();
    resolvePage(new Uint8Array(PAGE_BYTES));
    await flush();
    expect(store.getStats().atlasSlotsUsed).toBe(0);
  });

  test('loads one packed mip tail into a pinned physical slot', async () => {
    const requests: Array<{ mip: number; tail?: boolean }> = [];
    const store = new VT.VirtualTextureStore(loader, async (_path, req) => {
      requests.push(req);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('tail', { virtualSize: 512, mipTail: true });
    await flush(); await flush();
    expect(requests.filter(request => request.tail)).toHaveLength(1);
    const entry = store.getEntry('tail')!;
    expect(entry.tailFirstMip).toBe(3);
    expect(entry.textureMaxMip).toBe(9);
    expect(entry.tailEntry & 1).toBe(1);
  });

  test('pins terminal mips relative to each texture size', async () => {
    const calls: Array<{ mip: number; x: number; y: number }> = [];
    const store = new VT.VirtualTextureStore(loader, async (_path, req) => {
      calls.push(req);
      return new Uint8Array(PAGE_BYTES);
    });
    store.loadTexture('small', { virtualSize: 512 });
    await flush();
    expect(calls.some(req => req.mip === 2)).toBe(true);
    expect(calls.filter(req => req.mip === 1)).toHaveLength(4);
    expect(calls.some(req => req.mip > 2)).toBe(false);
  });
});
