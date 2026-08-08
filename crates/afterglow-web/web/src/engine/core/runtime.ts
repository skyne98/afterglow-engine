import { EngineMemory, EnginePhase, type EngineMemoryConfig } from './engine-memory.ts';
import { EngineDiagnostics, DiagnosticCode, DiagnosticSource } from './diagnostics.ts';
import { FrameBudget, type FrameBudgetConfig } from './frame-budget.ts';
import {
  prepareAfterglowFrame,
  type FrameRenderAdapter,
  type RenderWorkerInput,
  type VTInput,
} from './frame.ts';
import { RenderAdapter } from '../renderer/render-adapter.ts';
import {
  EngineMetric,
  EngineTelemetry,
  EngineTraceDescriptor,
  ENGINE_METRIC_DESCRIPTORS,
  ENGINE_TRACE_DESCRIPTORS,
  FRAME_BUDGET_TRACE_DESCRIPTORS,
  TelemetryRes,
} from '../telemetry/index.ts';
import { ResourceManifest, defineResource, type Resource } from './resource.ts';
import type { Camera, Scene } from 'three/webgpu';
import {
  RendererHost,
  type RendererHostOptions,
} from '../renderer/renderer-host.ts';
import {
  VirtualTextureFeedbackCoordinator,
  type VirtualTextureFeedbackOptions,
} from '../virtual-texturing/virtual-texture-feedback-coordinator.ts';
import type { VirtualTextureSystem } from '../virtual-texturing/virtual-texture-system.ts';
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
  PresentationAlreadyRegistered = 3,
}

export const enum RuntimeReadinessStage {
  Bootstrap = 0,
  Warmup = 1,
  GameplaySealed = 2,
  Starting = 3,
  GameReady = 4,
  Suspended = 5,
  Fatal = 6,
  Shutdown = 7,
}

export interface RuntimeReadinessSnapshot {
  stage: RuntimeReadinessStage;
  firstUpdateFrame: number;
  firstPresentationFrame: number;
  fatalDiagnostics: number;
}

interface NativeLifecycleGlobal {
  Deno?: { core?: { ops?: { op_afterglow_game_ready?(): void } } };
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
  /** Exactly one pass must identify the user-visible presentation boundary. */
  readonly presentation?: boolean;
  warm?(): Promise<void>;
  seal?(): void;
  render(frame: Readonly<RenderFrame>): void;
  dispose(): void;
}

export interface EngineFrameClient {
  /** @alloc-effect none */
  update(frame: Readonly<RenderFrame>): void;
}

export interface RuntimeDisposable { dispose(): void }
export interface RuntimeCloseable { close(): void | Promise<void> }

interface RuntimeShutdownTarget {
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
}

export interface EngineRuntimeOptions<TAdapter extends RuntimeRenderAdapter = RuntimeRenderAdapter> {
  adapter: TAdapter;
  memory: EngineMemoryConfig;
  frameBudget?: FrameBudgetConfig;
  diagnosticCapacity: number;
  maxWorkerInputs: number;
  maxRenderPasses: number;
  maxOwnedResources: number;
  scheduler?: AnimationScheduler;
  shutdownTarget?: RuntimeShutdownTarget | null;
  vt?: VTInput;
  resources?: readonly Resource<unknown>[];
}

export interface SceneEngineRuntimeOptions extends Omit<EngineRuntimeOptions<RenderAdapter>, 'adapter'> {
  scene: Scene;
  camera: Camera;
  entityCapacity: number;
  renderer?: Omit<RendererHostOptions, 'scene' | 'camera' | 'diagnostics' | 'onFatal'>;
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
  get hasCapacity(): boolean { return this.count < this.capacity; }

  add(input: RenderWorkerInput): RegistrationStatus {
    if (this.count === this.capacity) return RegistrationStatus.CapacityExceeded;
    this.inputs[this.count++] = input;
    return RegistrationStatus.Registered;
  }

  // @hot-no-alloc-begin FixedWorkerInputs.bootstrapReady
  bootstrapReady(): boolean {
    for (let index = 0; index < this.count; index++) {
      const input = this.inputs[index];
      if (input !== null && input !== undefined && input.isBootstrapReady?.() === false)
        return false;
    }
    return true;
  }
  // @hot-no-alloc-end FixedWorkerInputs.bootstrapReady

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

const enum OwnedResourceKind { Disposable = 0, Closeable = 1 }

class FixedOwnedResources {
  private readonly owners: Array<RuntimeDisposable | RuntimeCloseable | null>;
  private readonly kinds: Uint8Array;
  private count = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 0)
      throw new RangeError('owned resource capacity must be a non-negative integer');
    this.owners = new Array(capacity).fill(null);
    this.kinds = new Uint8Array(capacity);
  }

  add(owner: RuntimeDisposable | RuntimeCloseable, kind: OwnedResourceKind): RegistrationStatus {
    if (this.count === this.capacity) return RegistrationStatus.CapacityExceeded;
    this.owners[this.count] = owner;
    this.kinds[this.count] = kind;
    this.count++;
    return RegistrationStatus.Registered;
  }

  disposeSync(): void {
    for (let index = this.count - 1; index >= 0; index--) {
      const owner = this.owners[index];
      const kind = this.kinds[index] ?? OwnedResourceKind.Disposable;
      this.owners[index] = null;
      if (kind === OwnedResourceKind.Disposable) (owner as RuntimeDisposable | null)?.dispose();
      else void (owner as RuntimeCloseable | null)?.close();
    }
    this.count = 0;
  }

  async close(): Promise<void> {
    let firstError: unknown = null;
    for (let index = this.count - 1; index >= 0; index--) {
      const owner = this.owners[index];
      const kind = this.kinds[index] ?? OwnedResourceKind.Disposable;
      this.owners[index] = null;
      try {
        if (kind === OwnedResourceKind.Disposable) (owner as RuntimeDisposable | null)?.dispose();
        else await (owner as RuntimeCloseable | null)?.close();
      } catch (error) {
        if (firstError === null) firstError = error;
      }
    }
    this.count = 0;
    if (firstError !== null) throw firstError;
  }
}

/** Owns sealed frame order, fixed registrations, renderer, rAF, and reverse shutdown. */
export class EngineRuntime<TAdapter extends RuntimeRenderAdapter = RuntimeRenderAdapter> {
  readonly memory: EngineMemory;
  readonly budget: FrameBudget;
  readonly diagnostics: EngineDiagnostics;
  readonly telemetry: EngineTelemetry;
  private readonly mutableFrame = { frameId: 0, deltaSeconds: 0, elapsedSeconds: 0 };

  private readonly workers: FixedWorkerInputs;
  private readonly passes: Array<EngineRenderPass | null>;
  private readonly owners: FixedOwnedResources;
  private readonly scheduler: AnimationScheduler;
  private readonly manifest: ResourceManifest;
  private readonly shutdownTarget: RuntimeShutdownTarget | null;
  private readonly onBeforeUnload: EventListener;
  private readonly onGlobalError: EventListener;
  private readonly onUnhandledRejection: EventListener;
  readonly adapter: TAdapter;
  private readonly vt: VTInput | undefined;
  private readonly onAnimationFrame: (timestamp: number) => void;
  private passCount = 0;
  private client: EngineFrameClient | null = null;
  private animationHandle = 0;
  private previousTimestamp = -1;
  private elapsedSeconds = 0;
  private mutableState = RuntimeState.Bootstrap;
  private mutableReadinessStage = RuntimeReadinessStage.Bootstrap;
  private presentationPass = -1;
  private firstUpdateFrame = 0;
  private firstPresentationFrame = 0;
  private nativeReadySignaled = false;
  private ownedRendererHost: RendererHost | null = null;
  private pendingTextureSystem: VirtualTextureSystem | null = null;
  private pendingFeedbackCoordinator: VirtualTextureFeedbackCoordinator | null = null;
  private closing: Promise<void> | null = null;

  static async forScene(options: SceneEngineRuntimeOptions): Promise<EngineRuntime<RenderAdapter>> {
    const adapter = new RenderAdapter(options.scene, options.entityCapacity);
    const runtime = new EngineRuntime({ ...options, adapter });
    try {
      const host = await RendererHost.create({
        ...options.renderer,
        scene: options.scene,
        camera: options.camera,
        diagnostics: runtime.diagnostics,
        onFatal: error => runtime.fail(error, DiagnosticSource.Renderer),
      });
      const status = runtime.registerRenderPass(host);
      if (status !== RegistrationStatus.Registered)
        throw new Error(`runtime could not own its presentation pass: ${status}`);
      runtime.ownedRendererHost = host;
      return runtime;
    } catch (error) {
      runtime.dispose();
      throw error;
    }
  }

  constructor(options: EngineRuntimeOptions<TAdapter>) {
    if (!Number.isInteger(options.maxRenderPasses) || options.maxRenderPasses < 1)
      throw new RangeError('render pass capacity must be a positive integer');
    this.adapter = options.adapter;
    this.memory = new EngineMemory(options.memory);
    this.telemetry = new EngineTelemetry(
      ENGINE_TRACE_DESCRIPTORS,
      ENGINE_METRIC_DESCRIPTORS,
      this.memory.telemetryTrace,
      this.memory.telemetryMetrics,
    );
    this.budget = new FrameBudget(options.frameBudget, undefined, {
      recorder: this.telemetry.trace,
      stageDescriptors: FRAME_BUDGET_TRACE_DESCRIPTORS,
    });
    this.diagnostics = new EngineDiagnostics(options.diagnosticCapacity);
    this.workers = new FixedWorkerInputs(options.maxWorkerInputs);
    this.passes = new Array<EngineRenderPass | null>(options.maxRenderPasses).fill(null);
    this.owners = new FixedOwnedResources(options.maxOwnedResources);
    this.scheduler = options.scheduler ?? new BrowserAnimationScheduler();
    const defaultShutdownTarget = typeof window === 'object' ? window : null;
    this.shutdownTarget = options.shutdownTarget === undefined
      ? defaultShutdownTarget
      : options.shutdownTarget;
    this.onBeforeUnload = (): void => { void this.close(); };
    this.onGlobalError = (event): void => {
      const error = (event as ErrorEvent).error ?? event;
      this.fail(error, DiagnosticSource.Game);
    };
    this.onUnhandledRejection = (event): void => {
      this.fail((event as PromiseRejectionEvent).reason ?? event, DiagnosticSource.Game);
    };
    this.shutdownTarget?.addEventListener('beforeunload', this.onBeforeUnload);
    this.shutdownTarget?.addEventListener('error', this.onGlobalError);
    this.shutdownTarget?.addEventListener('unhandledrejection', this.onUnhandledRejection);
    this.vt = options.vt;
    this.onAnimationFrame = (timestamp: number): void => this.tick(timestamp);

    const memoryResource = defineResource('engineMemory', () => this.memory);
    const budgetResource = defineResource('frameBudget', () => this.budget);
    memoryResource.set(this.adapter.world, this.memory);
    budgetResource.set(this.adapter.world, this.budget);
    TelemetryRes.set(this.adapter.world, this.telemetry);
    this.manifest = new ResourceManifest(memoryResource, budgetResource, TelemetryRes, ...(options.resources ?? []));
    this.manifest.initialize(this.adapter.world);
  }

  get state(): RuntimeState { return this.mutableState; }
  get readinessStage(): RuntimeReadinessStage { return this.mutableReadinessStage; }
  get isGameReady(): boolean { return this.mutableReadinessStage === RuntimeReadinessStage.GameReady; }
  get frame(): Readonly<RenderFrame> { return this.mutableFrame; }
  readReadinessInto(out: RuntimeReadinessSnapshot): void {
    out.stage = this.mutableReadinessStage;
    out.firstUpdateFrame = this.firstUpdateFrame;
    out.firstPresentationFrame = this.firstPresentationFrame;
    out.fatalDiagnostics = this.diagnostics.count + this.diagnostics.dropped;
  }
  get registeredWorkers(): number { return this.workers.size; }
  get registeredRenderPasses(): number { return this.passCount; }
  get rendererHost(): RendererHost {
    if (!this.ownedRendererHost) throw new Error('runtime was not created with a renderer host');
    return this.ownedRendererHost;
  }

  ownDisposable(owner: RuntimeDisposable): RegistrationStatus {
    if (this.mutableState !== RuntimeState.Bootstrap) return RegistrationStatus.RuntimeSealed;
    return this.owners.add(owner, OwnedResourceKind.Disposable);
  }

  ownCloseable(owner: RuntimeCloseable): RegistrationStatus {
    if (this.mutableState !== RuntimeState.Bootstrap) return RegistrationStatus.RuntimeSealed;
    return this.owners.add(owner, OwnedResourceKind.Closeable);
  }

  registerWorker(input: RenderWorkerInput): RegistrationStatus {
    if (this.mutableState !== RuntimeState.Bootstrap) return RegistrationStatus.RuntimeSealed;
    return this.workers.add(input);
  }

  registerRenderPass(pass: EngineRenderPass): RegistrationStatus {
    if (this.mutableState !== RuntimeState.Bootstrap) return RegistrationStatus.RuntimeSealed;
    if (this.passCount === this.passes.length) return RegistrationStatus.CapacityExceeded;
    if (pass.presentation === true && this.presentationPass >= 0)
      return RegistrationStatus.PresentationAlreadyRegistered;
    const index = this.passCount++;
    this.passes[index] = pass;
    if (pass.presentation === true) this.presentationPass = index;
    return RegistrationStatus.Registered;
  }

  createVirtualTextureFeedback(
    textures: VirtualTextureSystem,
    options: Readonly<VirtualTextureFeedbackOptions>,
  ): VirtualTextureFeedbackCoordinator {
    if (this.mutableState !== RuntimeState.Bootstrap)
      throw new Error('virtual-texture presentation must be configured during bootstrap');
    if (!this.workers.hasCapacity || this.passCount === this.passes.length)
      throw new Error('runtime virtual-texture registration capacity exceeded');
    const host = this.rendererHost;
    const coordinator = new VirtualTextureFeedbackCoordinator(host.renderer, textures, options);
    try {
      if (this.pendingTextureSystem !== null)
        throw new Error('runtime already owns virtual-texture presentation');
      if (this.workers.add(coordinator) !== RegistrationStatus.Registered ||
          this.registerRenderPass(coordinator) !== RegistrationStatus.Registered)
        throw new Error('runtime virtual-texture registration failed after preflight');
      this.pendingTextureSystem = textures;
      this.pendingFeedbackCoordinator = coordinator;
      return coordinator;
    } catch (error) {
      coordinator.dispose();
      throw error;
    }
  }

  enterWarmup(): void {
    if (this.mutableState !== RuntimeState.Bootstrap)
      throw new Error('runtime can enter warmup only from bootstrap');
    this.memory.warmup();
    this.mutableState = RuntimeState.Warmup;
    this.mutableReadinessStage = RuntimeReadinessStage.Warmup;
  }

  async warm(): Promise<void> {
    if (this.mutableState !== RuntimeState.Warmup)
      throw new Error('runtime variants can be warmed only during warmup');
    for (let index = 0; index < this.passCount; index++) {
      const pass = this.passes[index];
      if (pass !== null && pass !== undefined) await pass.warm?.();
      if (index === this.presentationPass && this.pendingTextureSystem &&
          this.pendingFeedbackCoordinator) {
        this.rendererHost.initializeVirtualTextures(
          this.pendingTextureSystem, this.pendingFeedbackCoordinator,
        );
        this.pendingTextureSystem = null;
        this.pendingFeedbackCoordinator = null;
      }
    }
  }

  sealGameplay(): void {
    if (this.mutableState !== RuntimeState.Warmup)
      throw new Error('runtime can seal only after entering warmup');
    if (this.presentationPass < 0)
      throw new Error('runtime requires exactly one presentation render pass before seal');
    this.adapter.sealGameplay();
    this.memory.sealGameplay();
    this.manifest.seal(this.adapter.world);
    for (let index = 0; index < this.passCount; index++) {
      const pass = this.passes[index];
      if (pass !== null && pass !== undefined) pass.seal?.();
    }
    this.mutableState = RuntimeState.GameplaySealed;
    this.mutableReadinessStage = RuntimeReadinessStage.GameplaySealed;
  }

  start(client: EngineFrameClient): void {
    if (this.mutableState !== RuntimeState.GameplaySealed && this.mutableState !== RuntimeState.Stopped)
      throw new Error('runtime can start only after gameplay seal');
    if (this.diagnostics.count !== 0 || this.diagnostics.dropped !== 0) {
      this.mutableState = RuntimeState.Stopped;
      this.mutableReadinessStage = RuntimeReadinessStage.Fatal;
      throw new Error('runtime cannot start with fatal diagnostics');
    }
    this.client = client;
    this.previousTimestamp = -1;
    this.firstUpdateFrame = 0;
    this.firstPresentationFrame = 0;
    this.nativeReadySignaled = false;
    this.mutableState = RuntimeState.Running;
    this.mutableReadinessStage = RuntimeReadinessStage.Starting;
    this.animationHandle = this.scheduler.request(this.onAnimationFrame);
  }

  stop(): void {
    if (this.mutableState !== RuntimeState.Running) return;
    this.mutableState = RuntimeState.Stopped;
    if (this.mutableReadinessStage !== RuntimeReadinessStage.Fatal)
      this.mutableReadinessStage = RuntimeReadinessStage.Suspended;
    if (this.animationHandle !== 0) this.scheduler.cancel(this.animationHandle);
    this.animationHandle = 0;
  }

  dispose(): void {
    if (this.mutableState === RuntimeState.Shutdown) return;
    this.stop();
    this.owners.disposeSync();
    this.disposeCore();
  }

  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.stop();
    this.closing = this.closeOwnedAndDispose();
    return this.closing;
  }

  private async closeOwnedAndDispose(): Promise<void> {
    let failure: unknown = null;
    try { await this.owners.close(); }
    catch (error) { failure = error; }
    this.disposeCore();
    if (failure !== null) throw failure;
  }

  private disposeCore(): void {
    if (this.mutableState === RuntimeState.Shutdown) return;
    for (let index = this.passCount - 1; index >= 0; index--) {
      const pass = this.passes[index];
      if (pass !== null && pass !== undefined) pass.dispose();
      this.passes[index] = null;
    }
    this.passCount = 0;
    this.pendingTextureSystem = null;
    this.pendingFeedbackCoordinator = null;
    this.ownedRendererHost = null;
    this.adapter.dispose();
    this.client = null;
    this.shutdownTarget?.removeEventListener('beforeunload', this.onBeforeUnload);
    this.shutdownTarget?.removeEventListener('error', this.onGlobalError);
    this.shutdownTarget?.removeEventListener('unhandledrejection', this.onUnhandledRejection);
    this.memory.phase = EnginePhase.Shutdown;
    this.mutableState = RuntimeState.Shutdown;
    this.mutableReadinessStage = RuntimeReadinessStage.Shutdown;
  }

  fail(error: unknown, source = DiagnosticSource.Runtime): void {
    if (this.mutableState === RuntimeState.Shutdown ||
        this.mutableReadinessStage === RuntimeReadinessStage.Fatal) return;
    if (this.diagnostics.count === 0 && this.diagnostics.dropped === 0)
      this.diagnostics.tryRecord(DiagnosticCode.RuntimeState, source, error);
    this.stop();
    this.mutableReadinessStage = RuntimeReadinessStage.Fatal;
    this.showFatalPanel(error);
  }

  private showFatalPanel(error: unknown): void {
    if (typeof document !== 'object' || document.getElementById('afterglow-webgpu-failure') ||
        document.getElementById('afterglow-runtime-fatal')) return;
    const panel = document.createElement('pre');
    panel.id = 'afterglow-runtime-fatal';
    panel.textContent = `Afterglow stopped after a fatal engine error.\n\n${error instanceof Error ? error.message : String(error)}`;
    panel.style.cssText = 'position:fixed;inset:0;z-index:2147483647;box-sizing:border-box;margin:0;padding:24px;background:#11151c;color:#ff9a9a;font:16px/1.5 ui-monospace,monospace;white-space:pre-wrap';
    document.body.appendChild(panel);
  }

  // @hot-no-alloc-begin EngineRuntime.tick
  private tick(timestamp: number): void {
    if (this.mutableState !== RuntimeState.Running || this.client === null) return;
    this.animationHandle = 0;
    if (this.diagnostics.count !== 0 || this.diagnostics.dropped !== 0) {
      this.fail(new Error('runtime stopped after a fatal diagnostic'));
      return;
    }
    const deltaSeconds = this.previousTimestamp < 0
      ? 1 / 60
      : Math.max(0, (timestamp - this.previousTimestamp) / 1000);
    this.previousTimestamp = timestamp;
    this.elapsedSeconds += deltaSeconds;
    this.mutableFrame.frameId++;
    this.mutableFrame.deltaSeconds = deltaSeconds;
    this.mutableFrame.elapsedSeconds = this.elapsedSeconds;
    const frameNanoseconds = Math.floor(deltaSeconds * 1_000_000_000);
    this.telemetry.metrics.counterAdd(EngineMetric.Frames, 1);
    this.telemetry.metrics.histogramLog2(EngineMetric.FrameDeltaNs, frameNanoseconds);
    this.telemetry.metrics.maximum(EngineMetric.FrameMaxNs, frameNanoseconds);
    this.telemetry.trace.spanBegin(
      EngineTraceDescriptor.Frame, this.mutableFrame.frameId,
      this.mutableFrame.frameId, frameNanoseconds,
    );
    let gameSpanOpen = false;
    let renderSpanOpen = false;
    try {
      prepareAfterglowFrame(this.mutableFrame, this.workers, this.adapter, this.vt, this.memory, this.budget);
      this.telemetry.trace.spanBegin(
        EngineTraceDescriptor.GameUpdate, this.mutableFrame.frameId,
        this.mutableFrame.frameId, 0,
      );
      gameSpanOpen = true;
      this.client.update(this.mutableFrame);
      if (this.firstUpdateFrame === 0) this.firstUpdateFrame = this.mutableFrame.frameId;
      this.telemetry.trace.spanEnd(
        EngineTraceDescriptor.GameUpdate, this.mutableFrame.frameId,
        this.mutableFrame.frameId, 0,
      );
      gameSpanOpen = false;
      this.telemetry.trace.spanBegin(
        EngineTraceDescriptor.RenderPasses, this.mutableFrame.frameId,
        this.mutableFrame.frameId, 0,
      );
      renderSpanOpen = true;
      for (let index = 0; index < this.passCount; index++) {
        const pass = this.passes[index];
        if (pass !== null && pass !== undefined) {
          pass.render(this.mutableFrame);
          if (index === this.presentationPass && this.firstPresentationFrame === 0)
            this.firstPresentationFrame = this.mutableFrame.frameId;
        }
      }
      this.telemetry.trace.spanEnd(
        EngineTraceDescriptor.RenderPasses, this.mutableFrame.frameId,
        this.mutableFrame.frameId, 0,
      );
      renderSpanOpen = false;
      this.telemetry.trace.spanEnd(
        EngineTraceDescriptor.Frame, this.mutableFrame.frameId,
        this.mutableFrame.frameId, 0,
      );
      if (this.mutableState === RuntimeState.Running &&
          this.mutableReadinessStage === RuntimeReadinessStage.Starting &&
          this.firstUpdateFrame !== 0 && this.firstPresentationFrame !== 0 &&
          this.workers.bootstrapReady() &&
          this.diagnostics.count === 0 && this.diagnostics.dropped === 0) {
        this.mutableReadinessStage = RuntimeReadinessStage.GameReady;
        this.publishNativeGameReady();
      }
    } catch (error) {
      this.budget.abortOpenStages();
      if (gameSpanOpen)
        this.telemetry.trace.spanEnd(EngineTraceDescriptor.GameUpdate, this.mutableFrame.frameId, this.mutableFrame.frameId, 1);
      if (renderSpanOpen)
        this.telemetry.trace.spanEnd(EngineTraceDescriptor.RenderPasses, this.mutableFrame.frameId, this.mutableFrame.frameId, 1);
      this.telemetry.trace.spanEnd(EngineTraceDescriptor.Frame, this.mutableFrame.frameId, this.mutableFrame.frameId, 1);
      console.error('[afterglow] runtime frame failed:', error instanceof Error ? error.stack : String(error)); // @alloc-allowed reason=FatalDiagnostic issue=DME-044 expires=2026-10-01
      this.fail(error);
      return;
    }
    if (this.mutableState === RuntimeState.Running)
      this.animationHandle = this.scheduler.request(this.onAnimationFrame);
  }
  // @hot-no-alloc-end EngineRuntime.tick

  /** @alloc-effect none */
  private publishNativeGameReady(): void {
    if (this.nativeReadySignaled) return;
    const native = globalThis as typeof globalThis & NativeLifecycleGlobal;
    const signal = native.Deno?.core?.ops?.op_afterglow_game_ready;
    if (typeof signal !== 'function') return;
    signal();
    this.nativeReadySignaled = true;
  }
}
