import { describe, expect, test } from 'bun:test';
import { RenderAdapter, RenderAttachStatus } from './render-adapter.ts';
import { RenderTier } from './types.ts';

describe('RenderAdapter prewarmed unique proxy pool', () => {
  test('never instantiates after warm-up and reports deterministic exhaustion', () => {
    const attached: unknown[] = [];
    const scene = {
      add(object: unknown) { attached.push(object); },
      remove(object: unknown) { const index = attached.indexOf(object); if (index >= 0) attached.splice(index, 1); },
    };
    const adapter = new RenderAdapter(scene as never, 16);
    let instantiated = 0;
    let synced = 0;
    const descriptor = adapter.registry.register({
      tier: RenderTier.Unique,
      poolCapacity: 2,
      instantiate() { instantiated++; return { matrixAutoUpdate: true }; },
      continuous: true,
      sync() { synced++; },
    } as never);
    const internal = adapter as unknown as {
      attachProxy(entity: number, descriptor: number): RenderAttachStatus;
      detachProxy(entity: number): void;
      syncUniqueProxies(frame: unknown): void;
    };
    expect(internal.attachProxy(1, descriptor)).toBe(RenderAttachStatus.DescriptorNotWarmed);
    expect(() => adapter.sealGameplay()).toThrow('not warmed');
    adapter.warmDescriptor(descriptor);
    adapter.sealGameplay();
    expect(adapter.isGameplaySealed).toBe(true);
    expect(instantiated).toBe(2);
    expect(internal.attachProxy(1, descriptor)).toBe(RenderAttachStatus.Attached);
    expect(internal.attachProxy(2, descriptor)).toBe(RenderAttachStatus.Attached);
    expect(internal.attachProxy(3, descriptor)).toBe(RenderAttachStatus.CapacityExceeded);
    internal.syncUniqueProxies({ frameId: 1, deltaSeconds: 0, elapsedSeconds: 0 });
    expect(synced).toBe(2);
    internal.detachProxy(1);
    expect(internal.attachProxy(3, descriptor)).toBe(RenderAttachStatus.Attached);
    expect(instantiated).toBe(2);
    expect(attached).toHaveLength(2);
  });
});
