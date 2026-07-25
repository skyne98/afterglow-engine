import { describe, expect, test } from 'bun:test';
import {
  EngineMemory, EnginePhase, FixedIndexPool, FixedStructuralCommandRing,
  INVALID_MEMORY_OFFSET, INVALID_POOL_INDEX, LinearArena, RingPushStatus,
  StructuralCommandKind,
} from './engine-memory.ts';
import { prepareAfterglowFrame } from './frame.ts';

describe('EngineMemory fixed storage', () => {
  test('linear arena aligns, resets, and never grows', () => {
    const arena = new LinearArena(32);
    expect(arena.allocate(3)).toBe(0);
    expect(arena.allocate(4, 8)).toBe(8);
    expect(arena.allocate(21)).toBe(INVALID_MEMORY_OFFSET);
    expect(arena.buffer.byteLength).toBe(32);
    expect(arena.overflows).toBe(1);
    arena.reset();
    expect(arena.allocate(32)).toBe(0);
  });

  test('fixed index pool has deterministic overflow and O(1) reuse', () => {
    const pool = new FixedIndexPool(2);
    expect(pool.acquire()).toBe(0);
    expect(pool.acquire()).toBe(1);
    expect(pool.acquire()).toBe(INVALID_POOL_INDEX);
    expect(pool.release(0)).toBe(true);
    expect(pool.acquire()).toBe(0);
    expect(pool.highWater).toBe(2);
  });

  test('structural command ring reports overflow and drains a bounded prefix', () => {
    const ring = new FixedStructuralCommandRing(2);
    expect(ring.tryPush(StructuralCommandKind.Spawn, 7, 10, 11)).toBe(RingPushStatus.Accepted);
    expect(ring.tryPush(StructuralCommandKind.Reparent, 8, 2)).toBe(RingPushStatus.Accepted);
    expect(ring.tryPush(StructuralCommandKind.Despawn, 9)).toBe(RingPushStatus.CapacityExceeded);
    const seen: number[] = [];
    const sink = { applyStructuralCommand(kind: number, entity: number, a: number, b: number) {
      seen.push(kind, entity, a, b);
    } };
    expect(ring.drain(1, sink)).toBe(1);
    expect(ring.count).toBe(1);
    expect(seen).toEqual([StructuralCommandKind.Spawn, 7, 10, 11]);
    expect(ring.drain(1, sink)).toBe(1);
    expect(ring.highWater).toBe(2);
    expect(ring.overflows).toBe(1);
  });

  test('seals only after warmup and rewinds frame scratch', () => {
    const memory = new EngineMemory({
      frameScratchBytes: 64, renderScratchBytes: 64,
      structuralCommands: 8, workerCompletions: 8, assetRequests: 4, vtRequests: 16,
      telemetryRecords: 4, telemetryMetricCells: 4,
    });
    memory.warmup();
    memory.sealGameplay();
    expect(memory.phase).toBe(EnginePhase.GameplaySealed);
    expect(memory.telemetryTrace.byteLength).toBe(4 * 40);
    expect(memory.telemetryMetrics.length).toBe(4);
    expect(() => memory.sealGameplay()).toThrow('only after warmup');
    memory.frame.allocate(16);
    memory.beginFrame();
    expect(memory.frame.used).toBe(0);
  });

  test('frame orchestration rejects unsealed engine memory', () => {
    const memory = new EngineMemory({
      frameScratchBytes: 8, renderScratchBytes: 8,
      structuralCommands: 1, workerCompletions: 1, assetRequests: 1, vtRequests: 1,
      telemetryRecords: 1, telemetryMetricCells: 1,
    });
    expect(() => prepareAfterglowFrame(
      { frameId: 1, deltaSeconds: 1 / 60, elapsedSeconds: 0 }, null,
      { prepareFrame() {} } as never, undefined, memory,
    )).toThrow('must be sealed');
  });

  test('frame orchestration rewinds fixed scratch before worker stages', () => {
    const memory = new EngineMemory({
      frameScratchBytes: 64, renderScratchBytes: 64,
      structuralCommands: 8, workerCompletions: 8, assetRequests: 4, vtRequests: 16,
      telemetryRecords: 4, telemetryMetricCells: 4,
    });
    memory.warmup();
    memory.sealGameplay();
    memory.frame.allocate(32);
    let observedUsed = -1;
    const adapter = { prepareFrame() { observedUsed = memory.frame.used; } };
    prepareAfterglowFrame(
      { frameId: 1, deltaSeconds: 1 / 60, elapsedSeconds: 1 },
      null,
      adapter as never,
      undefined,
      memory,
    );
    expect(observedUsed).toBe(0);
  });
});
