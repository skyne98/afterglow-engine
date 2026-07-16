# Frame budgets

`engine/frame-budget.ts` provides deterministic, allocation-free stage admission
for the engine frame orchestrator.

## API

```ts
enum FrameStage {
  WorkerPoll,
  VirtualTexture,
  StructuralCommands,
  PoseBatches,
  RenderPrepare,
}

enum BudgetDecision {
  Run,
  DeferredOperationLimit,
  DeferredDeadline,
}

class FrameBudget {
  beginFrame(frameId: number, frameDurationMs: number): void;
  beginStage(stage: FrameStage, required?: boolean): BudgetDecision;
  endStage(stage: FrameStage): void;

  readonly stageOperations: Uint32Array;
  readonly stageExhaustions: Uint32Array;
  readonly stageOverruns: Uint32Array;
  readonly stageDeferred: Uint32Array;
  readonly stageElapsedUs: Float64Array;
  readonly stageTotalElapsedUs: Float64Array;
  readonly stageMaxElapsedUs: Float64Array;
}
```

Deadlines are cumulative fractions of the current frame duration, so the same
configuration scales between 60 Hz and 144 Hz. Operation limits, deadlines,
counters, and telemetry use fixed typed arrays allocated at construction.
Current-frame, cumulative, and maximum stage CPU durations are retained in
microsecond arrays.

`prepareAfterglowFrame(..., memory, budget)` starts the budget before any stage.
Worker polling, VT completion, and render preparation are required: they still
run after a miss and record the exhaustion/overrun. Structural and pose drains
are deferrable and remain queued in their owning systems when admission returns
a typed deferred status. Rendering is never left half-committed.

`FrameBudgetRes` exposes the default resource. The default cumulative deadline
fractions are 15%, 35%, 45%, 55%, and 95% of the measured frame interval, with
one top-level invocation per stage.

## Diagnostic frame capture

`engine/bench.ts` provides `FrameBench`. Construction reserves two fixed
`Float64Array` buffers at the declared capacity. `start(sampleCount)` returns a
typed invalid-count status instead of growing. `tick(timestamp)` only records a
numeric interval and marks results pending when full. Sorting, percentile
calculation, callbacks, and formatting happen only when diagnostic code calls
`finish()` outside the frame hot path. One caller-visible result object is
reused across runs.
