import { describe, expect, test } from 'bun:test';
import { MemoryPersistentBlobBackend, PersistentBlobStore } from '../streaming/persistent-blob-store.ts';
import { MemoryTextureWriteStatus } from './memory-page-source.ts';
import {
  INSPECT_VIRTUAL_TEXTURE,
  MemoryTexturePersistenceStatus,
  VirtualTextureSystem,
} from './virtual-texture-system.ts';
import { VirtualTextureTuning } from './virtual-texture-tuning.ts';
import { SLOT_SIZE } from './virtual-texture-format.ts';

const flush = async () => { await Promise.resolve(); await new Promise(resolve => setTimeout(resolve, 0)); };

function device(): GPUDevice {
  return {
    limits: { maxTextureDimension2D: SLOT_SIZE * 4 },
    queue: { writeTexture() {} },
  } as unknown as GPUDevice;
}

describe('VirtualTextureSystem', () => {
  test('routes generated identities to immutable source keys', async () => {
    const seen: string[] = [];
    const provider = async (path: string) => {
      seen.push(path);
      return new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4);
    };
    const system = new VirtualTextureSystem({
      maxTextures: 1, maxMutablePageRefreshesPerPoll: 1, device: device(),
      pools: [{
        format: 'rgba8unorm',
        capacities: { maxPendingPages: 1, maxPendingBytes: 256 * 1024 },
        tuning: new VirtualTextureTuning({ atlasMaxDimension: SLOT_SIZE * 4 }),
      }],
    });
    expect(system.createTexture({
      width: 128, height: 128, format: 'rgba8unorm', addressMode: 'clamp', mipTail: true,
    }, provider, 'packed/image')).not.toBe(0);
    await flush();
    system.poll();
    const entry = system.getEntryById(1)!;
    system.processFeedback(new Map([['page', {
      textureId: entry.textureId,
      path: entry.path,
      mip: 0,
      x: 0,
      y: 0,
      screenPriority: 1,
      coverage: 1,
    }]]));
    system.poll();
    await flush();
    expect(seen.every(path => path === 'packed/image')).toBe(true);
    system.dispose();
  });

  test('uses one handle/source API for bounded RAM textures in independent format pools', async () => {
    const tuning = () => new VirtualTextureTuning({ atlasMaxDimension: SLOT_SIZE * 4 });
    const system = new VirtualTextureSystem({
      maxTextures: 2,
      maxMutablePageRefreshesPerPoll: 2,
      device: device(),
      pools: [
        { format: 'rgba8unorm-srgb', capacities: { maxPendingPages: 2, maxPendingBytes: 512 * 1024 }, tuning: tuning() },
        { format: 'r8unorm', capacities: { maxPendingPages: 2, maxPendingBytes: 128 * 1024 }, tuning: tuning() },
      ],
    });
    const color = system.createMemoryTexture({
      width: 128, height: 128, format: 'rgba8unorm-srgb', addressMode: 'clamp',
    }, { mipFilter: 'linear-color', pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 2 });
    const mask = system.createMemoryTexture({
      width: 128, height: 128, format: 'r8unorm', addressMode: 'clamp',
    }, { mipFilter: 'scalar', pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 2 });
    expect(color).not.toBe(0);
    expect(mask).not.toBe(0);
    expect(system.createMemoryTexture({
      width: 128, height: 128, format: 'r8unorm', addressMode: 'clamp',
    }, { mipFilter: 'scalar', pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 1 })).toBe(0);
    if (color === 0 || mask === 0) throw new Error('texture registration failed');
    expect(system[INSPECT_VIRTUAL_TEXTURE](color)?.descriptor.format).toBe('rgba8unorm-srgb');
    expect(system[INSPECT_VIRTUAL_TEXTURE](mask)?.descriptor.format).toBe('r8unorm');
    const colorId = system[INSPECT_VIRTUAL_TEXTURE](color)?.entry.textureId ?? 0;
    const maskId = system[INSPECT_VIRTUAL_TEXTURE](mask)?.entry.textureId ?? 0;
    expect(colorId).not.toBe(maskId);
    expect(system.getEntryById(colorId)).toBe(system[INSPECT_VIRTUAL_TEXTURE](color)?.entry);
    expect(system.getEntryById(maskId)).toBe(system[INSPECT_VIRTUAL_TEXTURE](mask)?.entry);
    await flush();
    system.poll();
    const initialUploads = system[INSPECT_VIRTUAL_TEXTURE](mask)?.store.getStats().completedUploads ?? 0;
    expect(system.writeMemoryRegion(mask, 3, 4, 1, 1, new Uint8Array([220])))
      .toBe(MemoryTextureWriteStatus.Written);
    system.poll();
    expect(system[INSPECT_VIRTUAL_TEXTURE](mask)?.store.getStats().completedUploads).toBeGreaterThan(initialUploads);
    const blobs = await PersistentBlobStore.open(new MemoryPersistentBlobBackend(), {
      maxItems: 2, maxBytes: 64 * 1024, maxValueBytes: 32 * 1024,
      maxInFlightOperations: 1, maxInFlightBytes: 32 * 1024,
    });
    expect(await system.saveMemoryTexture(mask, 'paint', blobs))
      .toBe(MemoryTexturePersistenceStatus.Ok);
    expect(system.destroyTexture(mask)).toBe(true);
    expect(system[INSPECT_VIRTUAL_TEXTURE](mask)).toBeNull();
    const restored = await system.loadMemoryTexture('paint', blobs, {
      pageCapacity: 1, dirtyCapacity: 1, outputCapacity: 2,
    }, 32 * 1024);
    expect(restored.status).toBe(MemoryTexturePersistenceStatus.Ok);
    if (restored.handle === 0) throw new Error('snapshot restore failed');
    expect(system[INSPECT_VIRTUAL_TEXTURE](restored.handle)?.descriptor.format).toBe('r8unorm');
    system.dispose();
    expect(system[INSPECT_VIRTUAL_TEXTURE](color)).toBeNull();
  });
});
