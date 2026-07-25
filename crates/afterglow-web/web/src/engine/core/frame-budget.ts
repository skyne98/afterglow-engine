import { Resource, defineResource } from './resource.ts';
import type { TelemetryRecorder } from '../telemetry/telemetry.ts';

/** Fixed stages in deterministic frame order. */
export enum FrameStage {
  WorkerPoll = 0,
  VirtualTexture = 1,
  StructuralCommands = 2,
  PoseBatches = 3,
  RenderPrepare = 4,
  Count = 5,
}

export enum BudgetDecision {
  Run = 0,
  DeferredOperationLimit = 1,
  DeferredDeadline = 2,
}

export interface FrameBudgetConfig {
  /** Cumulative deadline within the frame, as a 0..1 fraction. */
  deadlineFractions: readonly number[];
  /** Maximum top-level invocations for each stage per frame. */
  operationLimits: readonly number[];
}

export const DEFAULT_FRAME_BUDGET: FrameBudgetConfig = {
  deadlineFractions: [0.15, 0.35, 0.45, 0.55, 0.95],
  operationLimits: [1, 1, 1, 1, 1],
};

export interface FrameBudgetTelemetry {
  readonly recorder: TelemetryRecorder;
  readonly stageDescriptors: readonly number[];
}

/**
 * Allocation-free frame-stage admission and telemetry after construction.
 *
 * Required stages still run after a miss, but record it. Deferrable stages
 * return a typed decision and remain queued in their owning system.
 */
export class FrameBudget {
  private readonly deadlineFractions = new Float64Array(FrameStage.Count);
  private readonly operationLimits = new Uint32Array(FrameStage.Count);
  private readonly deadlines = new Float64Array(FrameStage.Count);
  private readonly operations = new Uint32Array(FrameStage.Count);
  private readonly exhaustions = new Uint32Array(FrameStage.Count);
  private readonly overruns = new Uint32Array(FrameStage.Count);
  private readonly deferred = new Uint32Array(FrameStage.Count);
  private readonly stageStarts = new Float64Array(FrameStage.Count);
  private readonly stageActive = new Uint8Array(FrameStage.Count);
  private readonly elapsedUs = new Float64Array(FrameStage.Count);
  private readonly totalElapsedUs = new Float64Array(FrameStage.Count);
  private readonly maxElapsedUs = new Float64Array(FrameStage.Count);
  private frameId = 0;
  private frameStart = 0;
  private frameDurationMs = 0;

  constructor(
    config: FrameBudgetConfig = DEFAULT_FRAME_BUDGET,
    private readonly clock: () => number = () => performance.now(),
    private readonly telemetry?: FrameBudgetTelemetry,
  ) {
    if (config.deadlineFractions.length !== FrameStage.Count ||
        config.operationLimits.length !== FrameStage.Count)
      throw new RangeError(`FrameBudget requires ${FrameStage.Count} stage entries`);
    let previous = 0;
    for (let stage = 0; stage < FrameStage.Count; stage++) {
      const fraction = config.deadlineFractions[stage];
      const operations = config.operationLimits[stage];
      if (!Number.isFinite(fraction) || fraction < previous || fraction > 1)
        throw new RangeError('frame deadline fractions must be monotonic values in 0..1');
      if (!Number.isInteger(operations) || operations < 1)
        throw new RangeError('frame operation limits must be positive integers');
      this.deadlineFractions[stage] = fraction;
      this.operationLimits[stage] = operations;
      previous = fraction;
    }
  }

  // @hot-no-alloc-begin FrameBudget.beginFrame
  beginFrame(frameId: number, frameDurationMs: number): void {
    this.frameId = frameId;
    this.frameStart = this.clock();
    this.frameDurationMs = Math.max(0.1, frameDurationMs);
    for (let stage = 0; stage < FrameStage.Count; stage++) {
      this.operations[stage] = 0;
      this.elapsedUs[stage] = 0;
      this.stageStarts[stage] = 0;
      this.stageActive[stage] = 0;
      this.deadlines[stage] = this.frameStart + this.frameDurationMs * this.deadlineFractions[stage];
    }
  }
  // @hot-no-alloc-end FrameBudget.beginFrame

  // @hot-no-alloc-begin FrameBudget.beginStage
  beginStage(stage: FrameStage, required = false): BudgetDecision {
    const operationExceeded = this.operations[stage] >= this.operationLimits[stage];
    const deadlineExceeded = this.clock() > this.deadlines[stage];
    if (operationExceeded || deadlineExceeded) {
      this.exhaustions[stage]++;
      if (!required) {
        this.deferred[stage]++;
        return operationExceeded
          ? BudgetDecision.DeferredOperationLimit
          : BudgetDecision.DeferredDeadline;
      }
    }
    this.operations[stage]++;
    this.stageStarts[stage] = this.clock();
    this.stageActive[stage] = 1;
    const descriptor = this.telemetry?.stageDescriptors[stage];
    if (descriptor !== undefined)
      this.telemetry?.recorder.spanBegin(descriptor, this.frameId, stage, 0);
    return BudgetDecision.Run;
  }
  // @hot-no-alloc-end FrameBudget.beginStage

  // @hot-no-alloc-begin FrameBudget.endStage
  endStage(stage: FrameStage): void {
    const now = this.clock();
    const elapsedUs = Math.max(0, (now - this.stageStarts[stage]) * 1000);
    this.elapsedUs[stage] += elapsedUs;
    this.totalElapsedUs[stage] += elapsedUs;
    if (this.elapsedUs[stage] > this.maxElapsedUs[stage])
      this.maxElapsedUs[stage] = this.elapsedUs[stage];
    if (now > this.deadlines[stage]) this.overruns[stage]++;
    this.stageActive[stage] = 0;
    const descriptor = this.telemetry?.stageDescriptors[stage];
    if (descriptor !== undefined)
      this.telemetry?.recorder.spanEnd(descriptor, this.frameId, stage, elapsedUs);
  }
  // @hot-no-alloc-end FrameBudget.endStage

  // @hot-no-alloc-begin FrameBudget.abortOpenStages
  abortOpenStages(): void {
    for (let stage = 0; stage < FrameStage.Count; stage++)
      if (this.stageActive[stage] !== 0) this.endStage(stage);
  }
  // @hot-no-alloc-end FrameBudget.abortOpenStages

  get currentFrameId(): number { return this.frameId; }
  get currentFrameDurationMs(): number { return this.frameDurationMs; }
  get stageOperations(): Uint32Array { return this.operations; }
  get stageExhaustions(): Uint32Array { return this.exhaustions; }
  get stageOverruns(): Uint32Array { return this.overruns; }
  get stageDeferred(): Uint32Array { return this.deferred; }
  /** Microseconds consumed by each stage in the current frame. */
  get stageElapsedUs(): Float64Array { return this.elapsedUs; }
  /** Cumulative stage microseconds since construction. */
  get stageTotalElapsedUs(): Float64Array { return this.totalElapsedUs; }
  /** Largest complete-frame stage duration observed since construction. */
  get stageMaxElapsedUs(): Float64Array { return this.maxElapsedUs; }
}

export const FrameBudgetRes: Resource<FrameBudget> = defineResource(
  'frameBudget',
  () => new FrameBudget(),
);
