import { describe, expect, test } from 'bun:test';
import {
  MemoryTextureWriteStatus,
  MemoryVirtualTextureSource,
} from './memory-page-source.ts';
import { PAGE_BORDER, SLOT_SIZE } from './virtual-texture-format.ts';

function pixel(page: Uint8Array, x: number, y: number, components: number): number[] {
  const start = (y * SLOT_SIZE + x) * components;
  return Array.from(page.subarray(start, start + components));
}

describe('MemoryVirtualTextureSource', () => {
  test('stores only touched sparse pages and returns bordered runtime RGBA pages', () => {
    const source = new MemoryVirtualTextureSource({
      width: 256, height: 256, format: 'rgba8unorm', mipFilter: 'scalar',
      addressMode: 'clamp', pageCapacity: 8, dirtyCapacity: 8, outputCapacity: 2,
      defaultTexel: new Uint8Array([0, 0, 0, 0]),
    });
    const texel = new Uint8Array([220, 30, 10, 255]);
    expect(source.writeRegion(127, 8, 1, 1, texel)).toBe(MemoryTextureWriteStatus.Written);
    expect(source.pageCount).toBeGreaterThan(0);
    const left = source.readPage({ mip: 0, x: 0, y: 0 });
    const right = source.readPage({ mip: 0, x: 1, y: 0 });
    expect(pixel(left, PAGE_BORDER + 127, PAGE_BORDER + 8, 4)).toEqual(Array.from(texel));
    // The right page's four-texel border sees the edited neighboring texel.
    expect(pixel(right, PAGE_BORDER - 1, PAGE_BORDER + 8, 4)).toEqual(Array.from(texel));
  });

  test('regenerates dependent mips and drains a bounded dirty prefix', () => {
    const source = new MemoryVirtualTextureSource({
      width: 256, height: 256, format: 'r8unorm', mipFilter: 'scalar',
      addressMode: 'clamp', pageCapacity: 8, dirtyCapacity: 8, outputCapacity: 2,
    });
    expect(source.writeRegion(0, 0, 2, 2, new Uint8Array([80, 80, 80, 80]), 2))
      .toBe(MemoryTextureWriteStatus.Written);
    expect(source.writeRegion(0, 0, 2, 2, new Uint8Array([100, 100, 100, 100]), 2))
      .toBe(MemoryTextureWriteStatus.Written);
    const mip = source.readPage({ mip: 1, x: 0, y: 0 });
    expect(pixel(mip, PAGE_BORDER, PAGE_BORDER, 1)).toEqual([100]);
    const revisions: number[] = [];
    expect(source.drainDirty(1, (page) => { revisions.push(page.revision); return true; })).toBe(1);
    expect(source.pendingDirtyPages).toBeGreaterThan(0);
    source.drainDirty(8, (page) => { revisions.push(page.revision); return true; });
    expect(source.pendingDirtyPages).toBe(0);
    expect(new Set(revisions)).toEqual(new Set([source.contentRevision]));
  });

  test('preflights fixed page and dirty capacities without partial writes', () => {
    const pages = new MemoryVirtualTextureSource({
      width: 256, height: 256, format: 'rgba8unorm', mipFilter: 'linear-color',
      addressMode: 'clamp', pageCapacity: 1, dirtyCapacity: 8, outputCapacity: 1,
    });
    const data = new Uint8Array(2 * 2 * 4).fill(255);
    expect(pages.writeRegion(127, 127, 2, 2, data))
      .toBe(MemoryTextureWriteStatus.PageCapacityExceeded);
    expect(pages.pageCount).toBe(0);
    expect(pages.contentRevision).toBe(0);

    const dirty = new MemoryVirtualTextureSource({
      width: 256, height: 256, format: 'rgba8unorm', mipFilter: 'linear-color',
      addressMode: 'clamp', pageCapacity: 16, dirtyCapacity: 1, outputCapacity: 1,
    });
    expect(dirty.writeRegion(127, 127, 2, 2, data))
      .toBe(MemoryTextureWriteStatus.DirtyCapacityExceeded);
    expect(dirty.pageCount).toBe(0);
  });

  test('leases fixed asynchronous page outputs and reuses refresh scratch', async () => {
    const source = new MemoryVirtualTextureSource({
      width: 128, height: 128, format: 'r8unorm', mipFilter: 'scalar',
      addressMode: 'clamp', pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 1,
    });
    expect(source.writeRegion(0, 0, 1, 1, new Uint8Array([7])))
      .toBe(MemoryTextureWriteStatus.Written);
    const first = await source.provider('ignored', { mip: 0, x: 0, y: 0 });
    expect(first).not.toBeInstanceOf(Uint8Array);
    if (first instanceof Uint8Array) throw new Error('expected a fixed output lease');
    const firstBytes = first.bytes;
    expect(source.activeOutputPages).toBe(1);
    await expect(source.provider('ignored', { mip: 0, x: 0, y: 0 }))
      .rejects.toThrow('output pool is full');
    expect(first.release()).toBe(true);
    expect(first.release()).toBe(false);
    const second = await source.provider('ignored', { mip: 0, x: 0, y: 0 });
    if (second instanceof Uint8Array) throw new Error('expected a fixed output lease');
    expect(second.bytes).toBe(firstBytes);
    second.release();

    const outputs: Uint8Array[] = [];
    source.drainDirty(1, (_page, bytes) => { outputs.push(bytes); return false; });
    source.drainDirty(1, (_page, bytes) => { outputs.push(bytes); return true; });
    expect(outputs[1]).toBe(outputs[0]);
  });

  test('keeps R16 float source samples in canonical RAM', () => {
    // IEEE half 0.5 = 0x3800.
    const source = new MemoryVirtualTextureSource({
      width: 128, height: 128, format: 'r16float', mipFilter: 'scalar',
      addressMode: 'clamp', pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 1,
    });
    expect(source.writeRegion(4, 5, 1, 1, new Uint8Array([0x00, 0x38])))
      .toBe(MemoryTextureWriteStatus.Written);
    const page = source.readPage({ mip: 0, x: 0, y: 0 });
    expect(pixel(page, PAGE_BORDER + 4, PAGE_BORDER + 5, 2)).toEqual([0x00, 0x38]);
  });
});
