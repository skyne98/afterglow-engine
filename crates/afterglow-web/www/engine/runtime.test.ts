import { describe, expect, test } from 'bun:test';
import { EnginePhase, type EngineMemoryConfig } from './engine-memory.ts';
import {
  EngineRuntime, RegistrationStatus, RuntimeState,
  type AnimationScheduler, type EngineRenderPass, type RuntimeRenderAdapter,
} from './runtime.ts';
import type { RenderFrame } from './types.ts';

const memory: EngineMemoryConfig = {
  frameScratchBytes: 64,
  renderScratchBytes: 64,
  structuralCommands: 4,
  workerCompletions: 4,
  assetRequests: 4,
  vtRequests: 4,
};

class FakeScheduler implements AnimationScheduler {
  callback: ((timestamp: number) => void) | null = null;
  requests = 0;
  cancels = 0;
  private nextHandle = 1;

  request(callback: (timestamp: number) => void): number {
    this.callback = callback;
    this.requests++;
    return this.nextHandle++;
  }
  cancel(): void { this.cancels++; this.callback = null; }
  fire(timestamp: number): void {
    const callback = this.callback;
    if (callback === null) throw new Error('no scheduled frame');
    this.callback = null;
    callback(timestamp);
  }
}

class FakeAdapter implements RuntimeRenderAdapter {
  readonly world = {};
  isGameplaySealed = false;
  disposed = 0;
  readonly events: string[];

  constructor(events: string[]) { this.events = events; }
  sealGameplay(): void { this.isGameplaySealed = true; this.events.push('adapter-seal'); }
  prepareFrame(): void { this.events.push('prepare'); }
  dispose(): void { this.disposed++; this.events.push('adapter-dispose'); }
}

class FakePass implements EngineRenderPass {
  disposed = 0;
  constructor(private readonly name: string, private readonly events: string[]) {}
  async warm(): Promise<void> { this.events.push(`warm-${this.name}`); }
  render(): void { this.events.push(`render-${this.name}`); }
  dispose(): void { this.disposed++; this.events.push(`dispose-${this.name}`); }
}

function createRuntime(options: {
  workers?: number;
  passes?: number;
  events?: string[];
} = {}): { runtime: EngineRuntime; scheduler: FakeScheduler; adapter: FakeAdapter; events: string[] } {
  const events = options.events ?? [];
  const scheduler = new FakeScheduler();
  const adapter = new FakeAdapter(events);
  const runtime = new EngineRuntime({
    adapter,
    memory,
    diagnosticCapacity: 4,
    maxWorkerInputs: options.workers ?? 1,
    maxRenderPasses: options.passes ?? 1,
    scheduler,
  });
  return { runtime, scheduler, adapter, events };
}

describe('EngineRuntime', () => {
  test('enforces bootstrap, warmup, seal, run, stop, and shutdown transitions', async () => {
    const { runtime, scheduler, adapter } = createRuntime();
    expect(runtime.state).toBe(RuntimeState.Bootstrap);
    expect(() => runtime.start({ update() {} })).toThrow('gameplay seal');
    runtime.enterWarmup();
    expect(runtime.state).toBe(RuntimeState.Warmup);
    await runtime.warm();
    runtime.sealGameplay();
    expect(runtime.state).toBe(RuntimeState.GameplaySealed);
    expect(runtime.memory.phase).toBe(EnginePhase.GameplaySealed);
    expect(adapter.isGameplaySealed).toBe(true);
    runtime.start({ update() {} });
    expect(runtime.state).toBe(RuntimeState.Running);
    expect(scheduler.requests).toBe(1);
    runtime.stop();
    expect(runtime.state).toBe(RuntimeState.Stopped);
    expect(scheduler.cancels).toBe(1);
    runtime.dispose();
    expect(runtime.state).toBe(RuntimeState.Shutdown);
    expect(runtime.memory.phase).toBe(EnginePhase.Shutdown);
    expect(adapter.disposed).toBe(1);
    runtime.dispose();
    expect(adapter.disposed).toBe(1);
  });

  test('runs workers, prepare, client, and render passes in deterministic order', async () => {
    const { runtime, scheduler, events } = createRuntime({ workers: 1, passes: 2 });
    expect(runtime.registerWorker({
      poll() { events.push('poll'); },
      drainStructuralCommands() { events.push('structural'); },
      drainPoseBatches() { events.push('poses'); },
    })).toBe(RegistrationStatus.Registered);
    const first = new FakePass('first', events);
    const second = new FakePass('second', events);
    expect(runtime.registerRenderPass(first)).toBe(RegistrationStatus.Registered);
    expect(runtime.registerRenderPass(second)).toBe(RegistrationStatus.Registered);
    runtime.enterWarmup();
    await runtime.warm();
    runtime.sealGameplay();
    events.length = 0;
    let observed: Readonly<RenderFrame> | null = null;
    runtime.start({ update(frame) { observed = frame; events.push('update'); } });
    scheduler.fire(100);
    expect(events).toEqual([
      'poll', 'structural', 'poses', 'prepare', 'update', 'render-first', 'render-second',
    ]);
    expect(Object.is(observed, runtime.frame)).toBe(true);
    expect(runtime.frame.frameId).toBe(1);
    expect(runtime.frame.deltaSeconds).toBeCloseTo(1 / 60);
    const identity = runtime.frame;
    scheduler.fire(116);
    expect(runtime.frame).toBe(identity);
    expect(runtime.frame.frameId).toBe(2);
  });

  test('returns typed registration overflow and rejects registration after bootstrap', () => {
    const { runtime } = createRuntime({ workers: 0, passes: 1 });
    expect(runtime.registerWorker({ poll() {} })).toBe(RegistrationStatus.CapacityExceeded);
    expect(runtime.registerRenderPass(new FakePass('one', []))).toBe(RegistrationStatus.Registered);
    expect(runtime.registerRenderPass(new FakePass('two', []))).toBe(RegistrationStatus.CapacityExceeded);
    runtime.enterWarmup();
    expect(runtime.registerWorker({ poll() {} })).toBe(RegistrationStatus.RuntimeSealed);
    expect(runtime.registerRenderPass(new FakePass('late', []))).toBe(RegistrationStatus.RuntimeSealed);
  });

  test('stops and records a bounded diagnostic when frame code throws', async () => {
    const { runtime, scheduler } = createRuntime();
    runtime.enterWarmup();
    await runtime.warm();
    runtime.sealGameplay();
    runtime.start({ update() { throw new Error('frame failure'); } });
    scheduler.fire(1);
    expect(runtime.state).toBe(RuntimeState.Stopped);
    expect(runtime.diagnostics.count).toBe(1);
    expect(scheduler.requests).toBe(1);
  });

  test('stopping during update does not schedule another frame', async () => {
    const { runtime, scheduler } = createRuntime();
    runtime.enterWarmup();
    await runtime.warm();
    runtime.sealGameplay();
    runtime.start({ update() { runtime.stop(); } });
    scheduler.fire(1);
    expect(runtime.state).toBe(RuntimeState.Stopped);
    expect(scheduler.requests).toBe(1);
  });

  test('disposes render passes in reverse registration order', () => {
    const { runtime, events } = createRuntime({ passes: 2 });
    runtime.registerRenderPass(new FakePass('first', events));
    runtime.registerRenderPass(new FakePass('second', events));
    runtime.dispose();
    expect(events).toEqual(['dispose-second', 'dispose-first', 'adapter-dispose']);
  });
});
