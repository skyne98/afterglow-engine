# Frame Budgets

`FrameBudget` gives each engine stage a cumulative deadline and operation limit.
It uses the current frame interval, so deadlines scale at 60 Hz, 120 Hz, or
144 Hz rather than assuming a fixed 16.67 ms frame.

Pass it to the deterministic frame orchestrator:

```ts
prepareAfterglowFrame(frame, workers, renderer, virtualTextures, memory, budget);
```

Required stages—worker polling, virtual-texture completion, and renderer
preparation—always run but record misses and overruns. Deferrable structural and
pose drains return typed deferred statuses and remain queued for a later frame.
All counters and limits use storage allocated when `FrameBudget` is constructed;
checking a stage does not allocate.

Telemetry is available through fixed typed arrays:

- `stageOperations`
- `stageExhaustions`
- `stageOverruns`
- `stageDeferred`
- `stageElapsedUs`
- `stageTotalElapsedUs`
- `stageMaxElapsedUs`

Use `FrameBudgetRes` for the default 15%/35%/45%/55%/95% cumulative deadlines,
or construct a budget with project-specific fractions and operation limits.
