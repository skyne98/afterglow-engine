import { describe, expect, test } from 'bun:test';
import { BudgetDecision, FrameBudget, FrameStage } from './frame-budget.ts';
import { prepareAfterglowFrame } from './frame.ts';

const config = {
  deadlineFractions: [0.2, 0.4, 0.5, 0.6, 0.9],
  operationLimits: [1, 1, 1, 1, 1],
};

describe('FrameBudget', () => {
  test('returns typed deadline/operation decisions without resetting telemetry', () => {
    let now = 100;
    const budget = new FrameBudget(config, () => now);
    budget.beginFrame(7, 10);
    now = 101;
    expect(budget.beginStage(FrameStage.WorkerPoll)).toBe(BudgetDecision.Run);
    expect(budget.beginStage(FrameStage.WorkerPoll)).toBe(BudgetDecision.DeferredOperationLimit);
    now = 106;
    expect(budget.beginStage(FrameStage.StructuralCommands)).toBe(BudgetDecision.DeferredDeadline);
    expect(budget.stageDeferred[FrameStage.WorkerPoll]).toBe(1);
    expect(budget.stageDeferred[FrameStage.StructuralCommands]).toBe(1);
  });

  test('records reusable per-frame, cumulative, and maximum stage timings', () => {
    let now = 10;
    const budget = new FrameBudget(config, () => now);
    budget.beginFrame(1, 10);
    now = 11;
    budget.beginStage(FrameStage.VirtualTexture);
    now = 12.5;
    budget.endStage(FrameStage.VirtualTexture);
    expect(budget.stageElapsedUs[FrameStage.VirtualTexture]).toBe(1500);
    expect(budget.stageTotalElapsedUs[FrameStage.VirtualTexture]).toBe(1500);
    expect(budget.stageMaxElapsedUs[FrameStage.VirtualTexture]).toBe(1500);

    now = 20;
    budget.beginFrame(2, 10);
    expect(budget.stageElapsedUs[FrameStage.VirtualTexture]).toBe(0);
    budget.beginStage(FrameStage.VirtualTexture);
    now = 20.5;
    budget.endStage(FrameStage.VirtualTexture);
    expect(budget.stageElapsedUs[FrameStage.VirtualTexture]).toBe(500);
    expect(budget.stageTotalElapsedUs[FrameStage.VirtualTexture]).toBe(2000);
    expect(budget.stageMaxElapsedUs[FrameStage.VirtualTexture]).toBe(1500);
  });

  test('required stages run while recording deadline misses and overruns', () => {
    let now = 0;
    const budget = new FrameBudget(config, () => now);
    budget.beginFrame(1, 10);
    now = 20;
    expect(budget.beginStage(FrameStage.RenderPrepare, true)).toBe(BudgetDecision.Run);
    budget.endStage(FrameStage.RenderPrepare);
    expect(budget.stageExhaustions[FrameStage.RenderPrepare]).toBe(1);
    expect(budget.stageOverruns[FrameStage.RenderPrepare]).toBe(1);
    expect(budget.stageDeferred[FrameStage.RenderPrepare]).toBe(0);
  });

  test('feeds VT presentation timing into the central tuner before polling', () => {
    let recorded = -1;
    let processed = 0;
    let polled = 0;
    const adapter = { prepareFrame() {} };
    prepareAfterglowFrame(
      { frameId: 1, deltaSeconds: 0.016, elapsedSeconds: 0.016 },
      null,
      adapter as never,
      {
        frameTimeMs: 16.7,
        feedback: new Map(),
        store: {
          recordFrameTime(value: number) { recorded = value; },
          processFeedback() { processed++; },
          poll() { polled++; },
        } as never,
      },
    );
    expect(recorded).toBe(16.7);
    expect(processed).toBe(1);
    expect(polled).toBe(1);
  });

  test('frame orchestration defers optional work but always prepares rendering', () => {
    let now = 0;
    const budget = new FrameBudget(config, () => now);
    let structural = 0;
    let poses = 0;
    let prepared = 0;
    const worker = {
      poll() { now = 3; },
      drainStructuralCommands() { structural++; now = 7; },
      drainPoseBatches() { poses++; },
    };
    const adapter = { prepareFrame() { prepared++; } };
    prepareAfterglowFrame(
      { frameId: 1, deltaSeconds: 0.01, elapsedSeconds: 0.01 },
      worker as never,
      adapter as never,
      undefined,
      undefined,
      budget,
    );
    expect(structural).toBe(1);
    expect(poses).toBe(0);
    expect(prepared).toBe(1);
    expect(budget.stageDeferred[FrameStage.PoseBatches]).toBe(1);
  });
});
