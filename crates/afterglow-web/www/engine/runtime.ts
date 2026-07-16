import { EngineMemory, EnginePhase, type EngineMemoryConfig } from './engine-memory.ts';
import { EngineDiagnostics, DiagnosticCode, DiagnosticSource } from './diagnostics.ts';
import { FrameBudget, type FrameBudgetConfig } from './frame-budget.ts';
import {
  prepareAfterglowFrame,
  type FrameRenderAdapter,
  type RenderWorkerInput,
  type VTInput,
} from './frame.ts';
import { ResourceManifest, defineResource, type Resource } from './resource.ts';
import type { RenderFrame } from './types.ts';

export const enum RuntimeState {
  Bootstrap = 0,
  Warmup = 1,
  GameplaySealed = 2,
  Running = 3,
  Stopped = 4,
  Shutdown = 5,
}

export const enum RegistrationStatus {
  Registered = 0,
  CapacityExceeded = 1,
  RuntimeSealed = 2,
}

export interface AnimationScheduler {
  request(callback: (timestamp: number) => void): number;
  cancel(handle: number): void;
}

export interface RuntimeRenderAdapter extends FrameRenderAdapter {
  readonly world: object;
  readonly isGameplaySealed: boolean;
  sealGameplay(): void;
  dispose(): void;
}

export interface EngineRenderPass {
  warm?(): Promise<void>;
  seal?(): void;
  render(frame: Readonly<RenderFrame>): void;
  dispose(): void;
}

export interface EngineFrameClient {
  /** @alloc-effect none */
  update(frame: Readonly<RenderFrame>): void;
}

export interface EngineRuntimeOptions {
  adapter: RuntimeRenderAdapter;
  memory: EngineMemoryConfig;
  frameBudget?: FrameBudgetConfig;
  diagnosticCapacity: number;
  maxWorkerInputs: number;
  maxRenderPasses: number;
  scheduler?: AnimationScheduler;
  vt?: VTInput;
  resources?: readonly Resource<unknown>[];
}

class BrowserAnimationScheduler implements AnimationScheduler {
  request(callback: (timestamp: number) => void): number {
    return requestAnimationFrame(callback);
  }
  cancel(handle: number): void {
    cancelAnimationFrame(handle);
  }
}

class FixedWorkerInputs implements RenderWorkerInput {
  private readonly inputs: Array<RenderWorkerInput | null>;
  private count = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 0)
      throw new RangeError('worker input capacity must be a non-negative integer');
    this.inputs = new Array<RenderWorkerInput | null>(capacity).fill(null);
  }

  get size(): number { return this.count; }

  add(input: RenderWorkerInput): RegistrationStatus {
    if (this.count === this.capacity) return RegistrationStatus.CapacityExceeded;
    this.inputs[this.count++] = input;
    return RegistrationStatus.Registered;
  }

  // @hot-no-alloc-begin FixedWorkerInputs.poll
  poll(): void {
    for (let index = 0; index < this.count; index++) {
      const input = this.inputs[index];
      if (input !== null && input !== undefined) input.poll();
    }
  }
  // @hot-no-alloc-end FixedWorkerInputs.poll

  // @hot-no-alloc-begin FixedWorkerInputs.drainStructuralCommands
  drainStructuralCommands(adapter: FrameRenderAdapter): void {
    for (let index = 0; index < this.count; index++) {
      const input = this.inputs[index];
      if (input !== null && input !== undefined) input.drainStructuralCommands?.(adapter);
    }
  }
  // @hot-no-alloc-end FixedWorkerInputs.drainStructuralCommands

  // @hot-no-alloc-begin FixedWorkerInputs.drainPoseBatches
  drainPoseBatches(adapter: FrameRenderAdapter): void {
    for (let index = 0; index < this.count; index++) {
      const input = this.inputs[index];
      if (input !== null && input !== undefined) input.drainPoseBatches?.(adapter);
    }
  }
  // @hot-no-alloc-end FixedWorkerInputs.drainPoseBatches
}

/** Owns sealed frame order, fixed registrations, rAF, and runtime disposal. */
export class EngineRuntime {
  readonly memory: EngineMemory;
  readonly budget: FrameBudget;
  readonly diagnostics: EngineDiagnostics;
  private readonly mutableFrame = { frameId: 0, deltaSeconds: 0, elapsedSeconds: 0 };

  private readonly workers: FixedWorkerInputs;
  private readonly passes: Array<EngineRenderPass | null>;
  private readonly scheduler: AnimationScheduler;
  private readonly manifest: ResourceManifest;
  private readonly adapter: RuntimeRenderAdapter;
  private readonly vt: VTInput | undefined;
  private readonly onAnimationFrame: (timestamp: number) => void;
  private passCount = 0;
  private client: EngineFrameClient | null = null;
  private animationHandle = 0;
  private previousTimestamp = -1;
  private elapsedSeconds = 0;
  private mutableState = RuntimeState.Bootstrap;

  constructor(options: EngineRuntimeOptions) {
    if (!Number.isInteger(options.maxRenderPasses) || options.maxRenderPasses < 1)
      throw new RangeError('render pass capacity must be a positive integer');
    this.adapter = options.adapter;
    this.memory = new EngineMemory(options.memory);
    this.budget = new FrameBudget(options.frameBudget);
    this.diagnostics = new EngineDiagnostics(options.diagnosticCapacity);
    this.workers = new FixedWorkerInputs(options.maxWorkerInputs);
    this.passes = new Array<EngineRenderPass | null>(options.maxRenderPasses).fill(null);
    this.scheduler = options.scheduler ?? new BrowserAnimationScheduler();
    this.vt = options.vt;
    this.onAnimationFrame = (timestamp: number): void => this.tick(timestamp);

    const memoryResource = defineResource('engineMemory', () => this.memory);
    const budgetResource = defineResource('frameBudget', () => this.budget);
    memoryResource.set(this.adapter.world, this.memory);
    budgetResource.set(this.adapter.world, this.budget);
    this.manifest = new ResourceManifest(memoryResource, budgetResource, ...(options.resources ?? []));
    this.manifest.initialize(this.adapter.world);
  }

  get state(): RuntimeState { return this.mutableState; }
  get frame(): Readonly<RenderFrame> { return this.mutableFrame; }
  get registeredWorkers(): number { return this.workers.size; }
  get registeredRenderPasses(): number { return this.passCount; }

  registerWorker(input: RenderWorkerInput): RegistrationStatus {
    if (this.mutableState !== RuntimeState.Bootstrap) return RegistrationStatus.RuntimeSealed;
    return this.workers.add(input);
  }

  registerRenderPass(pass: EngineRenderPass): RegistrationStatus {
    if (this.mutableState !== RuntimeState.Bootstrap) return RegistrationStatus.RuntimeSealed;
    if (this.passCount === this.passes.length) return RegistrationStatus.CapacityExceeded;
    this.passes[this.passCount++] = pass;
    return RegistrationStatus.Registered;
  }

  enterWarmup(): void {
    if (this.mutableState !== RuntimeState.Bootstrap)
      throw new Error('runtime can enter warmup only from bootstrap');
    this.memory.warmup();
    this.mutableState = RuntimeState.Warmup;
  }

  async warm(): Promise<void> {
    if (this.mutableState !== RuntimeState.Warmup)
      throw new Error('runtime variants can be warmed only during warmup');
    for (let index = 0; index < this.passCount; index++) {
      const pass = this.passes[index];
      if (pass !== null && pass !== undefined) await pass.warm?.();
    }
  }

  sealGameplay(): void {
    if (this.mutableState !== RuntimeState.Warmup)
      throw new Error('runtime can seal only after entering warmup');
    this.adapter.sealGameplay();
    this.memory.sealGameplay();
    this.manifest.seal(this.adapter.world);
    for (let index = 0; index < this.passCount; index++) {
      const pass = this.passes[index];
      if (pass !== null && pass !== undefined) pass.seal?.();
    }
    this.mutableState = RuntimeState.GameplaySealed;
  }

  start(client: EngineFrameClient): void {
    if (this.mutableState !== RuntimeState.GameplaySealed && this.mutableState !== RuntimeState.Stopped)
      throw new Error('runtime can start only after gameplay seal');
    this.client = client;
    this.previousTimestamp = -1;
    this.mutableState = RuntimeState.Running;
    this.animationHandle = this.scheduler.request(this.onAnimationFrame);
  }

  stop(): void {
    if (this.mutableState !== RuntimeState.Running) return;
    this.mutableState = RuntimeState.Stopped;
    if (this.animationHandle !== 0) this.scheduler.cancel(this.animationHandle);
    this.animationHandle = 0;
  }

  dispose(): void {
    if (this.mutableState === RuntimeState.Shutdown) return;
    this.stop();
    for (let index = this.passCount - 1; index >= 0; index--) {
      const pass = this.passes[index];
      if (pass !== null && pass !== undefined) pass.dispose();
      this.passes[index] = null;
    }
    this.passCount = 0;
    this.adapter.dispose();
    this.client = null;
    this.memory.phase = EnginePhase.Shutdown;
    this.mutableState = RuntimeState.Shutdown;
  }

  // @hot-no-alloc-begin EngineRuntime.tick
  private tick(timestamp: number): void {
    if (this.mutableState !== RuntimeState.Running || this.client === null) return;
    this.animationHandle = 0;
    const deltaSeconds = this.previousTimestamp < 0
      ? 1 / 60
      : Math.max(0, (timestamp - this.previousTimestamp) / 1000);
    this.previousTimestamp = timestamp;
    this.elapsedSeconds += deltaSeconds;
    this.mutableFrame.frameId++;
    this.mutableFrame.deltaSeconds = deltaSeconds;
    this.mutableFrame.elapsedSeconds = this.elapsedSeconds;
    try {
      prepareAfterglowFrame(this.mutableFrame, this.workers, this.adapter, this.vt, this.memory, this.budget);
      this.client.update(this.mutableFrame);
      for (let index = 0; index < this.passCount; index++) {
        const pass = this.passes[index];
        if (pass !== null && pass !== undefined) pass.render(this.mutableFrame);
      }
    } catch (error) {
      this.diagnostics.tryRecord(DiagnosticCode.RuntimeState, DiagnosticSource.Runtime, error);
      this.mutableState = RuntimeState.Stopped;
      return;
    }
    if (this.mutableState === RuntimeState.Running)
      this.animationHandle = this.scheduler.request(this.onAnimationFrame);
  }
  // @hot-no-alloc-end EngineRuntime.tick
}
