import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../document.ts', import.meta.url), 'utf8');
test('document surface clears each frame and publishes readiness once', async () => {
  const calls: string[] = [];
  const device = {
    createCommandEncoder: () => ({
      beginRenderPass: (descriptor: any) => {
        expect(descriptor.colorAttachments[0].loadOp).toBe('clear');
        calls.push('clear');
        return { end() {} };
      },
      finish: () => ({}),
    }),
    queue: { submit: () => calls.push('submit') },
  };
  const context = vm.createContext({
    navigator: { gpu: {
      requestAdapter: async () => ({ requestDevice: async () => device }),
      getPreferredCanvasFormat: () => 'bgra8unorm',
    } },
    engineCanvas: { getContext: () => ({
      configure: () => calls.push('configure'),
      getCurrentTexture: () => ({ createView: () => ({}) }),
    }) },
    Deno: { core: {
      reportUnhandledException(error: unknown) { throw error; },
      ops: {
        op_try_present_surface: () => { calls.push('present'); return true; },
        op_engine_ready: () => calls.push('ready'),
      },
    } },
  });
  vm.runInContext(source, context);
  await new Promise(resolve => setImmediate(resolve));
  vm.runInContext('__presentDocument(); globalThis.__documentLoaded = true; __presentDocument(); __presentDocument();', context);
  expect(calls).toEqual(['configure', 'clear', 'submit', 'present', 'clear', 'submit', 'present', 'ready', 'clear', 'submit', 'present']);
});

test('document adapter failure has no fallback', async () => {
  let error = '';
  const context = vm.createContext({
    navigator: { gpu: { requestAdapter: async () => null } },
    Deno: { core: { reportUnhandledException: (failure: Error) => { error = failure.message; } } },
  });
  vm.runInContext(source, context);
  await new Promise(resolve => setImmediate(resolve));
  expect(error).toContain('No native document GPU adapter');
});
