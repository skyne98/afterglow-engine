import { describe, expect, test } from 'bun:test';
import { defineResource, initResources, ResourceManifest, resourcesAreSealed, sealResources } from './resource.ts';

describe('sealed engine resources', () => {
  test('allows eager initialization and rejects lazy gameplay allocation', () => {
    const world = {};
    initResources(world);
    const eager = defineResource('eager-test', () => ({ value: 7 }));
    const late = defineResource('late-test', () => ({ value: 9 }));
    expect(eager.get(world).value).toBe(7);
    sealResources(world);
    expect(resourcesAreSealed(world)).toBe(true);
    expect(eager.get(world).value).toBe(7);
    expect(() => late.get(world)).toThrow('was not initialized');
  });

  test('eagerly initializes and verifies every manifest resource before seal', () => {
    const world = {};
    let factories = 0;
    const injected = defineResource<{ value: number }>('manifest-injected', () => {
      throw new Error('must be injected');
    });
    const generated = defineResource('manifest-generated', () => ({ value: ++factories }));
    injected.set(world, { value: 5 });
    const manifest = new ResourceManifest(injected, generated);
    manifest.initializeAndSeal(world);
    expect(injected.get(world).value).toBe(5);
    expect(generated.get(world).value).toBe(1);
    expect(factories).toBe(1);
    expect(resourcesAreSealed(world)).toBe(true);
  });

  test('rejects missing and duplicate manifest declarations', () => {
    const resource = defineResource('manifest-required', () => 1);
    expect(() => new ResourceManifest(resource, resource)).toThrow('duplicate');
    const manifest = new ResourceManifest(resource);
    expect(() => manifest.seal({})).toThrow('missing before gameplay seal');
  });
});
